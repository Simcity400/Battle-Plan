//! Pooled naval/history acquisition; conversion and membership joins run off-thread.

use crate::{
    sampling::{full_match, qpc},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, comparison, comparison_rows},
};
use engine::{
    Memory, cair_group, cair_theatre, cair_wing, cnaval_combat_results, cnaval_region_dominance,
    corders_group, cprovince_railway_info, crailway_manager, csunk_convoy_info, csunk_ship_info,
    ctheater_group, ctheatre, rail_neighbor, walk::Outcome,
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

        let begin = qpc();
        self.capture_theatres(memory, base, game_state, needed, &mut matched);
        ticks[4] = elapsed(begin);
        ticks[6] = self.source_ticks[0];
        ticks[9] = self.source_ticks[1];
        ticks[14] = self.source_ticks[2];
        self.finish_order_of_battle(needed, &mut matched);

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
}
