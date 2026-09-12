//! Pooled naval/history acquisition; conversion and membership joins run off-thread.
//!
//! Source-fed split (copy/decode): the theatre block (LandTheatres,
//! LandTheatreGroups, LandTheatreMembers, AirTheatres, AirTheatreGroups,
//! AirTheatreMembers) and the order-of-battle block (OrdersGroups,
//! OrdersGroupMembers, ArmyGroupMembers) move to the single FIFO decode
//! worker as part of the map-pack coupled unit (one task). The four
//! non-source-fed theatres move with the five source-fed tables because
//! they share the `ctheatre::walk` and OOB walks; splitting would duplicate
//! those walks with separate censuses. Each stages its own root refs from
//! its own copies with no cross-family decoded roots; zero-engine-change
//! replay of the existing walks against `Recorded` spans. Choice per table:
//! zero-engine-change (Wars/Divisions precedent). The other eight naval
//! tables stay game-side.

use crate::{
    raw::RawLog,
    sampling::{full_match, qpc},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, comparison, comparison_rows},
};
use engine::{
    Memory, cair_group, cair_theatre, cair_wing, cnaval_combat_results, cnaval_region_dominance,
    corders_group, cpdx_array::Array, cprovince_railway_info, crailway_manager, csunk_convoy_info,
    csunk_ship_info, ctheater_group, ctheatre, rail_neighbor, walk::Outcome,
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
            Outcome::Truncated { .. } => [Outcome::Unavailable {
                reason: "orders registry census truncated",
            }; 3],
            _ => [Outcome::Unavailable {
                reason: "orders registry census exceeds captured roots",
            }; 3],
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
        let begin = qpc();
        matched[0] = capture_rows(&mut self.supremacy, needed[0], |rows| {
            cnaval_region_dominance::walk(memory, base, game_state, rows)
        });
        ticks[0] = elapsed(begin);
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
        matched[13] = self.capture_sunk(memory, base, game_state, current_hour, needed[13]);
        ticks[13] = elapsed(begin);
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

#[derive(Debug)]
pub(crate) struct NavalJob {
    pub log: RawLog<NAVAL_PACK_N>,
    pub game_state: usize,
}

impl NavalJob {
    pub fn new() -> Self {
        Self {
            log: RawLog::new(1 << 17, 4 << 20),
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
    let countries = match nspan(log, memory, game_state.checked_add(engine::cgame_state::COUNTRIES), 16, THEATRE_MASK) {
        Some(start) => start,
        None => return,
    };
    let links = match nspan(log, memory, game_state.checked_add(engine::cgame_state::COUNTRY_LINKS), 16, THEATRE_MASK) {
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
        let pointer_addr = (slot as usize).checked_mul(8).and_then(|o| countries_array.data.checked_add(o));
        let pointer = match nspan(log, memory, pointer_addr, 8, THEATRE_MASK)
            .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0)) {
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
        let id_start = match nspan(log, memory, pointer.checked_add(ccountry::ID), 4, THEATRE_MASK) {
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
        if theatre_link(log, memory, &links_array, countries_array.count, canonical) != Some(canonical) {
            return;
        }
        let canonical_addr = (canonical as usize).checked_mul(8).and_then(|o| countries_array.data.checked_add(o));
        let canonical_pointer = match nspan(log, memory, canonical_addr, 8, THEATRE_MASK) {
            Some(start) => start,
            None => return,
        };
        if npool_u64(log, canonical_pointer) != pointer as u64 {
            return;
        }
        // Land theatres array + per-theatre/group/member/name spans.
        for theatres_off in [ccountry::THEATRES, ccountry::AIR_THEATRES] {
            let theatres_header = match nspan(log, memory, pointer.checked_add(theatres_off), 16, THEATRE_MASK) {
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
                let entry_addr = theatres.data.checked_add((ordinal as usize).saturating_mul(8));
                let theatre = match nspan(log, memory, entry_addr, 8, THEATRE_MASK)
                    .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0)) {
                    Some(theatre) => theatre,
                    None => continue,
                };
                // Theatre identity (vtable + handle 16), owner, volunteers,
                // owner link resolve, group vector.
                let _ = nspan(log, memory, Some(theatre), 16, THEATRE_MASK);
                let owner_start = nspan(log, memory, theatre.checked_add(ctheatre::OWNER), 4, THEATRE_MASK);
                let _ = nspan(log, memory, theatre.checked_add(ctheatre::VOLUNTEERS_THEATRE), 4, THEATRE_MASK);
                if let Some(owner_start) = owner_start {
                    let owner = npool_u32(log, owner_start);
                    if owner != 0 && owner < links_array.count {
                        let offset = (owner as usize).checked_mul(4);
                        let _ = nspan(log, memory, offset.and_then(|o| links_array.data.checked_add(o)), 4, THEATRE_MASK);
                    }
                }
                let groups_header = match nspan(log, memory, theatre.checked_add(ctheatre::THEATER_GROUP_DATA), 16, THEATRE_MASK) {
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
                    let group_addr = groups.data.checked_add((group_ordinal as usize).saturating_mul(8));
                    let group = match nspan(log, memory, group_addr, 8, THEATRE_MASK)
                        .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0)) {
                        Some(group) => group,
                        None => continue,
                    };
                    // Group identity (vtable + handle 16), theatre backlink,
                    // name string, priority.
                    let _ = nspan(log, memory, Some(group), 16, THEATRE_MASK);
                    let _ = nspan(log, memory, group.checked_add(ctheater_group::THEATRE), 8, THEATRE_MASK);
                    // Name string header + payload + terminator (theatre arena).
                    copy_theatre_name(log, memory, group);
                    let _ = nspan(log, memory, group.checked_add(ctheater_group::PRIORITY), 4, THEATRE_MASK);
                    let members_header = match nspan(log, memory, group.checked_add(ctheater_group::ORDERS_GROUP_DATA), 16, THEATRE_MASK) {
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
                        let member_addr = members.data.checked_add((member_ordinal as usize).saturating_mul(8));
                        let member = match nspan(log, memory, member_addr, 8, THEATRE_MASK)
                            .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0)) {
                            Some(member) => member,
                            None => continue,
                        };
                        // Member vtable (8) for the normal/army-group check plus
                        // identity (vtable + handle 16) plus parents. Both
                        // lengths are recorded because the walk requests both
                        // exact spans (exact-match serving, no sub-ranges).
                        let _ = nspan(log, memory, Some(member), 8, THEATRE_MASK);
                        let _ = nspan(log, memory, Some(member), 16, THEATRE_MASK);
                        let _ = nspan(log, memory, member.checked_add(corders_group::THEATER_GROUP), 8, THEATRE_MASK);
                        let _ = nspan(log, memory, member.checked_add(corders_group::THEATRE), 8, THEATRE_MASK);
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

fn copy_theatre_name(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine::Memory,
    group: usize,
) {
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

fn copy_naval_oob(
    job: &mut NavalJob,
    memory: &impl engine::Memory,
    base: usize,
) {    use engine::{creference_registry as registry, reference_bucket as bucket};
    let log = &mut job.log;
    let table = match nspan(
        log,
        memory,
        base
            .checked_add(registry::LOW_ROOTS_RVA)
            .and_then(|p| p.checked_add(corders_group::REFERENCE_TYPE as usize * 8)),
        8,
        OOB_MASK,
    )
    .and_then(|start| usize::try_from(npool_u64(log, start)).ok().filter(|&p| p != 0))
    {
        Some(table) => table,
        None => return,
    };
    let header = match nspan(log, memory, table.checked_add(registry::ENTRIES), 17, OOB_MASK) {
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
        let batch_start = match nspan(log, memory, Some(address), slots * bucket::STRIDE, OOB_MASK) {
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
            if npool_u64(log, identity) != group_vtable && npool_u64(log, identity) != army_group_vtable {
                continue;
            }
            if npool_u64(log, identity + 8) != handle {
                continue;
            }
            let country_start = match nspan(log, memory, pointer.checked_add(corders_group::COUNTRY), 4, OOB_MASK) {
                Some(start) => start,
                None => continue,
            };
            let _country = npool_u32(log, country_start);
            // OOB identity fields: army-group flag, motorization, country
            // (already above), theatre/theater-group/parent/leader/provider
            // references with target authentication for non-null pointers.
            let _ = nspan(log, memory, pointer.checked_add(corders_group::IS_ARMY_GROUP), 1, OOB_MASK);
            let _ = nspan(log, memory, pointer.checked_add(corders_group::MOTORIZATION_LEVEL), 1, OOB_MASK);
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
                    // Target vtable + handle for reference authentication.
                    // Parent targets additionally read IS_ARMY_GROUP; leader
                    // and provider targets read their own rows (recorded as
                    // superset spans here; the replay validates specifics).
                    let _ = nspan(log, memory, Some(target), 16, OOB_MASK);
                    if offset == corders_group::PARENT {
                        let _ = nspan(log, memory, target.checked_add(corders_group::IS_ARMY_GROUP), 1, OOB_MASK);
                    }
                    if offset == corders_group::HQ_PROVIDER {
                        // Provider is a member pointer: host identity span
                        // plus group backlink, like member_unit.
                        if let Some(unit) = target.checked_sub(engine::corders_group_member::HOST_OFFSET) {
                            let _ = nspan(log, memory, Some(unit), 32, OOB_MASK);
                        }
                        let _ = nspan(log, memory, target.checked_add(engine::corders_group_member::GROUP), 8, OOB_MASK);
                    }
                }
            }
            // Member vector with per-member host identity (32-byte header)
            // and group backlink, exactly like `member_unit`.
            let members_header = match nspan(log, memory, pointer.checked_add(corders_group::MEMBER_DATA), 16, OOB_MASK) {
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
                    .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0)) {
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
                let _ = nspan(log, memory, member.checked_add(engine::corders_group_member::GROUP), 8, OOB_MASK);
                if log.overflowed {
                    return;
                }
            }
            // Child vector for army groups with per-child 16-byte identity.
            let children_header = match nspan(log, memory, pointer.checked_add(engine::carmy_group::MEMBER_DATA), 16, OOB_MASK) {
                Some(start) => start,
                None => continue,
            };
            let mut children_raw = [0u8; 16];
            children_raw.copy_from_slice(&log.bytes[children_header..children_header + 16]);
            if let Some(children) = Array::from_header(children_raw, 8) {
                for index in 0..children.count {
                    let entry_addr = children.data.checked_add((index as usize).saturating_mul(8));
                    let child = match nspan(log, memory, entry_addr, 8, OOB_MASK)
                        .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0)) {
                        Some(child) => child,
                        None => continue,
                    };
                    let _ = nspan(log, memory, Some(child), 16, OOB_MASK);
                    if log.overflowed {
                        return;
                    }
                }
            }
            // Order-membership reverse census for this root: theatre-group
            // pointer plus target identity when non-zero.
            let theater_group = nspan(log, memory, pointer.checked_add(corders_group::THEATER_GROUP), 8, OOB_MASK)
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
    copy_naval_wings(&mut job.log, memory, base, game_state);
    ticks[0] += qpc().saturating_sub(started).max(0) as u64;
}

/// Mirror of the wing GROUP_HANDLE reads feeding the air-theatre reverse
/// census. Records the wing census (manager/owners/pools/wings with
/// THEATRE_MASK) plus one GROUP_HANDLE span per wing. Takes run
/// unconditionally; the replay derives wing membership itself. Empty air
/// theatres still record wing handles (superset, harmless).
fn copy_naval_wings(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) {
    use engine::{cair_wing, cair_wing_pool, cstrategic_air, cstrategic_air_manager};
    let countries = match nspan(log, memory, game_state.checked_add(engine::cgame_state::COUNTRIES), 16, THEATRE_MASK) {
        Some(start) => start,
        None => return,
    };
    let links = match nspan(log, memory, game_state.checked_add(engine::cgame_state::COUNTRY_LINKS), 16, THEATRE_MASK) {
        Some(start) => start,
        None => return,
    };
    let manager_ptr = match nspan(log, memory, game_state.checked_add(engine::cgame_state::STRATEGIC_AIR_MANAGER), 8, THEATRE_MASK)
        .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0)) {
        Some(manager_ptr) => manager_ptr,
        None => return,
    };
    let _ = nspan(log, memory, Some(manager_ptr), 8, THEATRE_MASK);
    let owners_header = match nspan(log, memory, manager_ptr.checked_add(cstrategic_air_manager::COUNTRY_AIR), 16, THEATRE_MASK) {
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
    raw.copy_from_slice(&log.bytes[owners_header..owners_header + 16]);
    let owners = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    if owners.count == 0 || countries_array.count < 2 || links_array.count == 0 {
        return;
    }
    for slot in 0..owners.count {
        let owner_addr = owners.data.checked_add((slot as usize).saturating_mul(8));
        let owner_pointer = match nspan(log, memory, owner_addr, 8, THEATRE_MASK)
            .and_then(|s| usize::try_from(npool_u64(log, s)).ok()) {
            Some(owner_pointer) => owner_pointer,
            None => continue,
        };
        if owner_pointer == 0 {
            continue;
        }
        let _ = nspan(log, memory, Some(owner_pointer), 8, THEATRE_MASK);
        let pools_header = match nspan(log, memory, owner_pointer.checked_add(cstrategic_air::POOLS), 16, THEATRE_MASK) {
            Some(start) => start,
            None => continue,
        };
        let mut header = [0u8; 16];
        header.copy_from_slice(&log.bytes[pools_header..pools_header + 16]);
        let pools = match Array::from_header(header, 8) {
            Some(array) => array,
            None => continue,
        };
        for pool_index in 0..pools.count {
            let pool_addr = pools.data.checked_add((pool_index as usize).saturating_mul(8));
            let pool = match nspan(log, memory, pool_addr, 8, THEATRE_MASK)
                .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0)) {
                Some(pool) => pool,
                None => continue,
            };
            let wings_header = match nspan(log, memory, pool.checked_add(cair_wing_pool::WINGS), 16, THEATRE_MASK) {
                Some(start) => start,
                None => continue,
            };
            let mut wings_raw = [0u8; 16];
            wings_raw.copy_from_slice(&log.bytes[wings_header..wings_header + 16]);
            let wings = match Array::from_header(wings_raw, 8) {
                Some(array) => array,
                None => continue,
            };
            for wing_index in 0..wings.count {
                let wing_addr = wings.data.checked_add((wing_index as usize).saturating_mul(8));
                let wing = match nspan(log, memory, wing_addr, 8, THEATRE_MASK)
                    .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0)) {
                    Some(wing) => wing,
                    None => continue,
                };
                let _ = nspan(log, memory, wing.checked_add(cair_wing::GROUP_HANDLE), 8, THEATRE_MASK);
                if log.overflowed {
                    return;
                }
            }
        }
    }
    let _ = (base, countries_array, links_array);
}

