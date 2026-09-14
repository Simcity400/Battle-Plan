//! Pooled naval/history acquisition; conversion and membership joins run off-thread.
//!
//! Source-fed split (copy/decode): the theatre block (LandTheatres,
//! LandTheatreGroups, LandTheatreMembers, AirTheatres, AirTheatreGroups,
//! AirTheatreMembers) and the order-of-battle block (OrdersGroups,
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
    sampling::{full_match, qpc},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, comparison, comparison_rows},
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

/// Shared traversals are charged once to their first table; comparison costs
/// and the separately acquired reverse censuses are included in these totals.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("railway state", 2, &[2, 3]),
    ("land and air theatres", 4, &[4, 5, 6, 7, 8, 9]),
    ("retained naval results", 10, &[10, 11, 12]),
    ("army order of battle", 14, &[14, 15, 16]),
];

pub fn needs_order_sources(needed: &[bool]) -> bool {
    // The shared authenticated registry census feeds both the theatre reverse
    // membership and the order of battle, so any of them can request the second
    // pass. A table that does not need it still keeps its own outcome.
    needed[6] || needed[14] || needed[15] || needed[16]
}
pub fn needs_wing_sources(needed: &[bool]) -> bool {
    needed[9]
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub theatre_names: usize,
    pub sunk_names: usize,
    pub orders_sources: usize,
    pub wing_sources: usize,
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
    /// Second-pass outcomes of the order-of-battle read, kept beside the
    /// primary CaptureRows outcomes so each table compares independently.
    oob_second: [Outcome; 3],
    oob_finished: [bool; 2],
    theatre_names: Arena<u8>,
    sunk_names: Arena<u8>,
    orders_sources: CaptureRows<ctheatre::OrdersMembership>,
    wing_sources: CaptureRows<ctheatre::WingMembership>,
    orders_seen: Vec<u8>,
    wings_seen: Vec<u8>,
    orders_second: Outcome,
    wings_second: Outcome,
    orders_finished: [bool; 2],
    wings_finished: [bool; 2],
    source_ticks: [u64; 3],
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

impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        let theatre_success = *successful[4..10].iter().min().unwrap();
        let rail_success = successful[2].min(successful[3]);
        let result_success = *successful[10..13].iter().min().unwrap();
        Self {
            supremacy: CaptureRows::new(capacity.rows[0], successful[0]),
            convoys: CaptureRows::new(capacity.rows[1], successful[1]),
            cooldowns: CaptureRows::new(capacity.rows[2], rail_success),
            edges: CaptureRows::new(capacity.rows[3], rail_success),
            land: CaptureRows::new(capacity.rows[4], theatre_success),
            land_groups: CaptureRows::new(capacity.rows[5], theatre_success),
            land_members: CaptureRows::new(capacity.rows[6], theatre_success),
            air: CaptureRows::new(capacity.rows[7], theatre_success),
            air_groups: CaptureRows::new(capacity.rows[8], theatre_success),
            air_members: CaptureRows::new(capacity.rows[9], theatre_success),
            results: CaptureRows::new(capacity.rows[10], result_success),
            result_members: CaptureRows::new(capacity.rows[11], result_success),
            result_countries: CaptureRows::new(capacity.rows[12], result_success),
            sunk: CaptureRows::new(capacity.rows[13], successful[13]),
            groups: CaptureRows::new(capacity.rows[14], successful[14]),
            group_members: CaptureRows::new(capacity.rows[15], successful[15]),
            group_children: CaptureRows::new(capacity.rows[16], successful[16]),
            oob_second: [source_unavailable(); 3],
            oob_finished: [false; 2],
            theatre_names: Arena::new(capacity.theatre_names, theatre_success),
            sunk_names: Arena::new(capacity.sunk_names, successful[13]),
            orders_sources: CaptureRows::new(capacity.orders_sources, successful[6]),
            wing_sources: CaptureRows::new(capacity.wing_sources, successful[9]),
            orders_seen: comparison_rows(capacity.orders_sources, successful[6]),
            wings_seen: comparison_rows(capacity.wing_sources, successful[9]),
            orders_second: source_unavailable(),
            wings_second: source_unavailable(),
            orders_finished: [false; 2],
            wings_finished: [false; 2],
            source_ticks: [0; 3],
            raw_mode: false,
            supremacy_raw_mode: false,
            sunk_raw_mode: false,
        }
    }

    /// Call before shared map acquisition; capture() preserves these source visits.
    pub fn reset(&mut self) {
        self.reset_tables();
        self.orders_sources.reset();
        self.wing_sources.reset();
        self.groups.reset();
        self.group_members.reset();
        self.group_children.reset();
        self.oob_second = [source_unavailable(); 3];
        self.oob_finished = [false; 2];
        self.orders_sources.outcome = Outcome::Rows { len: 0 };
        self.wing_sources.outcome = Outcome::Rows { len: 0 };
        self.orders_second = Outcome::Rows { len: 0 };
        self.wings_second = Outcome::Rows { len: 0 };
        self.orders_finished = [false; 2];
        self.wings_finished = [false; 2];
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
            + self.theatre_names.allocated_bytes()
            + self.sunk_names.allocated_bytes()
            + self.orders_sources.allocated_bytes()
            + self.wing_sources.allocated_bytes()
            + self.orders_seen.capacity()
            + self.wings_seen.capacity()
    }

    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        let theatre_success = *successful[4..10].iter().min().unwrap();
        let rail_success = successful[2].min(successful[3]);
        let result_success = *successful[10..13].iter().min().unwrap();
        self.supremacy.grow(&mut capacity.rows[0], successful[0]);
        self.convoys.grow(&mut capacity.rows[1], successful[1]);
        self.cooldowns.grow(&mut capacity.rows[2], rail_success);
        self.edges.grow(&mut capacity.rows[3], rail_success);
        self.land.grow(&mut capacity.rows[4], theatre_success);
        self.land_groups
            .grow(&mut capacity.rows[5], theatre_success);
        self.land_members
            .grow(&mut capacity.rows[6], theatre_success);
        self.air.grow(&mut capacity.rows[7], theatre_success);
        self.air_groups.grow(&mut capacity.rows[8], theatre_success);
        self.air_members
            .grow(&mut capacity.rows[9], theatre_success);
        self.results.grow(&mut capacity.rows[10], result_success);
        self.result_members
            .grow(&mut capacity.rows[11], result_success);
        self.result_countries
            .grow(&mut capacity.rows[12], result_success);
        self.sunk.grow(&mut capacity.rows[13], successful[13]);
        self.groups.grow(&mut capacity.rows[14], successful[14]);
        self.group_members
            .grow(&mut capacity.rows[15], successful[15]);
        self.group_children
            .grow(&mut capacity.rows[16], successful[16]);
        self.theatre_names
            .grow(&mut capacity.theatre_names, theatre_success);
        self.sunk_names
            .grow(&mut capacity.sunk_names, successful[13]);
        self.orders_sources
            .grow(&mut capacity.orders_sources, successful[6]);
        self.wing_sources
            .grow(&mut capacity.wing_sources, successful[9]);
        comparison(
            &mut self.orders_seen,
            capacity.orders_sources,
            successful[6],
        );
        comparison(&mut self.wings_seen, capacity.wing_sources, successful[9]);
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
        second: bool,
    ) {
        let begin = qpc();
        let output = if second {
            &mut self.orders_sources.second
        } else {
            &mut self.orders_sources.rows
        };
        let outcome = match source {
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
        if second {
            self.orders_second = outcome;
        } else {
            self.orders_sources.outcome = outcome;
        }
        self.orders_finished[usize::from(second)] = true;
        self.source_ticks[0] = self.source_ticks[0].saturating_add(elapsed(begin));
        self.set_order_of_battle(memory, base, roots, source, second);
    }

    /// Reads every group's identity, its ordered member vector and, for army
    /// groups, its ordered child vector from the same authenticated census.
    /// A truncated or unavailable registry cannot establish a complete order of
    /// battle, so it fails all three tables with that reason.
    fn set_order_of_battle(
        &mut self,
        memory: &impl Memory,
        base: usize,
        roots: &[corders_group::Root],
        source: Outcome,
        second: bool,
    ) {
        let begin = qpc();
        let outcomes = match source {
            Outcome::Rows { len } if len <= roots.len() => {
                let (groups, members, children) = if second {
                    (
                        &mut self.groups.second,
                        &mut self.group_members.second,
                        &mut self.group_children.second,
                    )
                } else {
                    (
                        &mut self.groups.rows,
                        &mut self.group_members.rows,
                        &mut self.group_children.rows,
                    )
                };
                let capacity = [groups.len(), members.len(), children.len()];
                let walk = corders_group::walk_order_of_battle(
                    memory,
                    base,
                    &roots[..len],
                    &mut corders_group::OrderOfBattle {
                        groups,
                        members,
                        children,
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
        if second {
            self.oob_second = outcomes;
        } else {
            self.groups.outcome = outcomes[0];
            self.group_members.outcome = outcomes[1];
            self.group_children.outcome = outcomes[2];
        }
        self.oob_finished[usize::from(second)] = true;
        self.source_ticks[2] = self.source_ticks[2].saturating_add(elapsed(begin));
    }

    /// The order-of-battle tables are filled by the shared source callback, so
    /// their comparisons and their dependence on a finished first pass are
    /// settled here rather than inside a walker.
    fn finish_order_of_battle(&mut self, needed: &[bool], matched: &mut [bool; N]) {
        let others = if self.oob_finished[1] {
            self.oob_second
        } else {
            [source_unavailable(); 3]
        };
        if needed[14] {
            matched[14] = compare(&mut self.groups, others[0], true);
        }
        if needed[15] {
            matched[15] = compare(&mut self.group_members, others[1], true);
        }
        if needed[16] {
            matched[16] = compare(&mut self.group_children, others[2], true);
        }
        if !self.oob_finished[0] {
            self.groups.outcome = source_unavailable();
            self.group_members.outcome = source_unavailable();
            self.group_children.outcome = source_unavailable();
            matched[14..17].fill(false);
        }
    }

    pub fn visit_wing(
        &mut self,
        memory: &impl Memory,
        _base: usize,
        pointer: usize,
        wing: &cair_wing::Row,
        second: bool,
    ) {
        let begin = qpc();
        let (output, outcome) = if second {
            (&mut self.wing_sources.second, &mut self.wings_second)
        } else {
            (&mut self.wing_sources.rows, &mut self.wing_sources.outcome)
        };
        let row = if outcome.len() < output.len() {
            ctheatre::read_wing_membership(memory, pointer, wing)
        } else {
            Ok(ctheatre::WingMembership::default())
        };
        push(output, outcome, row);
        self.source_ticks[1] = self.source_ticks[1].saturating_add(elapsed(begin));
    }

    pub fn finish_wings(&mut self, source: Outcome, second: bool) {
        let begin = qpc();
        let outcome = if second {
            &mut self.wings_second
        } else {
            &mut self.wing_sources.outcome
        };
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
        self.wings_finished[usize::from(second)] = true;
        self.source_ticks[1] = self.source_ticks[1].saturating_add(elapsed(begin));
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        self.reset_tables();
        let mut matched = [false; N];
        let mut ticks = [0; N];
        // Hourly supremacy raw-copy split: on hours with a valid raw payload
        // the game thread stages bytes via `copy_supremacy_refresh` and the
        // decode worker fills the typed rows before the ready handoff, so
        // the typed walk is skipped here. Matched flags and costs come from
        // the raw compare, applied by the caller after this returns.
        if self.supremacy_raw_mode {
            // Refresh payload staged: leave the reset outcome for the decode
            // worker, which installs identical rows and outcomes.
            matched[0] = false;
            ticks[0] = 0;
        } else {
            let begin = qpc();
            matched[0] = capture_rows(&mut self.supremacy, needed[0], |rows| {
                cnaval_region_dominance::walk(memory, base, game_state, rows)
            });
            ticks[0] = elapsed(begin);
        }
        let begin = qpc();
        matched[1] = capture_rows(&mut self.convoys, needed[1], |rows| {
            csunk_convoy_info::walk(memory, base, game_state, rows)
        });
        ticks[1] = elapsed(begin);

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
        if needed[2] || needed[3] {
            let second = crailway_manager::walk(
                memory,
                base,
                game_state,
                &mut self.cooldowns.second,
                &mut self.edges.second,
            );
            matched[2] = compare(&mut self.cooldowns, second.provinces, needed[2]);
            matched[3] = compare(&mut self.edges, second.edges, needed[3]);
        }
        ticks[2] = elapsed(begin);

        // Raw-copy split (hourly source-fed naval block): the game thread
        // staged raw bytes and the decode worker replays theatres/OOB before
        // the ready handoff. Skip those walks here; matched flags and costs
        // install from the copy report in the tick.
        if !self.raw_mode {
            let begin = qpc();
            self.capture_theatres(memory, base, game_state, needed, &mut matched);
            ticks[4] = elapsed(begin);
            ticks[6] = self.source_ticks[0];
            ticks[9] = self.source_ticks[1];
            ticks[14] = self.source_ticks[2];
            self.finish_order_of_battle(needed, &mut matched);
        }

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
        if needed[10..13].iter().any(|&needed| needed) {
            let second = cnaval_combat_results::walk(
                memory,
                base,
                game_state,
                &mut self.results.second,
                &mut self.result_members.second,
                &mut self.result_countries.second,
            );
            matched[10] = compare(&mut self.results, second.records, needed[10]);
            matched[11] = compare(&mut self.result_members, second.members, needed[11]);
            matched[12] = compare(&mut self.result_countries, second.countries, needed[12]);
        }
        ticks[10] = elapsed(begin);

        let begin = qpc();
        // Hourly sunk-ships raw-copy split: on hours with a valid raw payload
        // the game thread stages bytes via `copy_sunk_refresh` and the decode
        // worker fills the typed rows before the ready handoff, so the typed
        // walk is skipped here.
        if self.sunk_raw_mode {
            // Refresh payload staged: leave the reset outcome for the decode
            // worker, which installs identical rows and outcomes.
            matched[13] = false;
            ticks[13] = 0;
        } else {
            matched[13] = self.capture_sunk(memory, base, game_state, current_hour, needed[13]);
            ticks[13] = elapsed(begin);
        }
        (matched, ticks)
    }

    fn capture_theatres(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        needed: &[bool],
        matched: &mut [bool; N],
    ) {
        let first = ctheatre::walk(
            memory,
            base,
            game_state,
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
        self.land_groups.outcome = bounded(first.land_groups, self.land_groups.rows.len());
        self.land_members.outcome = bounded(first.land_members, self.land_members.rows.len());
        self.air.outcome = bounded(first.air, self.air.rows.len());
        self.air_groups.outcome = bounded(first.air_groups, self.air_groups.rows.len());
        self.air_members.outcome = bounded(first.air_members, self.air_members.rows.len());
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
        if needed[4..10].iter().any(|&needed| needed) {
            let second = ctheatre::walk(
                memory,
                base,
                game_state,
                ctheatre::Buffers {
                    land: &mut self.land.second,
                    land_groups: &mut self.land_groups.second,
                    land_members: &mut self.land_members.second,
                    air: &mut self.air.second,
                    air_groups: &mut self.air_groups.second,
                    air_members: &mut self.air_members.second,
                    names: &mut self.theatre_names.second,
                },
            );
            self.theatre_names.required = self.theatre_names.required.max(second.names_required);
            let second_names = self
                .theatre_names
                .second
                .get(..second.names_len)
                .filter(|_| second.names_required >= second.names_len);
            matched[4] = compare(&mut self.land, second.land, needed[4]);
            matched[5] = compare_named(
                &mut self.land_groups,
                second.land_groups,
                needed[5],
                first_names,
                second_names,
                |r| (r.name_start, r.name_len),
                |r| {
                    let mut r = *r;
                    r.name_start = 0;
                    r
                },
            );
            matched[6] = compare(&mut self.land_members, second.land_members, needed[6]);
            matched[7] = compare_named(
                &mut self.air,
                second.air,
                needed[7],
                first_names,
                second_names,
                |r| (r.name_start, r.name_len),
                |r| {
                    let mut r = *r;
                    r.name_start = 0;
                    r
                },
            );
            matched[8] = compare_named(
                &mut self.air_groups,
                second.air_groups,
                needed[8],
                first_names,
                second_names,
                |r| (r.name_start, r.name_len),
                |r| {
                    let mut r = *r;
                    r.name_start = 0;
                    r
                },
            );
            matched[9] = compare(&mut self.air_members, second.air_members, needed[9]);
        }
        // During the finite consistency window, both complete forward/backlink
        // relationships are checked too. Only copied census rows are sorted;
        // pooled seen bytes detect duplicates and omitted memberships exactly.
        if needed[6] {
            let other = if self.orders_finished[1] {
                self.orders_second
            } else {
                source_unavailable()
            };
            let source_match = compare(&mut self.orders_sources, other, true);
            self.land_members.mismatches += self.orders_sources.mismatches;
            matched[6] &= source_match && self.orders_finished[0];
            if matched[6] {
                let agrees = checked_membership(
                    self.land_members.outcome,
                    &self.land_members.rows,
                    |r| (r.orders_group, r.theater_group),
                    self.orders_sources.outcome,
                    &mut self.orders_sources.rows,
                    |r| (r.orders_group, r.theater_group),
                    &mut self.orders_seen,
                ) && checked_membership(
                    self.land_members.outcome,
                    &self.land_members.second,
                    |r| (r.orders_group, r.theater_group),
                    self.orders_second,
                    &mut self.orders_sources.second,
                    |r| (r.orders_group, r.theater_group),
                    &mut self.orders_seen,
                );
                self.land_members.mismatches += usize::from(!agrees);
                matched[6] = agrees;
            }
        }
        if needed[9] {
            let other = if self.wings_finished[1] {
                self.wings_second
            } else {
                source_unavailable()
            };
            let source_match = compare(&mut self.wing_sources, other, true);
            self.air_members.mismatches += self.wing_sources.mismatches;
            matched[9] &= source_match && self.wings_finished[0];
            if matched[9] {
                let agrees = checked_membership(
                    self.air_members.outcome,
                    &self.air_members.rows,
                    |r| (r.wing, r.group),
                    self.wing_sources.outcome,
                    &mut self.wing_sources.rows,
                    |r| (r.wing, r.group),
                    &mut self.wings_seen,
                ) && checked_membership(
                    self.air_members.outcome,
                    &self.air_members.second,
                    |r| (r.wing, r.group),
                    self.wings_second,
                    &mut self.wing_sources.second,
                    |r| (r.wing, r.group),
                    &mut self.wings_seen,
                );
                self.air_members.mismatches += usize::from(!agrees);
                matched[9] = agrees;
            }
        }
        if !complete_source(&self.orders_sources, self.orders_finished[0]) {
            self.land_members.outcome = source_unavailable();
            matched[6] = false;
        }
        if !complete_source(&self.wing_sources, self.wings_finished[0]) {
            self.air_members.outcome = source_unavailable();
            matched[9] = false;
        }
    }

    fn capture_sunk(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        needed: bool,
    ) -> bool {
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
        if !needed {
            return false;
        }
        self.sunk.checked = true;
        let second = csunk_ship_info::walk(
            memory,
            base,
            game_state,
            current_hour,
            &mut self.sunk.second,
            &mut self.sunk_names.second,
        );
        let mut outcome = bounded(second.outcome, self.sunk.second.len());
        if !valid_sunk(
            outcome,
            &self.sunk.second,
            &self.sunk_names.second,
            second.bytes_len,
            second.required_bytes,
        ) {
            outcome = invalid_names();
        }
        self.sunk.compare(outcome);
        self.sunk.mismatches += usize::from(
            !self
                .sunk_names
                .compare(second.bytes_len, second.required_bytes),
        );
        full_match(self.sunk.outcome, outcome, self.sunk.mismatches)
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            self.supremacy.mismatches,
            self.convoys.mismatches,
            self.cooldowns.mismatches,
            self.edges.mismatches,
            self.land.mismatches,
            self.land_groups.mismatches,
            self.land_members.mismatches,
            self.air.mismatches,
            self.air_groups.mismatches,
            self.air_members.mismatches,
            self.results.mismatches,
            self.result_members.mismatches,
            self.result_countries.mismatches,
            self.sunk.mismatches,
            self.groups.mismatches,
            self.group_members.mismatches,
            self.group_children.mismatches,
        ]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        let mut result = [
            effective(&self.supremacy),
            effective(&self.convoys),
            effective(&self.cooldowns),
            effective(&self.edges),
            effective(&self.land),
            effective(&self.land_groups),
            effective(&self.land_members),
            effective(&self.air),
            effective(&self.air_groups),
            effective(&self.air_members),
            effective(&self.results),
            effective(&self.result_members),
            effective(&self.result_countries),
            effective(&self.sunk),
            effective(&self.groups),
            effective(&self.group_members),
            effective(&self.group_children),
        ];
        if !complete_source(&self.orders_sources, self.orders_finished[0]) {
            result[6] = source_unavailable();
        }
        if !complete_source(&self.wing_sources, self.wings_finished[0]) {
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
            effective(&self.orders_sources),
            &self.orders_sources.rows,
            |r| (r.orders_group, r.theater_group),
        ) {
            outcomes[6] = Outcome::Unavailable { reason };
        }
        if let Some(reason) = membership_issue(
            outcomes[9],
            &self.air_members.rows,
            |r| (r.wing, r.group),
            effective(&self.wing_sources),
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
        let outcome = effective(&self.sunk);
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
fn effective<R>(captured: &CaptureRows<R>) -> Outcome {
    if captured.checked && captured.mismatches != 0 {
        Outcome::Unavailable {
            reason: "table changed during double read",
        }
    } else {
        captured.outcome
    }
}
fn complete_source<R>(captured: &CaptureRows<R>, finished: bool) -> bool {
    finished && matches!(effective(captured), Outcome::Rows { len } if len <= captured.rows.len())
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
fn compare<R: Default + Clone + PartialEq>(
    captured: &mut CaptureRows<R>,
    other: Outcome,
    needed: bool,
) -> bool {
    if !needed {
        return false;
    }
    captured.checked = true;
    captured.compare(bounded(other, captured.second.len()))
}
fn capture_rows<R: Default + Clone + PartialEq>(
    captured: &mut CaptureRows<R>,
    needed: bool,
    mut walk: impl FnMut(&mut [R]) -> Outcome,
) -> bool {
    captured.outcome = bounded(walk(&mut captured.rows), captured.rows.len());
    if !needed {
        return false;
    }
    let other = bounded(walk(&mut captured.second), captured.second.len());
    compare(captured, other, true)
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
fn compare_named<R: PartialEq>(
    captured: &mut CaptureRows<R>,
    other: Outcome,
    needed: bool,
    first_bytes: Option<&[u8]>,
    other_bytes: Option<&[u8]>,
    name: impl Fn(&R) -> (usize, usize),
    without_offset: impl Fn(&R) -> R,
) -> bool {
    if !needed {
        return false;
    }
    captured.checked = true;
    let mut other = bounded(other, captured.second.len());
    if !valid_named(other, &captured.second, other_bytes, &name) {
        other = invalid_names();
    }
    captured.mismatches = if captured.outcome != other {
        1
    } else {
        captured.rows[..captured.outcome.len()]
            .iter()
            .zip(&captured.second[..other.len()])
            .filter(|(a, b)| {
                let (a_start, a_len) = name(a);
                let (b_start, b_len) = name(b);
                without_offset(a) != without_offset(b)
                    || first_bytes.and_then(|bytes| range(bytes, a_start, a_len))
                        != other_bytes.and_then(|bytes| range(bytes, b_start, b_len))
            })
            .count()
    };
    full_match(captured.outcome, other, captured.mismatches)
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

/// Finite consistency checks only. O(n log n), caller-owned buffers, exact keys.
fn checked_membership<A, B>(
    forward: Outcome,
    rows: &[A],
    member: impl Fn(&A) -> (u64, u64),
    reverse: Outcome,
    sources: &mut [B],
    backlink: impl Fn(&B) -> (u64, u64),
    seen: &mut [u8],
) -> bool {
    let (Outcome::Rows { len }, Outcome::Rows { len: source_len }) = (forward, reverse) else {
        return false;
    };
    let Some(rows) = rows.get(..len) else {
        return false;
    };
    let Some(sources) = sources.get_mut(..source_len) else {
        return false;
    };
    let Some(seen) = seen.get_mut(..source_len) else {
        return false;
    };
    sources.sort_unstable_by_key(|source| backlink(source).0);
    if sources
        .windows(2)
        .any(|pair| backlink(&pair[0]).0 == backlink(&pair[1]).0)
    {
        return false;
    }
    seen.fill(0);
    for row in rows {
        let (id, group) = member(row);
        let Ok(index) = sources.binary_search_by_key(&id, |source| backlink(source).0) else {
            return false;
        };
        if group == 0 || backlink(&sources[index]).1 != group || seen[index] != 0 {
            return false;
        }
        seen[index] = 1;
    }
    sources
        .iter()
        .zip(seen)
        .all(|(source, seen)| (*seen != 0) == (backlink(source).1 != 0))
}

pub(crate) const NAVAL_PACK_N: usize = 9;
/// Fault-attribution masks in closure order: [LandTheatres,
/// LandTheatreGroups, LandTheatreMembers, AirTheatres, AirTheatreGroups,
/// AirTheatreMembers, OrdersGroups, OrdersGroupMembers, ArmyGroupMembers].
/// Shared theatre roots block all six theatre tables; shared OOB roots block
/// all three OOB tables, matching the typed walks' error scope (root
/// failures fail the whole block; body failures are still attributed to the
/// whole block conservatively, so checks block together while rows,
/// outcomes and reasons stay byte-identical via replay).
const THEATRE_MASK: u32 = 0x3F;
const OOB_MASK: u32 = 0x1C0;

#[derive(Debug, Default)]
pub(crate) struct NavalJob {
    pub log: RawLog<NAVAL_PACK_N>,
    pub game_state: usize,
    wing_rows: Vec<cair_wing::Row>,
    wing_sources: Vec<ctheatre::WingMembership>,
    wing_outcome: Option<Outcome>,
    capacity_charge: crate::raw_memory::Charge,
}

impl NavalJob {
    pub fn new() -> Self {
        let mut job = Self {
            log: RawLog::new(1 << 17, 4 << 20),
            game_state: 0,
            wing_rows: vec![cair_wing::Row::default(); 4096],
            wing_sources: Vec::with_capacity(4096),
            wing_outcome: None,
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        job.update_capacity_charge();
        job
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.wing_sources.clear();
        self.wing_outcome = None;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next() || matches!(self.wing_outcome, Some(Outcome::Truncated { .. }))
    }

    pub fn grow(&mut self) {
        self.log.grow();
        if let Some(Outcome::Truncated { len, unread_rows }) = self.wing_outcome {
            let target = (len + unread_rows as usize).max(self.wing_rows.len() * 2);
            self.wing_rows.resize(target, cair_wing::Row::default());
            self.wing_sources
                .reserve_exact(target.saturating_sub(self.wing_sources.len()));
        }
        self.update_capacity_charge();
    }

    fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            auxiliary_vectors: vector_bytes(&self.wing_rows) + vector_bytes(&self.wing_sources),
            ..Default::default()
        });
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct NavalCopyReport {
    pub raw_ok: bool,
    pub matched: [bool; NAVAL_PACK_N],
    pub copy_ticks: [u64; NAVAL_PACK_N],
    pub raw_bytes: [u64; NAVAL_PACK_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NavalCopyContext {
    pub base: usize,
    pub game_state: usize,
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
) -> Option<u32> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = nspan(log, memory, links.data.checked_add(offset), 4, THEATRE_MASK)?;
    let slot = npool_u32(log, start);
    if slot == 0 || slot >= countries {
        return None;
    }
    Some(slot)
}

fn copy_naval_theatres(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) {
    use engine::{carmy_group, ccountry, ctheater_group};
    // Country census exactly like the fleet/wing censuses.
    let countries = match nspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::COUNTRIES),
        16,
        THEATRE_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let links = match nspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::COUNTRY_LINKS),
        16,
        THEATRE_MASK,
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
        let pointer = match nspan(log, memory, pointer_addr, 8, THEATRE_MASK)
            .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0))
        {
            Some(pointer) => pointer,
            None => continue,
        };
        let vtable = match nspan(log, memory, Some(pointer), 8, THEATRE_MASK) {
            Some(start) => start,
            None => continue,
        };
        if npool_u64(log, vtable) != country_vtable {
            return;
        }
        let id_start = match nspan(
            log,
            memory,
            pointer.checked_add(ccountry::ID),
            4,
            THEATRE_MASK,
        ) {
            Some(start) => start,
            None => continue,
        };
        let id = npool_u32(log, id_start);
        let canonical = match theatre_link(log, memory, &links_array, countries_array.count, id) {
            Some(canonical) => canonical,
            None => return,
        };
        if theatre_link(log, memory, &links_array, countries_array.count, slot) != Some(canonical) {
            return;
        }
        if theatre_link(log, memory, &links_array, countries_array.count, canonical)
            != Some(canonical)
        {
            return;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|o| countries_array.data.checked_add(o));
        let canonical_pointer = match nspan(log, memory, canonical_addr, 8, THEATRE_MASK) {
            Some(start) => start,
            None => return,
        };
        if npool_u64(log, canonical_pointer) != pointer as u64 {
            return;
        }
        // Land theatres array + per-theatre/group/member/name spans.
        for theatres_off in [ccountry::THEATRES, ccountry::AIR_THEATRES] {
            let theatres_header = match nspan(
                log,
                memory,
                pointer.checked_add(theatres_off),
                16,
                THEATRE_MASK,
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
            for ordinal in 0..theatres.count {
                let entry_addr = theatres
                    .data
                    .checked_add((ordinal as usize).saturating_mul(8));
                let theatre = match nspan(log, memory, entry_addr, 8, THEATRE_MASK)
                    .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0))
                {
                    Some(theatre) => theatre,
                    None => continue,
                };
                // Theatre identity (vtable + handle 16), owner, volunteers,
                // owner link resolve, group vector.
                let _ = nspan(log, memory, Some(theatre), 16, THEATRE_MASK);
                let owner_start = nspan(
                    log,
                    memory,
                    theatre.checked_add(ctheatre::OWNER),
                    4,
                    THEATRE_MASK,
                );
                let _ = nspan(
                    log,
                    memory,
                    theatre.checked_add(ctheatre::VOLUNTEERS_THEATRE),
                    4,
                    THEATRE_MASK,
                );
                if let Some(owner_start) = owner_start {
                    let owner = npool_u32(log, owner_start);
                    if owner != 0 && owner < links_array.count {
                        let offset = (owner as usize).checked_mul(4);
                        let _ = nspan(
                            log,
                            memory,
                            offset.and_then(|o| links_array.data.checked_add(o)),
                            4,
                            THEATRE_MASK,
                        );
                    }
                }
                let groups_header = match nspan(
                    log,
                    memory,
                    theatre.checked_add(ctheatre::THEATER_GROUP_DATA),
                    16,
                    THEATRE_MASK,
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
                    let group = match nspan(log, memory, group_addr, 8, THEATRE_MASK)
                        .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0))
                    {
                        Some(group) => group,
                        None => continue,
                    };
                    // Group identity (vtable + handle 16), theatre backlink,
                    // name string, priority.
                    let _ = nspan(log, memory, Some(group), 16, THEATRE_MASK);
                    let _ = nspan(
                        log,
                        memory,
                        group.checked_add(ctheater_group::THEATRE),
                        8,
                        THEATRE_MASK,
                    );
                    // Name string header + payload + terminator (theatre arena).
                    copy_theatre_name(log, memory, group);
                    let _ = nspan(
                        log,
                        memory,
                        group.checked_add(ctheater_group::PRIORITY),
                        4,
                        THEATRE_MASK,
                    );
                    let members_header = match nspan(
                        log,
                        memory,
                        group.checked_add(ctheater_group::ORDERS_GROUP_DATA),
                        16,
                        THEATRE_MASK,
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
                        let member = match nspan(log, memory, member_addr, 8, THEATRE_MASK)
                            .and_then(|s| {
                                usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0)
                            }) {
                            Some(member) => member,
                            None => continue,
                        };
                        // Member vtable (8) for the normal/army-group check plus
                        // identity (vtable + handle 16) plus parents. Both
                        // lengths are recorded because the walk requests both
                        // exact spans (exact-match serving, no sub-ranges).
                        let _ = nspan(log, memory, Some(member), 8, THEATRE_MASK);
                        let _ = nspan(log, memory, Some(member), 16, THEATRE_MASK);
                        let _ = nspan(
                            log,
                            memory,
                            member.checked_add(corders_group::THEATER_GROUP),
                            8,
                            THEATRE_MASK,
                        );
                        let _ = nspan(
                            log,
                            memory,
                            member.checked_add(corders_group::THEATRE),
                            8,
                            THEATRE_MASK,
                        );
                        if log.overflowed {
                            return;
                        }
                    }
                }
            }
        }
    }
    let _ = carmy_group::VTABLE_RVA;
}

fn copy_theatre_name(log: &mut RawLog<NAVAL_PACK_N>, memory: &impl engine::Memory, group: usize) {
    use engine::msvc_string;
    let header_addr = match group.checked_add(engine::ctheater_group::NAME) {
        Some(addr) => addr,
        None => return,
    };
    let header_start = match nspan(log, memory, Some(header_addr), 32, THEATRE_MASK) {
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
    if nspan(log, memory, Some(end), 1, THEATRE_MASK).is_none() {
        return;
    }
    if len != 0 {
        let _ = nspan(log, memory, Some(source), len, THEATRE_MASK);
    }
}

fn copy_naval_oob(job: &mut NavalJob, memory: &impl engine::Memory, base: usize) {
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
        None => return,
    };
    let header = match nspan(
        log,
        memory,
        table.checked_add(registry::ENTRIES),
        17,
        OOB_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut header_bytes = [0u8; 17];
    header_bytes.copy_from_slice(&log.bytes[header..header + 17]);
    let entries = match usize::try_from(u64::from_le_bytes(header_bytes[0..8].try_into().unwrap()))
        .ok()
        .filter(|&e| e != 0)
    {
        Some(entries) => entries,
        None => return,
    };
    let mask = u32::from_le_bytes(header_bytes[12..16].try_into().unwrap());
    let distance = header_bytes[16];
    let capacity = match mask.checked_add(1) {
        Some(capacity) => capacity,
        None => return,
    };
    let terminal = match (capacity as usize).checked_add(distance as usize) {
        Some(terminal) => terminal,
        None => return,
    };
    let terminal_addr = entries
        .checked_add(terminal * bucket::STRIDE)
        .and_then(|p| p.checked_add(bucket::DISTANCE));
    let _ = nspan(log, memory, terminal_addr, 1, OOB_MASK);
    let group_vtable = match base.checked_add(corders_group::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    let army_group_vtable = match base.checked_add(engine::carmy_group::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    let mut first: u32 = 0;
    let terminal_u32 = terminal.min(1 << 20) as u32;
    while first < terminal_u32 {
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
            // OOB identity fields: army-group flag, motorization, country
            // (already above), theatre/theater-group/parent/leader/provider
            // references with target authentication for non-null pointers.
            let is_army_group = nspan(
                log,
                memory,
                pointer.checked_add(corders_group::IS_ARMY_GROUP),
                1,
                OOB_MASK,
            )
            .map(|start| log.bytes[start]);
            let _ = nspan(
                log,
                memory,
                pointer.checked_add(corders_group::MOTORIZATION_LEVEL),
                1,
                OOB_MASK,
            );
            for offset in [
                corders_group::THEATRE,
                corders_group::THEATER_GROUP,
                corders_group::PARENT,
                corders_group::LEADER,
                corders_group::HQ_PROVIDER,
            ] {
                let target = nspan(log, memory, pointer.checked_add(offset), 8, OOB_MASK)
                    .and_then(|s| usize::try_from(npool_u64(log, s)).ok());
                if let Some(target) = target.filter(|&p| p != 0) {
                    // References use a vtable + handle identity, except the
                    // HQ provider: it is an embedded member whose host is
                    // authenticated below, not a referenced object itself.
                    if offset != corders_group::HQ_PROVIDER {
                        let _ = nspan(log, memory, Some(target), 16, OOB_MASK);
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
            let members_header = match nspan(
                log,
                memory,
                pointer.checked_add(corders_group::MEMBER_DATA),
                16,
                OOB_MASK,
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
                    return;
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
                    None => continue,
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
                            return;
                        }
                    }
                }
            }
            // Order-membership reverse census for this root: theatre-group
            // pointer plus target identity when non-zero.
            let theater_group = nspan(
                log,
                memory,
                pointer.checked_add(corders_group::THEATER_GROUP),
                8,
                OOB_MASK,
            )
            .and_then(|s| usize::try_from(npool_u64(log, s)).ok());
            if let Some(theater_group) = theater_group.filter(|&p| p != 0) {
                let _ = nspan(log, memory, Some(theater_group), 16, OOB_MASK);
            }
            if log.overflowed {
                return;
            }
        }
        first += slots as u32;
    }
}

fn copy_naval_pass(
    job: &mut NavalJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; NAVAL_PACK_N],
) {
    job.clear();
    job.game_state = game_state;
    let started = qpc();
    copy_naval_theatres(&mut job.log, memory, base, game_state);
    ticks[0] += qpc().saturating_sub(started).max(0) as u64;
    if job.log.overflowed {
        return;
    }
    let started = qpc();
    copy_naval_oob(job, memory, base);
    ticks[6] += qpc().saturating_sub(started).max(0) as u64;
    if job.log.overflowed {
        return;
    }
    let started = qpc();
    copy_naval_wings(job, memory, base, game_state);
    ticks[0] += qpc().saturating_sub(started).max(0) as u64;
}

/// The reverse census needs authenticated wing identities and group handles.
/// The former mirror copied only group handles, then tried to replay the full
/// wing walker from missing bytes. Use the existing reader into bounded,
/// reusable storage until a complete cheaper copy contract is measured.
fn copy_naval_wings(
    job: &mut NavalJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) {
    let mut failure = None;
    let outcome = cair_wing::walk_with(
        memory,
        base,
        game_state,
        &mut job.wing_rows,
        |pointer, _, row| match ctheatre::read_wing_membership(memory, pointer, row) {
            Ok(source) => job.wing_sources.push(source),
            Err(reason) => {
                failure.get_or_insert(reason);
            }
        },
    );
    job.wing_outcome = Some(match outcome {
        Outcome::Rows { .. } => failure.map_or(outcome, |reason| Outcome::Unavailable { reason }),
        _ => outcome,
    });
}

pub(crate) fn copy_naval_refresh(
    memory: &impl engine::Memory,
    ctx: NavalCopyContext,
    first: &mut NavalJob,
    mut second: Option<&mut NavalJob>,
) -> NavalCopyReport {
    let mut ticks = [0u64; NAVAL_PACK_N];
    copy_naval_pass(first, memory, ctx.base, ctx.game_state, &mut ticks);
    let mut matched = [true; NAVAL_PACK_N];
    if let Some(second_job) = second.as_deref_mut() {
        let mut second_ticks = [0u64; NAVAL_PACK_N];
        copy_naval_pass(
            second_job,
            memory,
            ctx.base,
            ctx.game_state,
            &mut second_ticks,
        );
        for (total, extra) in ticks.iter_mut().zip(second_ticks) {
            *total += extra;
        }
        // Per-table partitions: shared theatre/order roots gate their
        // dependents, private spans only their own table.
        let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
        for (slot, matched) in matched.iter_mut().enumerate() {
            let equal = raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
            *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
        }
        matched[5] &= matches!(first.wing_outcome, Some(Outcome::Rows { .. }))
            && first.wing_outcome == second_job.wing_outcome
            && first.wing_sources == second_job.wing_sources;
    }
    let raw_ok = !first.log.overflowed && second.is_none_or(|job| !job.log.overflowed);
    NavalCopyReport {
        raw_ok,
        matched,
        copy_ticks: ticks,
        raw_bytes: first.log.raw_bytes,
    }
}

pub(crate) fn decode_naval_payload(
    captured: &mut Captured,
    base: usize,
    job: &NavalJob,
    decode_ticks: &mut [u64; NAVAL_PACK_N],
) {
    let recorded = crate::raw::Recorded::build(&job.log);
    let started = qpc();
    // Order roots replay (worker-side, grows until the registry fits so the
    // census never truncates here; OOB row truncation is handled by the
    // pooled buffers through set_order_sources below).
    let mut roots: Vec<corders_group::Root> = Vec::new();
    let roots_outcome = loop {
        let outcome = corders_group::walk_roots(&recorded, base, &mut roots);
        match outcome {
            Outcome::Truncated { .. } => {
                roots.resize(roots.len() * 2 + 16, corders_group::Root::default());
                continue;
            }
            _ => break outcome,
        }
    };
    // Order sources + OOB through the existing setters (identical rows,
    // outcomes and ticks to the game thread's source callbacks).
    captured.set_order_sources(&recorded, base, &roots, roots_outcome, false);
    // Theatre walk replay into pooled buffers with names.
    let first = ctheatre::walk(
        &recorded,
        base,
        job.game_state,
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
    captured.land_groups.outcome = bounded(first.land_groups, captured.land_groups.rows.len());
    captured.land_members.outcome = bounded(first.land_members, captured.land_members.rows.len());
    captured.air.outcome = bounded(first.air, captured.air.rows.len());
    captured.air_groups.outcome = bounded(first.air_groups, captured.air_groups.rows.len());
    captured.air_members.outcome = bounded(first.air_members, captured.air_members.rows.len());
    captured.theatre_names.len = first.names_len;
    captured.theatre_names.required = first.names_required;
    // Preserve the complete authoritative reverse census without attempting
    // an unrelated full wing decode from an incomplete mirror.
    let source_len = job.wing_sources.len();
    let copied = source_len.min(captured.wing_sources.rows.len());
    captured.wing_sources.rows[..copied].copy_from_slice(&job.wing_sources[..copied]);
    captured.wing_sources.outcome = match job.wing_outcome {
        Some(Outcome::Rows { .. }) if copied < source_len => Outcome::Truncated {
            len: copied,
            unread_rows: (source_len - copied) as u32,
        },
        Some(outcome) => outcome,
        None => source_unavailable(),
    };
    captured.wings_finished[0] = true;
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
}

impl SupremacyJob {
    pub fn new() -> Self {
        Self {
            // Late-campaign worst case is 49 supremacy rows (appendix
            // census); both pools start above that so steady state never
            // overflows (see the worst-case caps test).
            log: RawLog::new(1 << 12, 1 << 20),
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
pub(crate) struct SupremacyCopyReport {
    /// False when staging overflowed: the caller falls back to the typed
    /// walk (identical rows through the existing path) and enqueues no
    /// payload.
    pub raw_ok: bool,
    pub matched: [bool; SUPREMACY_N],
    pub copy_ticks: [u64; SUPREMACY_N],
    pub raw_bytes: [u64; SUPREMACY_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SupremacyCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; SUPREMACY_N],
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
    job.clear();
    job.game_state = game_state;
    // Value vectors are bulk spans (see above); the flag is set up front so
    // both passes serve enclosed chunks identically.
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

impl Captured {
    /// Game-thread raw copy for an hourly refresh. Copies `first` always and
    /// `second` while the supremacy table still needs double-read checks,
    /// compares the raw logs, and records copy-only costs plus the existing
    /// checked/mismatches bookkeeping. Returns whether the payload is
    /// complete (`raw_ok`); on staging overflow the caller falls back to the
    /// typed walk and enqueues nothing.
    pub(crate) fn copy_supremacy_refresh(
        &mut self,
        memory: &impl Memory,
        ctx: SupremacyCopyContext,
        first: &mut SupremacyJob,
        mut second: Option<&mut SupremacyJob>,
    ) -> SupremacyCopyReport {
        let mut ticks = [0u64; SUPREMACY_N];
        copy_supremacy_pass(first, memory, ctx.base, ctx.game_state, &mut ticks);
        let mut matched = [true; SUPREMACY_N];
        if let Some(second_job) = second.as_deref_mut() {
            let mut second_ticks = [0u64; SUPREMACY_N];
            copy_supremacy_pass(
                second_job,
                memory,
                ctx.base,
                ctx.game_state,
                &mut second_ticks,
            );
            for (total, extra) in ticks.iter_mut().zip(second_ticks) {
                *total += extra;
            }
            let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
            for (slot, matched) in matched.iter_mut().enumerate() {
                let equal = raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
                *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
            }
        }
        // Conservative checked bookkeeping mirroring the typed path.
        self.supremacy.checked = ctx.needed[0];
        self.supremacy.mismatches = usize::from(ctx.needed[0] && !matched[0]);
        let raw_ok = !first.log.overflowed && second.is_none_or(|job| !job.log.overflowed);
        SupremacyCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: first.log.raw_bytes,
        }
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
        }
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.hour = 0;
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
    pub matched: [bool; SUNK_N],
    pub copy_ticks: [u64; SUNK_N],
    pub raw_bytes: [u64; SUNK_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SunkCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub needed: [bool; SUNK_N],
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
    job.clear();
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

impl Captured {
    /// Game-thread raw copy for an hourly refresh. Copies `first` always and
    /// `second` while the sunk-ships table still needs double-read checks,
    /// compares the raw logs, and records copy-only costs plus the existing
    /// checked/mismatches bookkeeping. Returns whether the payload is
    /// complete (`raw_ok`); on staging overflow the caller falls back to the
    /// typed walk and enqueues nothing.
    pub(crate) fn copy_sunk_refresh(
        &mut self,
        memory: &impl Memory,
        ctx: SunkCopyContext,
        first: &mut SunkJob,
        mut second: Option<&mut SunkJob>,
    ) -> SunkCopyReport {
        let mut ticks = [0u64; SUNK_N];
        copy_sunk_pass(
            first,
            memory,
            ctx.base,
            ctx.game_state,
            ctx.hour,
            &mut ticks,
        );
        let mut matched = [true; SUNK_N];
        if let Some(second_job) = second.as_deref_mut() {
            let mut second_ticks = [0u64; SUNK_N];
            copy_sunk_pass(
                second_job,
                memory,
                ctx.base,
                ctx.game_state,
                ctx.hour,
                &mut second_ticks,
            );
            for (total, extra) in ticks.iter_mut().zip(second_ticks) {
                *total += extra;
            }
            let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
            for (slot, matched) in matched.iter_mut().enumerate() {
                let equal = raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
                *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
            }
        }
        // Conservative checked bookkeeping mirroring the typed path.
        self.sunk.checked = ctx.needed[0];
        self.sunk.mismatches = usize::from(ctx.needed[0] && !matched[0]);
        let raw_ok = !first.log.overflowed && second.is_none_or(|job| !job.log.overflowed);
        SunkCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: first.log.raw_bytes,
        }
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

    #[test]
    fn naval_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
        use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
        static LOCAL: Tracker = Tracker::new();
        let mut job = NavalJob::default();
        job.wing_rows.reserve_exact(3);
        job.wing_sources.reserve_exact(3);
        job.capacity_charge = Charge::empty_on(&LOCAL);
        job.update_capacity_charge();
        let expected =
            |job: &NavalJob| vector_bytes(&job.wing_rows) + vector_bytes(&job.wing_sources);
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

    use engine::memory::SliceMemory;

    fn small() -> Captured {
        Captured::with_capacities(
            &Capacities {
                rows: [2; N],
                theatre_names: 32,
                sunk_names: 32,
                orders_sources: 2,
                wing_sources: 2,
            },
            &[0; N],
        )
    }

    #[test]
    fn both_membership_checks_reject_omissions_duplicates_and_incomplete_censuses() {
        let link = |row: &(u64, u64)| *row;
        let first_id = 0x1234_5678_0000_0035;
        let second_id = 0x9abc_def0_0000_0035;
        let group = 0xfedc_ba98_0000_004e;
        let rows = [(second_id, group), (first_id, group)];
        let sources = [(first_id, group), (42, 0), (second_id, group)];
        let full = Outcome::Rows { len: 2 };
        let census = Outcome::Rows { len: 3 };
        assert!(membership_issue(full, &rows, link, census, &sources, link).is_none());
        assert!(checked_membership(
            full,
            &rows,
            link,
            census,
            &mut { sources },
            link,
            &mut [0; 3]
        ));
        for bad in [
            vec![(first_id, group)],
            vec![(first_id, group), (first_id, group)],
            vec![(first_id, group), (second_id, group ^ (1_u64 << 32))],
        ] {
            let outcome = Outcome::Rows { len: bad.len() };
            assert!(membership_issue(outcome, &bad, link, census, &sources, link).is_some());
            assert!(!checked_membership(
                outcome,
                &bad,
                link,
                census,
                &mut { sources },
                link,
                &mut [0; 3]
            ));
        }
        let partial = Outcome::Truncated {
            len: 2,
            unread_rows: 1,
        };
        assert!(membership_issue(full, &rows, link, partial, &sources, link).is_some());
        assert!(!checked_membership(
            full,
            &rows,
            link,
            partial,
            &mut { sources },
            link,
            &mut [0; 3]
        ));
        assert!(
            membership_issue(full, &rows, link, Outcome::Rows { len: 4 }, &sources, link).is_some()
        );
        assert!(!checked_membership(
            full,
            &rows,
            link,
            census,
            &mut { sources },
            link,
            &mut [0; 2]
        ));
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
        captured.visit_wing(&memory, 0, 0x1000, &wing, false);
        assert!(!complete_source(
            &captured.wing_sources,
            captured.wings_finished[0]
        ));
        captured.finish_wings(Outcome::Rows { len: 1 }, false);
        assert!(complete_source(
            &captured.wing_sources,
            captured.wings_finished[0]
        ));
        assert_eq!(
            captured.wing_sources.rows[0],
            ctheatre::WingMembership {
                wing: wing.id,
                group
            }
        );
        captured.reset();
        captured.visit_wing(&memory, 0, 0x1000, &wing, false);
        captured.finish_wings(
            Outcome::Truncated {
                len: 1,
                unread_rows: 2,
            },
            false,
        );
        assert!(!complete_source(
            &captured.wing_sources,
            captured.wings_finished[0]
        ));
        captured.reset();
        captured.visit_wing(&memory, 0, 0x1000, &wing, false);
        captured.finish_wings(Outcome::Rows { len: 2 }, false);
        assert!(matches!(
            captured.wing_sources.outcome,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn name_checks_compare_only_the_table_bytes_and_refuse_invalid_ranges() {
        let mut rows = CaptureRows::<ctheater_group::Row>::new(1, 0);
        rows.outcome = Outcome::Rows { len: 1 };
        rows.rows[0] = ctheater_group::Row {
            id: u64::MAX,
            priority: -1,
            name_start: 0,
            name_len: 2,
            ..Default::default()
        };
        rows.second[0] = rows.rows[0];
        rows.second[0].name_start = 3;
        let name = |r: &ctheater_group::Row| (r.name_start, r.name_len);
        let normalize = |r: &ctheater_group::Row| {
            let mut r = *r;
            r.name_start = 0;
            r
        };
        assert!(compare_named(
            &mut rows,
            Outcome::Rows { len: 1 },
            true,
            Some(b"ab"),
            Some(b"xyzab"),
            name,
            normalize
        ));
        assert!(!compare_named(
            &mut rows,
            Outcome::Rows { len: 1 },
            true,
            Some(b"ab"),
            Some(b"xyzac"),
            name,
            normalize
        ));
        rows.second[0].name_start = usize::MAX;
        assert!(!compare_named(
            &mut rows,
            Outcome::Rows { len: 1 },
            true,
            Some(b"ab"),
            Some(b"xyzab"),
            name,
            normalize
        ));
        assert!(rows.mismatches > 0);
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
    fn shared_theatre_comparison_storage_survives_until_every_dependent_check_finishes() {
        let mut capacity = Capacities {
            rows: [1; N],
            theatre_names: 8,
            sunk_names: 8,
            orders_sources: 1,
            wing_sources: 1,
        };
        let mut successful = [crate::sampling::CONSISTENCY_TARGET; N];
        successful[9] = 0;
        let mut captured = Captured::with_capacities(&capacity, &successful);
        assert_eq!(captured.land_groups.second.len(), 1);
        assert_eq!(captured.theatre_names.second.len(), 8);
        assert_eq!(captured.wings_seen.len(), 1);
        successful[9] = crate::sampling::CONSISTENCY_TARGET;
        captured.grow(&mut capacity, &successful);
        assert!(captured.land_groups.second.is_empty());
        assert!(captured.theatre_names.second.is_empty());
        assert!(captured.wing_sources.second.is_empty());
        assert!(captured.wings_seen.is_empty());
        assert_eq!(captured.land_groups.rows.len(), 1);
    }

    #[test]
    fn naval_pack_tables_are_hourly_tier_h_with_no_refresh_gate() {
        use replay::rows::TableId;
        for table in [
            TableId::LandTheatres,
            TableId::LandTheatreGroups,
            TableId::LandTheatreMembers,
            TableId::AirTheatres,
            TableId::AirTheatreGroups,
            TableId::AirTheatreMembers,
            TableId::OrdersGroups,
            TableId::OrdersGroupMembers,
            TableId::ArmyGroupMembers,
        ] {
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
    fn naval_split_copy_matches_direct_walk() {
        use super::naval_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 9]);
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
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut first = NavalJob::new();
        let report = super::copy_naval_refresh(&memory, ctx, &mut first, None);
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
            };
            let direct = kit::direct_with(&memory, &capacity);
            let (split, report, _) = kit::split_with(&memory, &capacity);
            assert!(report.raw_ok, "rows={rows:?}");
            assert_eq!(report.matched, [true; 9], "rows={rows:?}");
            kit::assert_matches_direct(&split, &direct);
        }
        // Exact-fit arm pins Rows on land/groups with both members present.
        let capacity = Capacities {
            rows: [2; 17],
            theatre_names: 1024,
            sunk_names: 1024,
            orders_sources: 64,
            wing_sources: 64,
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
        assert_eq!(report.matched, [true; 9]);
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
        for (fault, matched) in [
            (
                kit::GAME_STATE + engine::cgame_state::COUNTRIES,
                [false, false, false, false, false, false, true, true, true],
            ),
            (
                kit::THEATRE,
                [false, false, false, false, false, false, true, true, true],
            ),
            (
                kit::GROUP,
                [false, false, false, false, false, false, true, true, true],
            ),
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
            assert_eq!(report.matched, matched, "fault={fault:x}");
        }
    }

    #[test]
    fn naval_double_copy_compare_blocks_checks_on_tearing() {
        use super::naval_test_kit as kit;
        use super::{NavalCopyContext, NavalJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = NavalCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut first = NavalJob::new();
        let mut second = NavalJob::new();
        let report = super::copy_naval_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 9]);
        second.log.bytes[100] ^= 0xff;
        assert!(!first.log.logs_equal(&second.log));
    }

    #[test]
    fn naval_staging_overflow_falls_back_to_typed_walk() {
        use super::naval_test_kit as kit;
        use super::{NavalCopyContext, NavalJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = NavalCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut tiny = NavalJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
            ..Default::default()
        };
        let report = super::copy_naval_refresh(&memory, ctx, &mut tiny, None);
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
    fn supremacy_tables_are_hourly_tier_h_with_no_refresh_gate() {
        use replay::rows::TableId;
        assert!(!replay::cadence::is_staggered(TableId::NavalSupremacy));
        for hour in [0, 1, 2, 3, 60_820_531] {
            assert!(replay::cadence::is_refresh(TableId::NavalSupremacy, hour));
        }
    }

    #[test]
    fn supremacy_split_copy_matches_direct_walk() {
        use super::supremacy_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 1]);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.outcome, Outcome::Rows { len: 2 });
        assert_eq!(split.rows[0].country, u32::MAX);
        assert_eq!(split.rows[0].current, 1000);
        assert_eq!(split.rows[1].country, 1);
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
            assert_eq!(report.matched, [true; 1], "cap={cap}");
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
            assert_eq!(report.matched, [false; 1], "fault={fault:x}");
        }
    }

    #[test]
    fn supremacy_double_copy_compare_blocks_checks_on_tearing() {
        use super::supremacy_test_kit as kit;
        use super::{SupremacyCopyContext, SupremacyJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = SupremacyCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            needed: [true; 1],
        };
        let capacities = super::Capacities::default();
        let mut captured = super::Captured::with_capacities(&capacities, &[0; 17]);
        let mut first = SupremacyJob::new();
        let report = captured.copy_supremacy_refresh(&memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        let mut torn = bytes.clone();
        torn[kit::DOMINANCE + 0x100] ^= 0xff;
        let torn_memory = kit::memory(&torn);
        let mut second = SupremacyJob::new();
        let mut captured2 = super::Captured::with_capacities(&capacities, &[0; 17]);
        let torn_report = captured2.copy_supremacy_refresh(&torn_memory, ctx, &mut second, None);
        assert!(torn_report.raw_ok);
        assert!(!first.log.partition_equal(&second.log, 0));
        assert!(!first.log.logs_equal(&second.log));
    }

    #[test]
    fn supremacy_staging_overflow_falls_back_to_typed_walk() {
        use super::supremacy_test_kit as kit;
        use super::{SupremacyCopyContext, SupremacyJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = SupremacyCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            needed: [true; 1],
        };
        let mut tiny = SupremacyJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
        };
        let capacities = super::Capacities::default();
        let mut captured = super::Captured::with_capacities(&capacities, &[0; 17]);
        let report = captured.copy_supremacy_refresh(&memory, ctx, &mut tiny, None);
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
    fn sunk_tables_are_hourly_tier_h_with_no_refresh_gate() {
        use replay::rows::TableId;
        assert!(!replay::cadence::is_staggered(TableId::SunkShips));
        for hour in [0, 1, 2, 3, 60_820_531] {
            assert!(replay::cadence::is_refresh(TableId::SunkShips, hour));
        }
    }

    #[test]
    fn sunk_split_copy_matches_direct_walk() {
        use super::sunk_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 1]);
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
            assert_eq!(report.matched, [true; 1], "rows={rows} names={names}");
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
            assert_eq!(report.matched, [false; 1], "fault={fault:x}");
        }
    }

    #[test]
    fn sunk_double_copy_compare_blocks_checks_on_tearing() {
        use super::sunk_test_kit as kit;
        use super::{SunkCopyContext, SunkJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = SunkCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            hour: kit::HOUR,
            needed: [true; 1],
        };
        let capacities = super::Capacities::default();
        let mut captured = super::Captured::with_capacities(&capacities, &[0; 17]);
        let mut first = SunkJob::new();
        let report = captured.copy_sunk_refresh(&memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        let mut torn = bytes.clone();
        torn[kit::FIRST + engine::csunk_ship_info::VICTIM_COUNTRY] ^= 0xff;
        let torn_memory = kit::memory(&torn);
        let mut second = SunkJob::new();
        let mut captured2 = super::Captured::with_capacities(&capacities, &[0; 17]);
        let torn_report = captured2.copy_sunk_refresh(&torn_memory, ctx, &mut second, None);
        assert!(torn_report.raw_ok);
        assert!(!first.log.partition_equal(&second.log, 0));
        assert!(!first.log.logs_equal(&second.log));
    }

    #[test]
    fn sunk_staging_overflow_falls_back_to_typed_walk() {
        use super::sunk_test_kit as kit;
        use super::{SunkCopyContext, SunkJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = SunkCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            hour: kit::HOUR,
            needed: [true; 1],
        };
        let mut tiny = SunkJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
            hour: 0,
        };
        let capacities = super::Capacities::default();
        let mut captured = super::Captured::with_capacities(&capacities, &[0; 17]);
        let report = captured.copy_sunk_refresh(&memory, ctx, &mut tiny, None);
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

    pub(crate) const BASE: usize = 0;
    pub(crate) const GAME_STATE: usize = 0x10000;
    const CTAB: usize = 0x11000;
    const LTAB: usize = 0x12000;
    const C0: usize = 0x20000;
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
        let mut captured = Captured::with_capacities(capacity, &[0; 17]);
        // Typed reference: theatre walk + OOB via roots, single pass (no
        // double-read). Mirrors the game thread's capture order.
        let first = engine::ctheatre::walk(
            memory,
            BASE,
            GAME_STATE,
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

    pub(crate) fn split(memory: &impl Memory) -> (Split, NavalCopyReport, NavalJob) {
        let ctx = NavalCopyContext {
            base: BASE,
            game_state: GAME_STATE,
        };
        let mut first = NavalJob::new();
        let mut second = NavalJob::new();
        let report = super::copy_naval_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0; 17]);
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
            base: BASE,
            game_state: GAME_STATE,
        };
        let mut first = NavalJob::new();
        let mut second = NavalJob::new();
        let report = super::copy_naval_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut captured = Captured::with_capacities(capacity, &[0; 17]);
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

    pub(crate) fn direct_truncated(memory: &impl Memory) -> Split {
        let capacity = Capacities {
            rows: [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
            theatre_names: 8,
            sunk_names: 8,
            orders_sources: 1,
            wing_sources: 1,
        };
        collect(memory, &capacity)
    }

    pub(crate) fn split_truncated(memory: &impl Memory) -> (Split, NavalCopyReport, NavalJob) {
        let ctx = NavalCopyContext {
            base: BASE,
            game_state: GAME_STATE,
        };
        let mut first = NavalJob::new();
        let mut second = NavalJob::new();
        let report = super::copy_naval_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let capacity = Capacities {
            rows: [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
            theatre_names: 8,
            sunk_names: 8,
            orders_sources: 1,
            wing_sources: 1,
        };
        let mut captured = Captured::with_capacities(&capacity, &[0; 17]);
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
            let (replayed, report, _) = split(&memory);
            assert_matches_direct(&replayed, &direct);
            assert!(report.raw_ok, "{name}");
            let mut matched = [true; 9];
            if required_fault {
                matched[6..].fill(false);
            }
            assert_eq!(report.matched, matched, "{name}");
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
                    4,
                    "{name}"
                );
            }
            assert!(!reads.contains(&(OOB_MEMBER, 16)), "{name}");
            if provider {
                assert!(
                    reads.windows(3).any(|window| window
                        == [
                            (OOB_GROUP + corders_group::HQ_PROVIDER, 8),
                            (OOB_UNIT, 32),
                            (backlink, 8),
                        ]),
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
        let mut captured = Captured::with_capacities(&capacities, &[0; 17]);
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
            base: BASE,
            game_state: GAME_STATE,
            needed: [true; 1],
        };
        let mut capacities = Capacities::default();
        capacities.rows[0] = cap;
        let mut captured = Captured::with_capacities(&capacities, &[0; 17]);
        let mut first = SupremacyJob::new();
        let mut second = SupremacyJob::new();
        let report = captured.copy_supremacy_refresh(memory, ctx, &mut first, Some(&mut second));
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
        let mut captured = Captured::with_capacities(&capacities, &[0; 17]);
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
            base: BASE,
            game_state: GAME_STATE,
            hour: HOUR,
            needed: [true; 1],
        };
        let mut capacities = Capacities::default();
        capacities.rows[13] = rows_cap;
        capacities.sunk_names = names_cap;
        let mut captured = Captured::with_capacities(&capacities, &[0; 17]);
        let mut first = SunkJob::new();
        let mut second = SunkJob::new();
        let report = captured.copy_sunk_refresh(memory, ctx, &mut first, Some(&mut second));
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