pub(crate) fn copy_naval_refresh(
    memory: &impl engine::Memory,
    ctx: NavalCopyContext,
    first: &mut NavalJob,
    second: Option<&mut NavalJob>,
) -> NavalCopyReport {
    let mut ticks = [0u64; NAVAL_PACK_N];
    copy_naval_pass(first, memory, ctx.base, ctx.game_state, &mut ticks);
    let mut matched = [true; NAVAL_PACK_N];
    if let Some(second_job) = second {
        let mut second_ticks = [0u64; NAVAL_PACK_N];
        copy_naval_pass(second_job, memory, ctx.base, ctx.game_state, &mut second_ticks);
        for (total, extra) in ticks.iter_mut().zip(second_ticks) {
            *total += extra;
        }
        // Per-table partitions: shared theatre/order roots gate their
        // dependents, private spans only their own table.
        let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
        for (slot, matched) in matched.iter_mut().enumerate() {
            let equal =
                raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
            *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
        }
    }
    let raw_ok = !first.log.overflowed;
    NavalCopyReport { raw_ok, matched, copy_ticks: ticks, raw_bytes: first.log.raw_bytes }
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
    // Wing reverse census: replay wing walk collecting (pointer,row), then
    // visit each through the naval source path (GROUP_HANDLE reads).
    let mut wing_rows: Vec<cair_wing::Row> = Vec::new();
    loop {
        let mut collected: Vec<(usize, cair_wing::Row)> = Vec::new();
        let outcome = cair_wing::walk_with(
            &recorded,
            base,
            job.game_state,
            &mut wing_rows,
            |pointer, _, row| {
                collected.push((pointer, *row));
            },
        );
        match outcome {
            Outcome::Truncated { .. } => {
                wing_rows.resize(wing_rows.len() * 2 + 16, cair_wing::Row::default());
                continue;
            }
            _ => {
                for (pointer, row) in &collected {
                    captured.visit_wing(&recorded, base, *pointer, row, false);
                }
                captured.finish_wings(outcome, false);
                break;
            }
        }
    }
    decode_ticks[0] += qpc().saturating_sub(started).max(0) as u64;
    captured.raw_mode = false;
}

#[cfg(test)]
mod tests {
    use super::*;
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
                assert!(replay::cadence::is_refresh(table, hour), "{table:?} at {hour}");
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
        let memory = Recorder { inner, sizes: std::cell::RefCell::new(BTreeSet::new()) };
        let ctx = NavalCopyContext { base: kit::BASE, game_state: kit::GAME_STATE };
        let mut first = NavalJob::new();
        let report = super::copy_naval_refresh(&memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        let sizes = memory.sizes.borrow().clone();
        for required in [4, 8, 16] {
            assert!(sizes.contains(&required), "missing size {required} in {sizes:?}");
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
        use super::naval_test_kit as kit;
        use super::Capacities;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        for rows in [[0; 17], [1; 17], [2; 17], [64; 17]] {
            let capacity = Capacities { rows, theatre_names: 1024, sunk_names: 1024, orders_sources: 64, wing_sources: 64 };
            let direct = kit::direct_with(&memory, &capacity);
            let (split, report, _) = kit::split_with(&memory, &capacity);
            assert!(report.raw_ok, "rows={rows:?}");
            assert_eq!(report.matched, [true; 9], "rows={rows:?}");
            kit::assert_matches_direct(&split, &direct);
        }
        // Exact-fit arm pins Rows on land/groups with both members present.
        let capacity = Capacities { rows: [2; 17], theatre_names: 1024, sunk_names: 1024, orders_sources: 64, wing_sources: 64 };
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
            (kit::GAME_STATE + engine::cgame_state::COUNTRIES, [false, false, false, false, false, false, true, true, true]),
            (kit::THEATRE, [false, false, false, false, false, false, true, true, true]),
            (kit::GROUP, [false, false, false, false, false, false, true, true, true]),
        ] {
            let bytes = kit::fixture();
            let memory = Fault {
                memory: SliceMemory { base: 0, bytes: &bytes },
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
        let ctx = NavalCopyContext { base: kit::BASE, game_state: kit::GAME_STATE };
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
        let ctx = NavalCopyContext { base: kit::BASE, game_state: kit::GAME_STATE };
        let mut tiny = NavalJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
        };
        let report = super::copy_naval_refresh(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
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
    use engine::{memory::SliceMemory, walk::Outcome, Memory};

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
        use engine::{carmy, ccountry, corder_instance, corders_group, corders_group_member, ctheater_group, ctheatre, cunit, creference_registry as registry, reference_bucket as bucket};
        let mut bytes = vec![0; 0x3500000];
        header(&mut bytes, GAME_STATE + engine::cgame_state::COUNTRIES, CTAB, 3);
        p64(&mut bytes, CTAB, 0);
        p64(&mut bytes, CTAB + 8, C0 as u64);
        p64(&mut bytes, CTAB + 16, C0 as u64);
        header(&mut bytes, GAME_STATE + engine::cgame_state::COUNTRY_LINKS, LTAB, 3);
        p32(&mut bytes, LTAB, 0);
        p32(&mut bytes, LTAB + 4, 1);
        p32(&mut bytes, LTAB + 8, 1);
        p64(&mut bytes, C0, ccountry::VTABLE_RVA as u64);
        p32(&mut bytes, C0 + ccountry::ID, 1);
        // Land theatres: one theatre, one group named "AB", two members.
        header(&mut bytes, C0 + ccountry::THEATRES, 0x36000, 1);
        p64(&mut bytes, 0x36000, THEATRE as u64);
        header(&mut bytes, C0 + ccountry::AIR_THEATRES, 0x36100, 0);
        p64(&mut bytes, THEATRE, ctheatre::VTABLE_RVA as u64);
        p64(&mut bytes, THEATRE + ctheatre::HANDLE, (1u64 << 32) | ctheatre::REFERENCE_TYPE as u64);
        p32(&mut bytes, THEATRE + ctheatre::OWNER, 1);
        p32(&mut bytes, THEATRE + ctheatre::VOLUNTEERS_THEATRE, 0);
        header(&mut bytes, THEATRE + ctheatre::THEATER_GROUP_DATA, 0x36200, 1);
        p64(&mut bytes, 0x36200, GROUP as u64);
        p64(&mut bytes, GROUP, ctheater_group::VTABLE_RVA as u64);
        p64(&mut bytes, GROUP + ctheater_group::HANDLE, (1u64 << 32) | ctheater_group::REFERENCE_TYPE as u64);
        p64(&mut bytes, GROUP + ctheater_group::THEATRE, THEATRE as u64);
        // Inline name "AB" (len 2, capacity 15, terminator).
        bytes[GROUP + ctheater_group::NAME..GROUP + ctheater_group::NAME + 2].copy_from_slice(b"AB");
        p64(&mut bytes, GROUP + ctheater_group::NAME + 16, 2);
        p64(&mut bytes, GROUP + ctheater_group::NAME + 24, 15);
        bytes[GROUP + ctheater_group::NAME + 2] = 0;
        p32(&mut bytes, GROUP + ctheater_group::PRIORITY, 5);
        header(&mut bytes, GROUP + ctheater_group::ORDERS_GROUP_DATA, MEMBERS, 2);
        p64(&mut bytes, MEMBERS, MEMBER1 as u64);
        p64(&mut bytes, MEMBERS + 8, MEMBER2 as u64);
        for (member, id) in [(MEMBER1, 1u64), (MEMBER2, 2u64)] {
            p64(&mut bytes, member, corders_group::VTABLE_RVA as u64);
            p64(&mut bytes, member + corders_group::HANDLE, (id << 32) | corders_group::REFERENCE_TYPE as u64);
            p64(&mut bytes, member + corders_group::THEATER_GROUP, GROUP as u64);
            p64(&mut bytes, member + corders_group::THEATRE, THEATRE as u64);
        }
        // OOB registry with one group (same MEMBER1 group for simplicity,
        // plus dedicated OOB group with member + child).
        const HANDLE: u64 = (u32::MAX as u64) << 32 | corders_group::REFERENCE_TYPE as u64;
        p64(&mut bytes, BASE + registry::LOW_ROOTS_RVA + corders_group::REFERENCE_TYPE as usize * 8, OOB_TABLE as u64);
        bytes[OOB_TABLE..OOB_TABLE + 32].fill(0);
        p64(&mut bytes, OOB_TABLE + registry::ENTRIES, OOB_BUCKETS as u64);
        p32(&mut bytes, OOB_TABLE + registry::COUNT, 1);
        p32(&mut bytes, OOB_TABLE + registry::MASK, 7);
        bytes[OOB_TABLE + registry::OVERFLOW_DISTANCE] = 5;
        bytes[OOB_BUCKETS..OOB_BUCKETS + 14 * bucket::STRIDE].fill(0);
        bytes[OOB_BUCKETS + bucket::DISTANCE] = 1;
        p64(&mut bytes, OOB_BUCKETS + bucket::TYPE, HANDLE);
        p64(&mut bytes, OOB_BUCKETS + bucket::OBJECT, OOB_GROUP as u64);
        bytes[OOB_BUCKETS + 13 * bucket::STRIDE + bucket::DISTANCE] = bucket::TERMINAL_DISTANCE;
        bytes[OOB_GROUP..OOB_GROUP + 0x240].fill(0);
        p64(&mut bytes, OOB_GROUP, (BASE + corders_group::VTABLE_RVA) as u64);
        p64(&mut bytes, OOB_GROUP + corders_group::HANDLE, HANDLE);
        p32(&mut bytes, OOB_GROUP + corders_group::COUNTRY, 1);
        bytes[OOB_GROUP + corders_group::IS_ARMY_GROUP] = 0;
        bytes[OOB_GROUP + corders_group::MOTORIZATION_LEVEL] = 0;
        p64(&mut bytes, OOB_GROUP + corders_group::THEATRE, 0);
        header(&mut bytes, OOB_GROUP + corders_group::MEMBER_DATA, OOB_MEMBERS, 1);
        p64(&mut bytes, OOB_MEMBERS, OOB_MEMBER as u64);
        // Member host (division) with backlink.
        p64(&mut bytes, OOB_UNIT, carmy::VTABLE_RVA as u64);
        p64(&mut bytes, OOB_UNIT + carmy::SECONDARY_BASE, carmy::SECONDARY_VTABLE_RVA as u64);
        p64(&mut bytes, OOB_UNIT + cunit::HANDLE, 0x4500_0001u64);
        let member_host = OOB_MEMBER;
        let unit_base = member_host - corders_group_member::HOST_OFFSET;
        assert_eq!(unit_base, OOB_UNIT);
        p64(&mut bytes, member_host + corders_group_member::GROUP, OOB_GROUP as u64);
        header(&mut bytes, OOB_GROUP + engine::carmy_group::MEMBER_DATA, 0x66000, 0);
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
        }
    }

    pub(crate) fn direct(memory: &impl Memory) -> Split {
        collect(memory, &Capacities::default())
    }

    pub(crate) fn split(memory: &impl Memory) -> (Split, NavalCopyReport, NavalJob) {
        let ctx = NavalCopyContext { base: BASE, game_state: GAME_STATE };
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
            },
            report,
            first,
        )
    }

    pub(crate) fn direct_with(memory: &impl Memory, capacity: &Capacities) -> Split {
        collect(memory, capacity)
    }

    pub(crate) fn split_with(memory: &impl Memory, capacity: &Capacities) -> (Split, NavalCopyReport, NavalJob) {
        let ctx = NavalCopyContext { base: BASE, game_state: GAME_STATE };
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
            },
            report,
            first,
        )
    }

    pub(crate) fn direct_truncated(memory: &impl Memory) -> Split {
        let capacity = Capacities { rows: [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1], theatre_names: 8, sunk_names: 8, orders_sources: 1, wing_sources: 1 };
        collect(memory, &capacity)
    }

    pub(crate) fn split_truncated(memory: &impl Memory) -> (Split, NavalCopyReport, NavalJob) {
        let ctx = NavalCopyContext { base: BASE, game_state: GAME_STATE };
        let mut first = NavalJob::new();
        let mut second = NavalJob::new();
        let report = super::copy_naval_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let capacity = Capacities { rows: [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1], theatre_names: 8, sunk_names: 8, orders_sources: 1, wing_sources: 1 };
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
            },
            report,
            first,
        )
    }

    pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
        assert_eq!(split.land_outcome, direct.land_outcome, "land outcome");
        assert_eq!(split.land, direct.land, "land rows");
        assert_eq!(split.groups_outcome, direct.groups_outcome, "groups outcome");
        assert_eq!(split.groups, direct.groups, "groups rows");
        assert_eq!(split.members_outcome, direct.members_outcome, "members outcome");
        assert_eq!(split.members, direct.members, "members rows");
        assert_eq!(split.oob_groups_outcome, direct.oob_groups_outcome, "oob groups outcome");
        assert_eq!(split.oob_groups, direct.oob_groups, "oob groups rows");
        assert_eq!(split.oob_members_outcome, direct.oob_members_outcome, "oob members outcome");
        assert_eq!(split.oob_members, direct.oob_members, "oob members rows");
    }

    /// Decode-worker assertion against pooled buffers (same module, so
    /// private rows are visible).
    pub(crate) fn assert_captured_matches_direct(captured: &super::Captured, direct: &Split) {
        assert_eq!(captured.land.outcome, direct.land_outcome, "land outcome");
        assert_eq!(&captured.land.rows, &direct.land, "land rows");
        assert_eq!(captured.groups.outcome, direct.oob_groups_outcome, "oob groups outcome");
        assert_eq!(&captured.groups.rows, &direct.oob_groups, "oob groups rows");
        assert_eq!(captured.group_members.outcome, direct.oob_members_outcome, "oob members outcome");
        assert_eq!(&captured.group_members.rows, &direct.oob_members, "oob members rows");
    }
}
