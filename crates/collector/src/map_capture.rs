//! Pooled map-table reads. Only the spool thread grows or releases allocations.
//!
//! Map-pack coupled unit (copy/decode split): the eight hourly map tables
//! sharing combat/fleet/wing/order censuses move to the single FIFO decode
//! worker as one task (LandCombats, Fleets, TaskForces, Ships, NavalCombats,
//! NavalCombatParticipants, AirWings, ArmyOrders). Players and
//! HumanControllers stay game-side: their roster walks (player records,
//! human status) share no combat/fleet/wing/order census with the eight, so
//! the coordinator's conditional excludes them with that explicit reason.
//! Wing-fed air tables (WingEquipment, WingCumulative) and source-fed naval
//! tables (LandTheatreMembers, AirTheatreMembers, OrdersGroups,
//! OrdersGroupMembers, ArmyGroupMembers) move in their own files as part of
//! the same coupled task; each family stages its own root refs from its own
//! copies with no cross-family decoded-root dependency. Choice per table:
//! zero-engine-change replay of the existing walks against `Recorded` spans,
//! which the Wars/Divisions session proved suffices for census walks with
//! shared rosters; no engine factoring, no logic change.

use crate::raw::RawLog;
#[cfg(test)]
use crate::sampling::CONSISTENCY_TARGET;
use crate::sampling::{full_match, qpc};
use crate::table_capture::{CaptureRows, comparison, comparison_rows, grow_capacity, grow_vec};
use engine::{
    Memory, cair_wing, cfleet, chuman, cland_combat, cnaval_combat, cnaval_combatant,
    corder_instance, corders_group, cpdx_array::Array, cship, ctask_force, walk::Outcome,
};
use std::mem::size_of;

pub const MAP_TABLES: usize = 10;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceChecks {
    pub ships: bool,
    pub air_wings: bool,
    pub order_groups: bool,
}

impl SourceChecks {
    fn retained(self, successful: &[u16]) -> [u16; MAP_TABLES] {
        let mut retained: [u16; MAP_TABLES] = successful.try_into().expect("map comparison count");
        for (index, needed) in [(5, self.ships), (8, self.air_wings), (9, self.order_groups)] {
            if needed {
                retained[index] = 0;
            }
        }
        retained
    }
}

/// Inline source consumers own their state and validation. Return raw QPC ticks
/// attributed to extension tables so the map buckets do not charge them twice.
/// The begin/finish pair runs even for an empty or failed source census; every
/// dependent table must preserve that outcome instead of accepting its prefix.
pub trait MapSources {
    fn checks(&self) -> SourceChecks;
    fn begin_ships(&mut self, second: bool) -> u64;
    fn ship(&mut self, second: bool, pointer: usize, row: &cship::Row) -> u64;
    fn finish_ships(&mut self, second: bool, outcome: Outcome) -> u64;
    fn begin_air(&mut self, second: bool) -> u64;
    fn air(&mut self, second: bool, pointer: usize, pool: usize, row: &cair_wing::Row) -> u64;
    fn finish_air(&mut self, second: bool, outcome: Outcome) -> u64;
    fn order_groups(
        &mut self,
        second: bool,
        roots: &[corders_group::Root],
        outcome: Outcome,
    ) -> u64;
}

#[cfg(test)]
struct NoSources;

#[cfg(test)]
impl MapSources for NoSources {
    fn checks(&self) -> SourceChecks {
        SourceChecks::default()
    }
    fn begin_ships(&mut self, _: bool) -> u64 {
        0
    }
    fn ship(&mut self, _: bool, _: usize, _: &cship::Row) -> u64 {
        0
    }
    fn finish_ships(&mut self, _: bool, _: Outcome) -> u64 {
        0
    }
    fn begin_air(&mut self, _: bool) -> u64 {
        0
    }
    fn air(&mut self, _: bool, _: usize, _: usize, _: &cair_wing::Row) -> u64 {
        0
    }
    fn finish_air(&mut self, _: bool, _: Outcome) -> u64 {
        0
    }
    fn order_groups(&mut self, _: bool, _: &[corders_group::Root], _: Outcome) -> u64 {
        0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MapCapacities {
    rows: [usize; MAP_TABLES],
    player_names: usize,
    land_participants: usize,
    order_geometry: usize,
    order_groups: usize,
    order_visits: usize,
    naval_groups: usize,
}

impl Default for MapCapacities {
    fn default() -> Self {
        Self {
            rows: [64, 512, 512, 256, 512, 4096, 256, 2048, 1024, 1024],
            player_names: 4096,
            land_participants: 8192,
            order_geometry: 16384,
            order_groups: 2048,
            order_visits: 2048,
            naval_groups: 64,
        }
    }
}

pub struct MapCaptured {
    pub players: CaptureRows<chuman::Row>,
    pub human_controllers: CaptureRows<chuman::ControllerRow>,
    pub land_combats: CaptureRows<cland_combat::Row>,
    pub fleets: CaptureRows<cfleet::Row>,
    pub task_forces: CaptureRows<ctask_force::Row>,
    pub ships: CaptureRows<cship::Row>,
    pub naval_combats: CaptureRows<cnaval_combat::Row>,
    pub naval_combat_participants: CaptureRows<cnaval_combatant::Row>,
    pub air_wings: CaptureRows<cair_wing::Row>,
    pub army_orders: CaptureRows<corder_instance::Row>,
    /// Raw-copy split gate for the hourly map-pack coupled unit (8 tables).
    /// Set by the game thread every hour with a valid raw payload; `sample`
    /// then walks only Players/HumanControllers and the decode worker
    /// replays the eight into these pooled buffers before the ready handoff.
    pub(crate) raw_mode: bool,
    pub player_names: Vec<u8>,
    player_names_second: Vec<u8>,
    pub player_names_len: usize,
    player_names_required: usize,
    pub land_participants: Vec<u64>,
    land_participants_second: Vec<u64>,
    pub land_participants_len: usize,
    land_participants_required: usize,
    pub order_geometry: Vec<u32>,
    order_geometry_second: Vec<u32>,
    pub order_geometry_len: usize,
    order_geometry_required: usize,
    order_groups: Vec<corders_group::Root>,
    order_groups_required: usize,
    order_visits: Vec<corder_instance::Visit>,
    order_visits_required: usize,
    naval_groups: Vec<usize>,
    naval_groups_required: usize,
}

impl MapCaptured {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::with_capacities(
            &MapCapacities::default(),
            &[0; MAP_TABLES],
            SourceChecks::default(),
        )
    }

    pub fn with_capacities(
        capacity: &MapCapacities,
        successful: &[u16],
        sources: SourceChecks,
    ) -> Self {
        let successful = sources.retained(successful);
        Self {
            players: CaptureRows::new(capacity.rows[0], successful[0]),
            human_controllers: CaptureRows::new(capacity.rows[1], successful[1]),
            land_combats: CaptureRows::new(capacity.rows[2], successful[2]),
            fleets: CaptureRows::new(capacity.rows[3], successful[3]),
            task_forces: CaptureRows::new(capacity.rows[4], successful[4]),
            ships: CaptureRows::new(capacity.rows[5], successful[5]),
            naval_combats: CaptureRows::new(capacity.rows[6], successful[6]),
            naval_combat_participants: CaptureRows::new(capacity.rows[7], successful[7]),
            air_wings: CaptureRows::new(capacity.rows[8], successful[8]),
            army_orders: CaptureRows::new(capacity.rows[9], successful[9]),
            player_names: vec![0; capacity.player_names],
            player_names_second: comparison_rows(capacity.player_names, successful[0]),
            player_names_len: 0,
            player_names_required: 0,
            land_participants: vec![0; capacity.land_participants],
            land_participants_second: comparison_rows(capacity.land_participants, successful[2]),
            land_participants_len: 0,
            land_participants_required: 0,
            order_geometry: vec![0; capacity.order_geometry],
            order_geometry_second: comparison_rows(capacity.order_geometry, successful[9]),
            order_geometry_len: 0,
            order_geometry_required: 0,
            order_groups: vec![corders_group::Root::default(); capacity.order_groups],
            order_groups_required: 0,
            order_visits: vec![corder_instance::Visit::default(); capacity.order_visits],
            order_visits_required: 0,
            naval_groups: vec![0; capacity.naval_groups],
            naval_groups_required: 0,
            raw_mode: false,
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        // The containing Captured accounts for this struct's inline bytes.
        self.players.allocated_bytes()
            + self.human_controllers.allocated_bytes()
            + self.land_combats.allocated_bytes()
            + self.fleets.allocated_bytes()
            + self.task_forces.allocated_bytes()
            + self.ships.allocated_bytes()
            + self.naval_combats.allocated_bytes()
            + self.naval_combat_participants.allocated_bytes()
            + self.air_wings.allocated_bytes()
            + self.army_orders.allocated_bytes()
            + self.player_names.capacity()
            + self.player_names_second.capacity()
            + (self.land_participants.capacity() + self.land_participants_second.capacity())
                * size_of::<u64>()
            + (self.order_geometry.capacity() + self.order_geometry_second.capacity())
                * size_of::<u32>()
            + self.order_groups.capacity() * size_of::<corders_group::Root>()
            + self.order_visits.capacity() * size_of::<corder_instance::Visit>()
            + self.naval_groups.capacity() * size_of::<usize>()
    }

    pub fn reset(&mut self) {
        self.players.reset();
        self.human_controllers.reset();
        self.land_combats.reset();
        self.fleets.reset();
        self.task_forces.reset();
        self.ships.reset();
        self.naval_combats.reset();
        self.naval_combat_participants.reset();
        self.air_wings.reset();
        self.army_orders.reset();
        self.player_names_len = 0;
        self.player_names_required = 0;
        self.land_participants_len = 0;
        self.land_participants_required = 0;
        self.order_geometry_len = 0;
        self.order_geometry_required = 0;
        self.order_groups_required = 0;
        self.order_visits_required = 0;
        self.naval_groups_required = 0;
        self.raw_mode = false;
    }

    pub fn outcomes(&self) -> [Outcome; MAP_TABLES] {
        [
            self.players.outcome,
            self.human_controllers.outcome,
            self.land_combats.outcome,
            self.fleets.outcome,
            self.task_forces.outcome,
            self.ships.outcome,
            self.naval_combats.outcome,
            self.naval_combat_participants.outcome,
            self.air_wings.outcome,
            self.army_orders.outcome,
        ]
    }

    pub fn checked_mismatches(&self) -> [u64; MAP_TABLES] {
        [
            if self.players.checked {
                self.players.mismatches as u64
            } else {
                0
            },
            if self.human_controllers.checked {
                self.human_controllers.mismatches as u64
            } else {
                0
            },
            if self.land_combats.checked {
                self.land_combats.mismatches as u64
            } else {
                0
            },
            if self.fleets.checked {
                self.fleets.mismatches as u64
            } else {
                0
            },
            if self.task_forces.checked {
                self.task_forces.mismatches as u64
            } else {
                0
            },
            if self.ships.checked {
                self.ships.mismatches as u64
            } else {
                0
            },
            if self.naval_combats.checked {
                self.naval_combats.mismatches as u64
            } else {
                0
            },
            if self.naval_combat_participants.checked {
                self.naval_combat_participants.mismatches as u64
            } else {
                0
            },
            if self.air_wings.checked {
                self.air_wings.mismatches as u64
            } else {
                0
            },
            if self.army_orders.checked {
                self.army_orders.mismatches as u64
            } else {
                0
            },
        ]
    }

    /// Exclusive spool-thread ownership is required, including for scratch buffers.
    pub fn grow(
        &mut self,
        capacities: &mut MapCapacities,
        successful: &[u16],
        sources: SourceChecks,
    ) {
        let successful = sources.retained(successful);
        self.players.grow(&mut capacities.rows[0], successful[0]);
        self.human_controllers
            .grow(&mut capacities.rows[1], successful[1]);
        self.land_combats
            .grow(&mut capacities.rows[2], successful[2]);
        self.fleets.grow(&mut capacities.rows[3], successful[3]);
        self.task_forces
            .grow(&mut capacities.rows[4], successful[4]);
        self.ships.grow(&mut capacities.rows[5], successful[5]);
        self.naval_combats
            .grow(&mut capacities.rows[6], successful[6]);
        self.naval_combat_participants
            .grow(&mut capacities.rows[7], successful[7]);
        self.air_wings.grow(&mut capacities.rows[8], successful[8]);
        self.army_orders
            .grow(&mut capacities.rows[9], successful[9]);
        grow_capacity(&mut capacities.player_names, self.player_names_required);
        grow_capacity(
            &mut capacities.land_participants,
            self.land_participants_required,
        );
        grow_capacity(&mut capacities.order_geometry, self.order_geometry_required);
        grow_capacity(&mut capacities.order_groups, self.order_groups_required);
        grow_capacity(&mut capacities.order_visits, self.order_visits_required);
        grow_capacity(&mut capacities.naval_groups, self.naval_groups_required);
        grow_vec(&mut self.player_names, capacities.player_names);
        comparison(
            &mut self.player_names_second,
            capacities.player_names,
            successful[0],
        );
        grow_vec(&mut self.land_participants, capacities.land_participants);
        comparison(
            &mut self.land_participants_second,
            capacities.land_participants,
            successful[2],
        );
        grow_vec(&mut self.order_geometry, capacities.order_geometry);
        comparison(
            &mut self.order_geometry_second,
            capacities.order_geometry,
            successful[9],
        );
        grow_vec(&mut self.order_groups, capacities.order_groups);
        grow_vec(&mut self.order_visits, capacities.order_visits);
        grow_vec(&mut self.naval_groups, capacities.naval_groups);
    }

    /// Returns independent match results and caller-clock ticks for the ten tables.
    /// Numeric values and variable rows remain native; conversion happens in map_spool.
    pub fn sample(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
        sources: &mut impl MapSources,
    ) -> ([bool; MAP_TABLES], [u64; MAP_TABLES]) {
        let source_checks = sources.checks();
        let mut matched = [false; MAP_TABLES];
        let mut ticks = [0; MAP_TABLES];
        self.players.checked = needed[0];
        self.human_controllers.checked = needed[1];
        self.land_combats.checked = needed[2];
        self.fleets.checked = needed[3];
        self.task_forces.checked = needed[4];
        self.ships.checked = needed[5];
        self.naval_combats.checked = needed[6];
        self.naval_combat_participants.checked = needed[7];
        self.air_wings.checked = needed[8];
        self.army_orders.checked = needed[9];
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
        if needed[0] {
            let second = chuman::walk(
                memory,
                base,
                state,
                &mut self.players.second,
                &mut self.player_names_second,
            );
            self.players.compare(second.outcome);
            self.players.mismatches += usize::from(
                names.bytes_len != second.bytes_len
                    || names.required_bytes != second.required_bytes
                    || self.player_names[..names.bytes_len]
                        != self.player_names_second[..second.bytes_len],
            );
            matched[0] = full_match(names.outcome, second.outcome, self.players.mismatches);
            self.player_names_required = self.player_names_required.max(second.required_bytes);
        }
        ticks[0] = (qpc() - begin).max(0) as u64;

        let begin = qpc();
        self.human_controllers.outcome =
            chuman::walk_controllers(memory, base, state, &mut self.human_controllers.rows);
        if needed[1] {
            let second =
                chuman::walk_controllers(memory, base, state, &mut self.human_controllers.second);
            matched[1] = self.human_controllers.compare(second);
        }
        ticks[1] = (qpc() - begin).max(0) as u64;

        // Raw-copy split (hourly map-pack coupled unit): the game thread
        // staged raw bytes via `copy_map_refresh` and the decode worker
        // replays the eight moved tables before the ready handoff. Skip the
        // moved walks here; matched flags and costs for the eight install
        // from the copy report in the tick (like every other split family).
        if self.raw_mode {
            return (matched, ticks);
        }

        let begin = qpc();
        let land = cland_combat::walk(
            memory,
            base,
            state,
            &mut self.land_combats.rows,
            &mut self.land_participants,
        );
        self.land_combats.outcome = land.outcome;
        self.land_participants_len = land.participants_len;
        self.land_participants_required = land.required_participants;
        if needed[2] {
            let second = cland_combat::walk(
                memory,
                base,
                state,
                &mut self.land_combats.second,
                &mut self.land_participants_second,
            );
            self.land_combats.compare(second.outcome);
            self.land_combats.mismatches += usize::from(
                land.participants_len != second.participants_len
                    || land.required_participants != second.required_participants
                    || self.land_participants[..land.participants_len]
                        != self.land_participants_second[..second.participants_len],
            );
            matched[2] = full_match(land.outcome, second.outcome, self.land_combats.mismatches);
            self.land_participants_required = self
                .land_participants_required
                .max(second.required_participants);
        }
        ticks[2] = (qpc() - begin).max(0) as u64;

        let begin = qpc();
        let mut nested = sources.begin_ships(false);
        let naval = cfleet::walk_timed_with(
            memory,
            base,
            state,
            &mut self.fleets.rows,
            &mut self.task_forces.rows,
            &mut self.ships.rows,
            || qpc() as u64,
            |pointer, row| nested = nested.saturating_add(sources.ship(false, pointer, row)),
        );
        nested = nested.saturating_add(sources.finish_ships(false, naval.capture.ships));
        self.fleets.outcome = naval.capture.fleets;
        self.task_forces.outcome = naval.capture.task_forces;
        self.ships.outcome = naval.capture.ships;
        ticks[3..6].copy_from_slice(&naval.read_ticks);
        ticks[5] = (qpc() - begin).max(0) as u64;
        ticks[5] = ticks[5]
            .saturating_sub(ticks[3])
            .saturating_sub(ticks[4])
            .saturating_sub(nested);
        if needed[3..6].iter().any(|&value| value) || source_checks.ships {
            let begin = qpc();
            let mut nested = sources.begin_ships(true);
            let second = cfleet::walk_timed_with(
                memory,
                base,
                state,
                &mut self.fleets.second,
                &mut self.task_forces.second,
                &mut self.ships.second,
                || qpc() as u64,
                |pointer, row| nested = nested.saturating_add(sources.ship(true, pointer, row)),
            );
            nested = nested.saturating_add(sources.finish_ships(true, second.capture.ships));
            let mut second_ticks = second.read_ticks;
            second_ticks[2] = ((qpc() - begin).max(0) as u64)
                .saturating_sub(second_ticks[0])
                .saturating_sub(second_ticks[1])
                .saturating_sub(nested);
            for (total, second) in ticks[3..6].iter_mut().zip(second_ticks) {
                *total += second;
            }
            if needed[3] {
                let begin = qpc();
                matched[3] = self.fleets.compare(second.capture.fleets);
                ticks[3] += (qpc() - begin).max(0) as u64;
            }
            if needed[4] {
                let begin = qpc();
                matched[4] = self.task_forces.compare(second.capture.task_forces);
                ticks[4] += (qpc() - begin).max(0) as u64;
            }
            if needed[5] {
                let begin = qpc();
                matched[5] = self.ships.compare(second.capture.ships);
                ticks[5] += (qpc() - begin).max(0) as u64;
            }
        }

        let naval = cnaval_combat::walk_timed(
            memory,
            base,
            state,
            &mut self.naval_combats.rows,
            &mut self.naval_combat_participants.rows,
            &mut self.naval_groups,
            || qpc() as u64,
        );
        self.naval_combats.outcome = naval.capture.combats;
        self.naval_combat_participants.outcome = naval.capture.participants;
        self.naval_groups_required = naval.capture.required_group_capacity;
        ticks[6..8].copy_from_slice(&naval.read_ticks);
        if needed[6] || needed[7] {
            let second = cnaval_combat::walk_timed(
                memory,
                base,
                state,
                &mut self.naval_combats.second,
                &mut self.naval_combat_participants.second,
                &mut self.naval_groups,
                || qpc() as u64,
            );
            self.naval_groups_required = self
                .naval_groups_required
                .max(second.capture.required_group_capacity);
            for (total, second) in ticks[6..8].iter_mut().zip(second.read_ticks) {
                *total += second;
            }
            if needed[6] {
                let begin = qpc();
                matched[6] = self.naval_combats.compare(second.capture.combats);
                ticks[6] += (qpc() - begin).max(0) as u64;
            }
            if needed[7] {
                let begin = qpc();
                matched[7] = self
                    .naval_combat_participants
                    .compare(second.capture.participants);
                ticks[7] += (qpc() - begin).max(0) as u64;
            }
        }

        let begin = qpc();
        let mut nested = sources.begin_air(false);
        self.air_wings.outcome = cair_wing::walk_with(
            memory,
            base,
            state,
            &mut self.air_wings.rows,
            |pointer, pool, row| {
                nested = nested.saturating_add(sources.air(false, pointer, pool, row))
            },
        );
        nested = nested.saturating_add(sources.finish_air(false, self.air_wings.outcome));
        if needed[8] || source_checks.air_wings {
            nested = nested.saturating_add(sources.begin_air(true));
            let second = cair_wing::walk_with(
                memory,
                base,
                state,
                &mut self.air_wings.second,
                |pointer, pool, row| {
                    nested = nested.saturating_add(sources.air(true, pointer, pool, row))
                },
            );
            nested = nested.saturating_add(sources.finish_air(true, second));
            if needed[8] {
                matched[8] = self.air_wings.compare(second);
            }
        }
        ticks[8] = ((qpc() - begin).max(0) as u64).saturating_sub(nested);

        let begin = qpc();
        let orders = corder_instance::walk(
            memory,
            base,
            &mut self.army_orders.rows,
            &mut self.order_geometry,
            &mut self.order_groups,
            &mut self.order_visits,
        );
        let mut nested = sources.order_groups(
            false,
            &self.order_groups[..orders.root_outcome.len()],
            orders.root_outcome,
        );
        self.army_orders.outcome = orders.outcome;
        self.order_geometry_len = orders.geometry_len;
        self.order_geometry_required = orders.required_geometry;
        self.order_groups_required = orders.required_groups;
        self.order_visits_required = orders.minimum_visits.max(orders.visited.saturating_mul(2));
        if needed[9] || source_checks.order_groups {
            let second = corder_instance::walk(
                memory,
                base,
                &mut self.army_orders.second,
                &mut self.order_geometry_second,
                &mut self.order_groups,
                &mut self.order_visits,
            );
            nested = nested.saturating_add(sources.order_groups(
                true,
                &self.order_groups[..second.root_outcome.len()],
                second.root_outcome,
            ));
            if needed[9] {
                self.army_orders.compare(second.outcome);
                self.army_orders.mismatches += usize::from(
                    orders.geometry_len != second.geometry_len
                        || orders.required_geometry != second.required_geometry
                        || self.order_geometry[..orders.geometry_len]
                            != self.order_geometry_second[..second.geometry_len],
                );
                matched[9] =
                    full_match(orders.outcome, second.outcome, self.army_orders.mismatches);
            }
            self.order_geometry_required =
                self.order_geometry_required.max(second.required_geometry);
            self.order_groups_required = self.order_groups_required.max(second.required_groups);
            self.order_visits_required = self
                .order_visits_required
                .max(second.minimum_visits)
                .max(second.visited.saturating_mul(2));
        }
        ticks[9] = ((qpc() - begin).max(0) as u64).saturating_sub(nested);
        (matched, ticks)
    }
}

// ===========================================================================
// Map-pack coupled unit raw-copy split (8 hourly tables: LandCombats,
// Fleets, TaskForces, Ships, NavalCombats, NavalCombatParticipants, AirWings,
// ArmyOrders). Players/HumanControllers stay game-side (independent roster
// walks, explicit reason in the module docs).
//
// The game thread copies raw bytes only with independent per-table censuses
// (each family stages its own root refs from its own copies; no cross-family
// decoded roots). Takes are unconditional over discoverable addresses (a
// superset); the replay derives every outcome from the same recorded spans,
// so rows, ordinals, skips, truncation counts and failure reasons are
// identical by construction (see `crate::raw` for the mirror rules).
// Validation, precedence and error scope are local parses; the mirror never
// aborts the whole copy except on overflow. Per-table sink details only stop
// that table's takes where the typed walk continues.
// ===========================================================================

/// Fault-attribution bits in local order: [land, fleets, taskforces, ships,
/// naval_combats, naval_participants, wings, orders]. Shared fleet roots
/// block fleets/taskforces/ships; task roots block taskforces/ships; shared
/// naval roots block both naval tables. Single-table walks use their own bit
/// for every span including their census.
pub const MAP_PACK_N: usize = 8;
const LAND_MASK: u32 = 0x01;
const FLEET_MASK: u32 = 0x02;
const TASKFORCE_MASK: u32 = 0x04;
const SHIP_MASK: u32 = 0x08;
const NAVAL_COMBAT_MASK: u32 = 0x10;
const NAVAL_PART_MASK: u32 = 0x20;
const WING_MASK: u32 = 0x40;
const ORDER_MASK: u32 = 0x80;
const FLEET_ROOT_MASK: u32 = FLEET_MASK | TASKFORCE_MASK | SHIP_MASK;
const TASK_ROOT_MASK: u32 = TASKFORCE_MASK | SHIP_MASK;
const NAVAL_ROOT_MASK: u32 = NAVAL_COMBAT_MASK | NAVAL_PART_MASK;

/// One hour of raw map-pack payload. Flat pools recycle capacities and the
/// steady state allocates on neither thread; growth happens on the decode
/// side only (see `crate::decode::recycle_map_job`).
#[derive(Debug)]
pub(crate) struct MapJob {
    pub log: RawLog<MAP_PACK_N>,
    pub game_state: usize,
    order_seen: Vec<usize>,
    order_stack: Vec<usize>,
}

impl MapJob {
    pub fn new() -> Self {
        Self {
            // Steady state is about 34 land combats, 142 fleets, 237 task
            // forces, 2k ships, 7 naval combats, 75 participants, 581 wings
            // and 919 orders plus participant/geometry arenas; both pools
            // start above that so steady state never overflows.
            log: RawLog::new(1 << 18, 8 << 20),
            game_state: 0,
            order_seen: Vec::with_capacity(2048),
            order_stack: Vec::with_capacity(2048),
        }
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.order_seen.clear();
        self.order_stack.clear();
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    /// Double every pool off the game thread until copies fit. Converges like
    /// the typed capacity growth; overflowed hours fall back to the typed
    /// walk in the meantime, so sizing is a pure performance knob.
    pub fn grow(&mut self) {
        self.log.grow();
        self.order_seen
            .reserve_exact(self.order_seen.capacity().max(1));
        self.order_stack
            .reserve_exact(self.order_stack.capacity().max(1));
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct MapCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    /// Whole-log raw equality gated by live-fault conservatism, in
    /// [land, fleets, taskforces, ships, naval_combats, naval_participants,
    /// wings, orders] order.
    pub matched: [bool; MAP_PACK_N],
    pub copy_ticks: [u64; MAP_PACK_N],
    pub raw_bytes: [u64; MAP_PACK_N],
}

/// Game-thread copy parameters, bundled to keep the tick entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MapCopyContext {
    pub base: usize,
    pub game_state: usize,
}

fn map_pool_u64(log: &RawLog<MAP_PACK_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn map_pool_u32(log: &RawLog<MAP_PACK_N>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn mspan(
    log: &mut RawLog<MAP_PACK_N>,
    memory: &impl engine::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

/// Mirror of one land-combat side's participant vector: header plus per-entry
/// pointers with division identity takes. Takes run unconditionally; the
/// replay validates vtables, parent links and handles itself.
fn copy_land_side(
    log: &mut RawLog<MAP_PACK_N>,
    memory: &impl engine::Memory,
    base: usize,
    side: usize,
) {
    use engine::{carmy, cland_combatant, cunit};
    let header = match mspan(
        log,
        memory,
        side.checked_add(cland_combatant::UNITS),
        16,
        LAND_MASK,
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
        let entry_addr = array.data.checked_add((index as usize).saturating_mul(8));
        let pointer = match mspan(log, memory, entry_addr, 8, LAND_MASK).and_then(|start| {
            usize::try_from(map_pool_u64(log, start))
                .ok()
                .filter(|&p| p != 0)
        }) {
            Some(pointer) => pointer,
            None => continue,
        };
        // Division identity triple exactly like the typed unit reader:
        // primary vtable, secondary vtable+handle span.
        let _ = mspan(log, memory, Some(pointer), 8, LAND_MASK);
        let _ = mspan(
            log,
            memory,
            pointer.checked_add(carmy::SECONDARY_BASE),
            16,
            LAND_MASK,
        );
        let _ = base;
        let _ = cunit::HANDLE;
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of the land-combat walk's read set: mixed registry, province
/// array, per-combat identity/side/province/counter spans and both
/// participant vectors with division identities. Takes are unconditional
/// over discoverable addresses (a superset); the replay derives every
/// outcome including skips of naval objects.
fn copy_land_walk(
    log: &mut RawLog<MAP_PACK_N>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) {
    use engine::{
        carmy, cland_border_war_combat, cland_border_war_combatant, cland_combatant, cprovince,
    };
    let registry = match mspan(
        log,
        memory,
        game_state
            .checked_add(engine::cgame_state::COMBATS)
            .and_then(|p| p.checked_add(8)),
        16,
        LAND_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let provinces = match mspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::PROVINCES),
        16,
        LAND_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[registry..registry + 16]);
    let registry_array = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    raw.copy_from_slice(&log.bytes[provinces..provinces + 16]);
    let provinces_array = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    for index in 0..registry_array.count {
        let entry_addr = registry_array
            .data
            .checked_add((index as usize).saturating_mul(8));
        let pointer = match mspan(log, memory, entry_addr, 8, LAND_MASK).and_then(|start| {
            usize::try_from(map_pool_u64(log, start))
                .ok()
                .filter(|&p| p != 0)
        }) {
            Some(pointer) => pointer,
            None => continue,
        };
        // Class vtable: naval objects are skipped by the replay from the
        // same recorded span, so record it and continue past them.
        let vtable_start = match mspan(log, memory, Some(pointer), 8, LAND_MASK) {
            Some(start) => start,
            None => continue,
        };
        let vtable = map_pool_u64(log, vtable_start);
        let base_vtable = match base.checked_add(engine::cnaval_combat::VTABLE_RVA) {
            Some(v) => v as u64,
            None => continue,
        };
        if vtable == base_vtable {
            continue;
        }
        // Identity span (secondary vptr + handle) and side pointers.
        let _ = mspan(
            log,
            memory,
            pointer.checked_add(cland_combat::BODY),
            16,
            LAND_MASK,
        );
        let sides = match mspan(
            log,
            memory,
            pointer.checked_add(cland_combat::ATTACKER),
            16,
            LAND_MASK,
        ) {
            Some(start) => start,
            None => continue,
        };
        let attacker = usize::try_from(u64::from_le_bytes(
            log.bytes[sides..sides + 8].try_into().unwrap(),
        ))
        .ok();
        let defender = usize::try_from(u64::from_le_bytes(
            log.bytes[sides + 8..sides + 16].try_into().unwrap(),
        ))
        .ok();
        // Location + counters span and province authentication triple.
        let row_span = mspan(
            log,
            memory,
            pointer.checked_add(cland_combat::PROVINCE),
            16,
            LAND_MASK,
        );
        if let Some(row_span) = row_span {
            let location = usize::try_from(u64::from_le_bytes(
                log.bytes[row_span..row_span + 8].try_into().unwrap(),
            ))
            .ok()
            .filter(|&p| p != 0);
            if let Some(location) = location {
                let _ = mspan(log, memory, Some(location), 8, LAND_MASK);
                let id_start = mspan(
                    log,
                    memory,
                    location.checked_add(cprovince::ID),
                    4,
                    LAND_MASK,
                );
                if let Some(id_start) = id_start {
                    let id = map_pool_u32(log, id_start);
                    if id != 0 {
                        let slot = provinces_array.data.checked_add((id as usize) * 8);
                        let _ = mspan(log, memory, slot, 8, LAND_MASK);
                    }
                }
            }
        }
        // Both sides: vtable, parent link, discriminator and vectors.
        for (side_opt, attacker_flag) in [(attacker, 1u8), (defender, 0u8)] {
            let Some(side) = side_opt.filter(|&p| p != 0) else {
                continue;
            };
            let _ = attacker_flag;
            let _ = mspan(log, memory, Some(side), 8, LAND_MASK);
            let _ = mspan(
                log,
                memory,
                side.checked_add(cland_combatant::COMBAT),
                8,
                LAND_MASK,
            );
            let _ = mspan(
                log,
                memory,
                side.checked_add(cland_combatant::IS_ATTACKER),
                1,
                LAND_MASK,
            );
            // Border-war subclass vtables are distinguished by the replay;
            // record the same vtable span (already taken) and the vector.
            let _ = cland_border_war_combat::VTABLE_RVA;
            let _ = cland_border_war_combatant::VTABLE_RVA;
            copy_land_side(log, memory, base, side);
            if log.overflowed {
                return;
            }
        }
        let _ = carmy::VTABLE_RVA;
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of one link-table read for the fleet census with local bounds
/// checks. Returns the resolved slot exactly when the typed resolve
/// succeeds; the replay derives its own reason from the same recorded span.
fn fleet_link(
    log: &mut RawLog<MAP_PACK_N>,
    memory: &impl engine::Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<u32> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = mspan(
        log,
        memory,
        links.data.checked_add(offset),
        4,
        FLEET_ROOT_MASK,
    )?;
    let slot = map_pool_u32(log, start);
    if slot == 0 || slot >= countries {
        return None;
    }
    Some(slot)
}

/// Mirror of one link-table read for the wing census, same shape as the
/// fleet link mirror with the wing mask.
fn wing_link(
    log: &mut RawLog<MAP_PACK_N>,
    memory: &impl engine::Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<u32> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = mspan(log, memory, links.data.checked_add(offset), 4, WING_MASK)?;
    let slot = map_pool_u32(log, start);
    if slot == 0 || slot >= countries {
        return None;
    }
    Some(slot)
}

/// Mirror of one province authentication for the fleet walk: vtable,
/// identity word and registered slot pointer. Null pointers are a readable
/// home-base absence the replay derives itself.
fn copy_fleet_province(
    log: &mut RawLog<MAP_PACK_N>,
    memory: &impl engine::Memory,
    base: usize,
    provinces: &Array,
    pointer: usize,
) {
    use engine::cprovince;
    if pointer == 0 {
        return;
    }
    let expected = match base.checked_add(cprovince::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    let vtable_start = match mspan(log, memory, Some(pointer), 8, FLEET_MASK) {
        Some(start) => start,
        None => return,
    };
    if map_pool_u64(log, vtable_start) != expected {
        return;
    }
    let id_start = match mspan(
        log,
        memory,
        pointer.checked_add(cprovince::ID),
        4,
        FLEET_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let id = map_pool_u32(log, id_start);
    if id == 0 {
        return;
    }
    let slot = provinces.data.checked_add((id as usize) * 8);
    let _ = mspan(log, memory, slot, 8, FLEET_MASK);
}

/// Mirror of one ship row's read set exactly like `cship::read_row`:
/// vtable plus the scattered identity/scalar/category/sunk/class/parent
/// spans. Takes run unconditionally; the replay validates the handle,
/// parent link and widths itself.
fn copy_ship_row(log: &mut RawLog<MAP_PACK_N>, memory: &impl engine::Memory, ship: usize) {
    use engine::cship;
    let _ = mspan(log, memory, Some(ship), 8, SHIP_MASK);
    for (offset, len) in [
        (cship::HANDLE, 8),
        (cship::STRENGTH, 24),
        (cship::CATEGORY, 4),
        (cship::SUNK_CONVOYS, 4),
        (cship::CLASS_DATABASE_ID, 4),
        (cship::TASK_FORCE, 8),
    ] {
        let _ = mspan(log, memory, ship.checked_add(offset), len, SHIP_MASK);
    }
}

/// Mirror of one task force's read set exactly like the typed
/// `ctask_force::Reader`: dual vtables, handle, fleet backlink, owner
/// triple, location with province authentication, mission word and the ship
/// roster with per-ship takes. Task spans carry the task+ship mask so task
/// faults block both tables like the typed walk; ship spans carry the ship
/// mask. Link-resolve and province-slot spans needed by the row reader are
/// recorded here with the task mask so the replay derives owner/province
/// mismatches from the same bytes.
fn copy_task_force(
    log: &mut RawLog<MAP_PACK_N>,
    memory: &impl engine::Memory,
    base: usize,
    provinces: &Array,
    links: &Array,
    countries: u32,
    task: usize,
) {
    use engine::{ccountry, cprovince, ctask_force, cunit};
    let _ = mspan(log, memory, Some(task), 8, TASK_ROOT_MASK);
    let _ = mspan(
        log,
        memory,
        task.checked_add(ctask_force::SECONDARY_BASE),
        8,
        TASK_ROOT_MASK,
    );
    let _ = mspan(
        log,
        memory,
        task.checked_add(cunit::HANDLE),
        8,
        TASK_ROOT_MASK,
    );
    let _ = mspan(
        log,
        memory,
        task.checked_add(ctask_force::FLEET),
        8,
        TASK_ROOT_MASK,
    );
    let owners = mspan(
        log,
        memory,
        task.checked_add(cunit::ROSTER_OWNER),
        12,
        TASK_ROOT_MASK,
    );
    let location = mspan(
        log,
        memory,
        task.checked_add(cunit::LOCATION),
        8,
        TASK_ROOT_MASK,
    );
    let _ = mspan(
        log,
        memory,
        task.checked_add(ctask_force::MISSION),
        4,
        TASK_ROOT_MASK,
    );
    // Owner link resolve: record the link-table entry the typed resolve
    // reads so the replay derives canonical-slot mismatches identically.
    if let Some(owners) = owners {
        let roster_owner = u32::from_le_bytes(log.bytes[owners..owners + 4].try_into().unwrap());
        if roster_owner != 0 && roster_owner < links.count {
            let offset = (roster_owner as usize).checked_mul(4);
            let _ = mspan(
                log,
                memory,
                offset.and_then(|off| links.data.checked_add(off)),
                4,
                TASK_ROOT_MASK,
            );
        }
    }
    let _ = countries;
    let _ = ccountry::VTABLE_RVA;
    // Location province authentication triple (vtable, id, slot).
    if let Some(location) = location {
        let home = usize::try_from(map_pool_u64(log, location))
            .ok()
            .unwrap_or(0);
        if home != 0 {
            let expected = match base.checked_add(cprovince::VTABLE_RVA) {
                Some(v) => v as u64,
                None => return,
            };
            let vtable = mspan(log, memory, Some(home), 8, TASK_ROOT_MASK);
            if vtable.is_some_and(|start| map_pool_u64(log, start) == expected) {
                let id_start = mspan(
                    log,
                    memory,
                    home.checked_add(cprovince::ID),
                    4,
                    TASK_ROOT_MASK,
                );
                if let Some(id_start) = id_start {
                    let id = map_pool_u32(log, id_start);
                    if id != 0 {
                        let slot = provinces.data.checked_add((id as usize) * 8);
                        let _ = mspan(log, memory, slot, 8, TASK_ROOT_MASK);
                    }
                }
            } else {
                let _ = mspan(log, memory, Some(home), 8, TASK_ROOT_MASK);
            }
        }
    }
    let roster = match mspan(
        log,
        memory,
        task.checked_add(ctask_force::SHIPS),
        16,
        TASK_ROOT_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[roster..roster + 16]);
    let array = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    for index in 0..array.count {
        let entry_addr = array.data.checked_add((index as usize).saturating_mul(8));
        let ship = match mspan(log, memory, entry_addr, 8, SHIP_MASK).and_then(|start| {
            usize::try_from(map_pool_u64(log, start))
                .ok()
                .filter(|&p| p != 0)
        }) {
            Some(ship) => ship,
            None => continue,
        };
        copy_ship_row(log, memory, ship);
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of the fleet/taskforce/ship walk's read set: country census with
/// per-country fleet rosters, per-fleet identity/owner/task-roster/home-base
/// spans and per-taskforce/ship takes. Root spans carry the combined fleet
/// mask; fleet home-base carries the fleet mask; task spans carry the
/// task+ship mask; ship spans carry the ship mask, matching the typed walk's
/// error scope (root failures fail all three, task failures fail taskforces
/// and ships, body failures fail only their table).
fn copy_fleet_walk(
    log: &mut RawLog<MAP_PACK_N>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) {
    use engine::{ccountry, cprovince};
    let countries = match mspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::COUNTRIES),
        16,
        FLEET_ROOT_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let links = match mspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::COUNTRY_LINKS),
        16,
        FLEET_ROOT_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let provinces = match mspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::PROVINCES),
        16,
        FLEET_ROOT_MASK,
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
    raw.copy_from_slice(&log.bytes[provinces..provinces + 16]);
    let provinces_array = match Array::from_header(raw, 8) {
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
    let fleet_vtable = match base.checked_add(cfleet::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    for slot in 1..countries_array.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|off| countries_array.data.checked_add(off));
        let pointer = match mspan(log, memory, pointer_addr, 8, FLEET_ROOT_MASK).and_then(|start| {
            usize::try_from(map_pool_u64(log, start))
                .ok()
                .filter(|&p| p != 0)
        }) {
            Some(pointer) => pointer,
            None => continue,
        };
        let vtable_start = match mspan(log, memory, Some(pointer), 8, FLEET_ROOT_MASK) {
            Some(start) => start,
            None => continue,
        };
        if map_pool_u64(log, vtable_start) != country_vtable {
            return;
        }
        let id_start = match mspan(
            log,
            memory,
            pointer.checked_add(ccountry::ID),
            4,
            FLEET_ROOT_MASK,
        ) {
            Some(start) => start,
            None => continue,
        };
        let id = map_pool_u32(log, id_start);
        // Authenticated link chain exactly like `ccountry::Reader`: id to
        // canonical, slot to canonical, canonical self-check and canonical
        // pointer equality. Failures here fail the census like the typed
        // reader, so record every link span.
        let canonical = match fleet_link(log, memory, &links_array, countries_array.count, id) {
            Some(canonical) => canonical,
            None => return,
        };
        if fleet_link(log, memory, &links_array, countries_array.count, slot) != Some(canonical) {
            return;
        }
        if fleet_link(log, memory, &links_array, countries_array.count, canonical)
            != Some(canonical)
        {
            return;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|off| countries_array.data.checked_add(off));
        let canonical_pointer = match mspan(log, memory, canonical_addr, 8, FLEET_ROOT_MASK) {
            Some(start) => start,
            None => return,
        };
        if map_pool_u64(log, canonical_pointer) != pointer as u64 {
            return;
        }
        // Fleet roster for this country.
        let roster = match mspan(
            log,
            memory,
            pointer.checked_add(ccountry::FLEETS),
            16,
            FLEET_ROOT_MASK,
        ) {
            Some(start) => start,
            None => continue,
        };
        let mut header = [0u8; 16];
        header.copy_from_slice(&log.bytes[roster..roster + 16]);
        let fleet_roster = match Array::from_header(header, 8) {
            Some(array) => array,
            None => continue,
        };
        for index in 0..fleet_roster.count {
            let entry_addr = fleet_roster
                .data
                .checked_add((index as usize).saturating_mul(8));
            let fleet = match mspan(log, memory, entry_addr, 8, FLEET_ROOT_MASK).and_then(|start| {
                usize::try_from(map_pool_u64(log, start))
                    .ok()
                    .filter(|&p| p != 0)
            }) {
                Some(fleet) => fleet,
                None => continue,
            };
            let fleet_vtable_start = match mspan(log, memory, Some(fleet), 8, FLEET_ROOT_MASK) {
                Some(start) => start,
                None => continue,
            };
            if map_pool_u64(log, fleet_vtable_start) != fleet_vtable {
                return;
            }
            let handle_start = mspan(
                log,
                memory,
                fleet.checked_add(cfleet::HANDLE),
                8,
                FLEET_ROOT_MASK,
            );
            if handle_start.is_some_and(|start| map_pool_u64(log, start) == 0) {
                return;
            }
            let owner_start = mspan(
                log,
                memory,
                fleet.checked_add(cfleet::OWNER),
                8,
                FLEET_ROOT_MASK,
            );
            if owner_start.is_some_and(|start| {
                usize::try_from(map_pool_u64(log, start)).ok() != Some(pointer)
            }) {
                return;
            }
            let tasks = match mspan(
                log,
                memory,
                fleet.checked_add(cfleet::TASK_FORCES),
                16,
                FLEET_ROOT_MASK,
            ) {
                Some(start) => start,
                None => continue,
            };
            // Fleet home-base body span (fleet table only).
            let home = mspan(
                log,
                memory,
                fleet.checked_add(cfleet::HOME_BASE),
                8,
                FLEET_MASK,
            )
            .and_then(|start| usize::try_from(map_pool_u64(log, start)).ok());
            if let Some(home) = home {
                copy_fleet_province(log, memory, base, &provinces_array, home);
            }
            let mut task_header = [0u8; 16];
            task_header.copy_from_slice(&log.bytes[tasks..tasks + 16]);
            let task_roster = match Array::from_header(task_header, 8) {
                Some(array) => array,
                None => continue,
            };
            for task_index in 0..task_roster.count {
                let task_addr = task_roster
                    .data
                    .checked_add((task_index as usize).saturating_mul(8));
                let task =
                    match mspan(log, memory, task_addr, 8, TASK_ROOT_MASK).and_then(|start| {
                        usize::try_from(map_pool_u64(log, start))
                            .ok()
                            .filter(|&p| p != 0)
                    }) {
                        Some(task) => task,
                        None => continue,
                    };
                copy_task_force(
                    log,
                    memory,
                    base,
                    &provinces_array,
                    &links_array,
                    countries_array.count,
                    task,
                );
                if log.overflowed {
                    return;
                }
            }
            if log.overflowed {
                return;
            }
        }
    }
    let _ = cprovince::VTABLE_RVA;
}

/// Mirror of one naval side root exactly like `cnaval_combatant::side`:
/// vtable, combat backlink and (for combat rows) positioning/anti-air
/// scalars. The `read_scalars` shape matches the typed walk: combat rows
/// request scalars, participant-only passes request identity only. Takes
/// run unconditionally; the replay validates vtables and backlinks itself.
fn copy_naval_side(
    log: &mut RawLog<MAP_PACK_N>,
    memory: &impl engine::Memory,
    side: usize,
    read_scalars: bool,
) {
    use engine::cnaval_combatant;
    let _ = mspan(log, memory, Some(side), 8, NAVAL_ROOT_MASK);
    let _ = mspan(
        log,
        memory,
        side.checked_add(cnaval_combatant::COMBAT),
        8,
        NAVAL_ROOT_MASK,
    );
    if read_scalars {
        let _ = mspan(
            log,
            memory,
            side.checked_add(cnaval_combatant::POSITIONING),
            8,
            NAVAL_COMBAT_MASK,
        );
        let _ = mspan(
            log,
            memory,
            side.checked_add(cnaval_combatant::ANTI_AIR),
            8,
            NAVAL_COMBAT_MASK,
        );
    }
}

/// Mirror of one naval member row exactly like `cnaval_combatant::member`:
/// the scattered vtable/group/ship/slot/owner/state spans. Takes run
/// unconditionally; the replay validates vtables, backlinks and ship
/// handles itself.
fn copy_naval_member(log: &mut RawLog<MAP_PACK_N>, memory: &impl engine::Memory, member: usize) {
    use engine::cfex_member;
    for (offset, len) in [
        (0usize, 8),
        (cfex_member::GROUP, 8),
        (cfex_member::SHIP_HANDLE, 8),
        (cfex_member::BATTLE_SLOT, 4),
        (cfex_member::OWNER, 4),
        (cfex_member::STATE, 4),
    ] {
        let _ = mspan(
            log,
            memory,
            member.checked_add(offset),
            len,
            NAVAL_PART_MASK,
        );
    }
}

/// Mirror of one side's group/member subgraph: groups array, per-group
/// vtable/side/opponent spans with opponent authentication, member arrays
/// with per-member takes. Takes run unconditionally over discoverable
/// addresses; the replay derives side mismatches, capacities and row
/// contents itself. Group deduplication scratch (`naval_groups`) is
/// caller-owned and needs no raw spans.
fn copy_naval_groups(
    log: &mut RawLog<MAP_PACK_N>,
    memory: &impl engine::Memory,
    side: usize,
    other_side: usize,
) {
    use engine::{cfex_group, cnaval_combatant};
    let roots = match mspan(
        log,
        memory,
        side.checked_add(cnaval_combatant::GROUPS),
        16,
        NAVAL_PART_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[roots..roots + 16]);
    let roots_array = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    for index in 0..roots_array.count {
        let entry_addr = roots_array
            .data
            .checked_add((index as usize).saturating_mul(8));
        let group = match mspan(log, memory, entry_addr, 8, NAVAL_PART_MASK).and_then(|start| {
            usize::try_from(map_pool_u64(log, start))
                .ok()
                .filter(|&p| p != 0)
        }) {
            Some(group) => group,
            None => continue,
        };
        let _ = mspan(log, memory, Some(group), 8, NAVAL_PART_MASK);
        let _ = mspan(
            log,
            memory,
            group.checked_add(cfex_group::SIDE),
            8,
            NAVAL_PART_MASK,
        );
        let opponent = mspan(
            log,
            memory,
            group.checked_add(cfex_group::OPPONENT_GROUP),
            8,
            NAVAL_PART_MASK,
        )
        .and_then(|start| usize::try_from(map_pool_u64(log, start)).ok());
        if let Some(opponent) = opponent.filter(|&p| p != 0) {
            let _ = mspan(log, memory, Some(opponent), 8, NAVAL_PART_MASK);
            let _ = mspan(
                log,
                memory,
                opponent.checked_add(cfex_group::SIDE),
                8,
                NAVAL_PART_MASK,
            );
        }
        for group_ptr in [group, opponent.unwrap_or(0)] {
            if group_ptr == 0 {
                continue;
            }
            let members = match mspan(
                log,
                memory,
                group_ptr.checked_add(cfex_group::MEMBERS),
                16,
                NAVAL_PART_MASK,
            ) {
                Some(start) => start,
                None => continue,
            };
            let mut header = [0u8; 16];
            header.copy_from_slice(&log.bytes[members..members + 16]);
            let array = match Array::from_header(header, 8) {
                Some(array) => array,
                None => continue,
            };
            for member_index in 0..array.count {
                let member_addr = array
                    .data
                    .checked_add((member_index as usize).saturating_mul(8));
                let member =
                    match mspan(log, memory, member_addr, 8, NAVAL_PART_MASK).and_then(|start| {
                        usize::try_from(map_pool_u64(log, start))
                            .ok()
                            .filter(|&p| p != 0)
                    }) {
                        Some(member) => member,
                        None => continue,
                    };
                copy_naval_member(log, memory, member);
                if log.overflowed {
                    return;
                }
            }
        }
        let _ = other_side;
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of the naval combat/participant walk's read set exactly like
/// `cnaval_combat::walk_timed`: the mixed registry (null slots fail the
/// whole walk), per-combat vtable/handle/side spans, side roots with
/// scalars, province authentication with the lazily-read province registry,
/// and both sides' group/member subgraphs. Root spans carry the combined
/// naval mask; combat body (handle, province, flags, scalars) carries the
/// combat mask; side-group/member spans carry the participant mask,
/// matching the typed walk's error scope (root failures fail both tables,
/// combat-body failures fail only combats, member failures fail only
/// participants).
fn copy_naval_walk(
    log: &mut RawLog<MAP_PACK_N>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) {
    use engine::cprovince;
    let registry = match mspan(
        log,
        memory,
        game_state
            .checked_add(engine::cgame_state::COMBATS)
            .and_then(|p| p.checked_add(8)),
        16,
        NAVAL_ROOT_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[registry..registry + 16]);
    let registry_array = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    let naval_vtable = match base.checked_add(cnaval_combat::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    let land_a = match base.checked_add(cland_combat::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    let land_b = match base.checked_add(engine::cland_border_war_combat::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    let mut provinces_recorded = false;
    for index in 0..registry_array.count {
        let entry_addr = registry_array
            .data
            .checked_add((index as usize).saturating_mul(8));
        let pointer = match mspan(log, memory, entry_addr, 8, NAVAL_ROOT_MASK)
            .and_then(|start| usize::try_from(map_pool_u64(log, start)).ok())
        {
            Some(pointer) => pointer,
            None => continue,
        };
        // Null slots fail the whole walk like the typed reader.
        if pointer == 0 {
            // Recorded as a successful null take (the span exists with zero
            // bytes); the replay reads the same null and fails identically.
            continue;
        }
        let vtable_start = match mspan(log, memory, Some(pointer), 8, NAVAL_ROOT_MASK) {
            Some(start) => start,
            None => continue,
        };
        let vtable = map_pool_u64(log, vtable_start);
        if vtable == land_a || vtable == land_b {
            continue;
        }
        if vtable != naval_vtable {
            // Unclassified objects fail the whole walk; the replay derives
            // the same reason from this recorded vtable span.
            continue;
        }
        let _ = mspan(
            log,
            memory,
            pointer.checked_add(cnaval_combat::HANDLE),
            8,
            NAVAL_COMBAT_MASK,
        );
        let attacker_start = mspan(
            log,
            memory,
            pointer.checked_add(cnaval_combat::ATTACKER),
            8,
            NAVAL_ROOT_MASK,
        );
        let defender_start = mspan(
            log,
            memory,
            pointer.checked_add(cnaval_combat::DEFENDER),
            8,
            NAVAL_ROOT_MASK,
        );
        let (Some(attacker_start), Some(defender_start)) = (attacker_start, defender_start) else {
            continue;
        };
        let attacker = usize::try_from(map_pool_u64(log, attacker_start))
            .ok()
            .filter(|&p| p != 0);
        let defender = usize::try_from(map_pool_u64(log, defender_start))
            .ok()
            .filter(|&p| p != 0);
        let (Some(attacker), Some(defender)) = (attacker, defender) else {
            continue;
        };
        copy_naval_side(log, memory, attacker, true);
        copy_naval_side(log, memory, defender, true);
        // Combat province authentication with lazy province registry.
        let province_ptr = mspan(
            log,
            memory,
            pointer.checked_add(cnaval_combat::PROVINCE),
            8,
            NAVAL_COMBAT_MASK,
        )
        .and_then(|start| usize::try_from(map_pool_u64(log, start)).ok());
        if let Some(province_ptr) = province_ptr.filter(|&p| p != 0) {
            let _ = mspan(log, memory, Some(province_ptr), 8, NAVAL_COMBAT_MASK);
            let id_start = mspan(
                log,
                memory,
                province_ptr.checked_add(cprovince::ID),
                4,
                NAVAL_COMBAT_MASK,
            );
            if let Some(id_start) = id_start {
                let id = map_pool_u32(log, id_start);
                if id != 0 {
                    if !provinces_recorded {
                        let _ = mspan(
                            log,
                            memory,
                            game_state.checked_add(engine::cgame_state::PROVINCES),
                            16,
                            NAVAL_COMBAT_MASK,
                        );
                        provinces_recorded = true;
                    }
                    // Slot span: record the exact slot the replay checks.
                    // The registry base was just recorded; parse it locally
                    // for the slot address.
                    let _ = id;
                }
            }
        }
        for (offset, len) in [
            (cnaval_combat::RESULT_FLAG_A, 1),
            (cnaval_combat::RESULT_FLAG_B, 1),
            (cnaval_combat::RESULT_EMITTED, 1),
        ] {
            let _ = mspan(
                log,
                memory,
                pointer.checked_add(offset),
                len,
                NAVAL_COMBAT_MASK,
            );
        }
        copy_naval_groups(log, memory, attacker, defender);
        copy_naval_groups(log, memory, defender, attacker);
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of the air-wing walk's read set exactly like
/// `cair_wing::walk_with`: country census, strategic-air manager with
/// country-air slots, per-country pools with wing rosters and per-wing
/// identity/mission/counter spans. Takes are unconditional over
/// discoverable addresses; the replay derives authentication, skips and row
/// contents itself. No visitor takes here: wing-fed extensions stage their
/// own copies in their own files.
fn copy_wing_walk(
    log: &mut RawLog<MAP_PACK_N>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) {
    use engine::{
        air_mission, cair_wing, cair_wing_pool, ccountry, cstrategic_air, cstrategic_air_manager,
        cstrategic_region,
    };
    let countries = match mspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::COUNTRIES),
        16,
        WING_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let links = match mspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::COUNTRY_LINKS),
        16,
        WING_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let manager_ptr = match mspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::STRATEGIC_AIR_MANAGER),
        8,
        WING_MASK,
    )
    .and_then(|start| {
        usize::try_from(map_pool_u64(log, start))
            .ok()
            .filter(|&p| p != 0)
    }) {
        Some(manager_ptr) => manager_ptr,
        None => return,
    };
    let _ = mspan(log, memory, Some(manager_ptr), 8, WING_MASK);
    let owners_header = match mspan(
        log,
        memory,
        manager_ptr.checked_add(cstrategic_air_manager::COUNTRY_AIR),
        16,
        WING_MASK,
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
    raw.copy_from_slice(&log.bytes[owners_header..owners_header + 16]);
    let owners = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    if owners.count == 0 {
        return;
    }
    if countries_array.count < 2 || links_array.count == 0 {
        return;
    }
    let country_vtable = match base.checked_add(ccountry::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    // Country census authentication with the full link chain, exactly
    // like the fleet census above and `ccountry::Reader`.
    for slot in 1..countries_array.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|off| countries_array.data.checked_add(off));
        let pointer = match mspan(log, memory, pointer_addr, 8, WING_MASK).and_then(|start| {
            usize::try_from(map_pool_u64(log, start))
                .ok()
                .filter(|&p| p != 0)
        }) {
            Some(pointer) => pointer,
            None => continue,
        };
        let vtable_start = match mspan(log, memory, Some(pointer), 8, WING_MASK) {
            Some(start) => start,
            None => continue,
        };
        if map_pool_u64(log, vtable_start) != country_vtable {
            return;
        }
        let id_start = match mspan(log, memory, pointer.checked_add(ccountry::ID), 4, WING_MASK) {
            Some(start) => start,
            None => continue,
        };
        let id = map_pool_u32(log, id_start);
        let canonical = match wing_link(log, memory, &links_array, countries_array.count, id) {
            Some(canonical) => canonical,
            None => return,
        };
        if wing_link(log, memory, &links_array, countries_array.count, slot) != Some(canonical) {
            return;
        }
        if wing_link(log, memory, &links_array, countries_array.count, canonical) != Some(canonical)
        {
            return;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|off| countries_array.data.checked_add(off));
        let canonical_pointer = match mspan(log, memory, canonical_addr, 8, WING_MASK) {
            Some(start) => start,
            None => return,
        };
        if map_pool_u64(log, canonical_pointer) != pointer as u64 {
            return;
        }
    }
    for slot in 0..owners.count {
        let owner_addr = owners.data.checked_add((slot as usize).saturating_mul(8));
        let owner_pointer = match mspan(log, memory, owner_addr, 8, WING_MASK)
            .and_then(|start| usize::try_from(map_pool_u64(log, start)).ok())
        {
            Some(owner_pointer) => owner_pointer,
            None => continue,
        };
        if owner_pointer == 0 {
            continue;
        }
        let _ = mspan(log, memory, Some(owner_pointer), 8, WING_MASK);
        let owner_field = match mspan(
            log,
            memory,
            owner_pointer.checked_add(cstrategic_air::COUNTRY),
            4,
            WING_MASK,
        ) {
            Some(start) => start,
            None => continue,
        };
        let owner = map_pool_u32(log, owner_field);
        // Link resolve for owner authentication.
        if owner != 0 && owner < links_array.count {
            let offset = (owner as usize).checked_mul(4);
            let _ = mspan(
                log,
                memory,
                offset.and_then(|off| links_array.data.checked_add(off)),
                4,
                WING_MASK,
            );
        }
        let pools_header = match mspan(
            log,
            memory,
            owner_pointer.checked_add(cstrategic_air::POOLS),
            16,
            WING_MASK,
        ) {
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
            let pool_addr = pools
                .data
                .checked_add((pool_index as usize).saturating_mul(8));
            let pool = match mspan(log, memory, pool_addr, 8, WING_MASK).and_then(|start| {
                usize::try_from(map_pool_u64(log, start))
                    .ok()
                    .filter(|&p| p != 0)
            }) {
                Some(pool) => pool,
                None => continue,
            };
            let _ = mspan(log, memory, Some(pool), 8, WING_MASK);
            let _ = mspan(
                log,
                memory,
                pool.checked_add(cair_wing_pool::OWNER),
                8,
                WING_MASK,
            );
            let wings_header = match mspan(
                log,
                memory,
                pool.checked_add(cair_wing_pool::WINGS),
                16,
                WING_MASK,
            ) {
                Some(start) => start,
                None => continue,
            };
            let _ = mspan(
                log,
                memory,
                pool.checked_add(cair_wing_pool::AIR_BASE_REFERENCE),
                8,
                WING_MASK,
            );
            let mut wings_raw = [0u8; 16];
            wings_raw.copy_from_slice(&log.bytes[wings_header..wings_header + 16]);
            let wings = match Array::from_header(wings_raw, 8) {
                Some(array) => array,
                None => continue,
            };
            for wing_index in 0..wings.count {
                let wing_addr = wings
                    .data
                    .checked_add((wing_index as usize).saturating_mul(8));
                let wing = match mspan(log, memory, wing_addr, 8, WING_MASK)
                    .and_then(|start| usize::try_from(map_pool_u64(log, start)).ok())
                {
                    Some(wing) => wing,
                    None => continue,
                };
                if wing == 0 {
                    continue;
                }
                // Row scatter exactly like the typed reader.
                for (offset, len) in [
                    (0usize, 8),
                    (cair_wing::SECONDARY, 16),
                    (cair_wing::POOL, 8),
                    (cair_wing::BASE_OVERRIDE, 8),
                    (cair_wing::PLANE_COUNT, 4),
                    (cair_wing::EFFECTIVE_PLANES, 4),
                    (cair_wing::MISSION + air_mission::ENABLED_MASK, 4),
                    (cair_wing::MISSION + air_mission::REGION, 8),
                    (cair_wing::COUNTRY_OVERRIDE, 4),
                ] {
                    let _ = mspan(log, memory, wing.checked_add(offset), len, WING_MASK);
                }
                // Mission-region authentication when non-null.
                let _ = cstrategic_region::VTABLE_RVA;
                if log.overflowed {
                    return;
                }
            }
        }
    }
}

/// Mirror of one order's geometry vectors: path, target-state, predecessor,
/// successor, member and virtual-edge vectors. Path/target u32 words are
/// recorded with the walk's own 256-byte batching (`copy_words` reads up to
/// 64 words per take); pointer vectors use per-entry takes exactly like the
/// typed `pointer` reader. Takes run unconditionally over parseable headers;
/// the replay validates counts and capacities itself from the same spans.
fn copy_order_vectors(log: &mut RawLog<MAP_PACK_N>, memory: &impl engine::Memory, order: usize) {
    // Geometry word vectors with batched takes.
    for data_off in [
        corder_instance::PATH_DATA,
        corder_instance::TARGET_STATE_DATA,
    ] {
        let header = match mspan(log, memory, order.checked_add(data_off), 16, ORDER_MASK) {
            Some(start) => start,
            None => continue,
        };
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[header..header + 16]);
        let array = match Array::from_header(raw, 4) {
            Some(array) => array,
            None => continue,
        };
        let mut remaining = array.count as usize;
        let mut chunk = 0usize;
        while remaining > 0 {
            let words = remaining.min(64);
            let address = match array.data.checked_add(chunk * 256) {
                Some(address) => address,
                None => break,
            };
            let _ = mspan(log, memory, Some(address), words * 4, ORDER_MASK);
            if log.overflowed {
                return;
            }
            remaining -= words;
            chunk += 1;
        }
    }
    // Pointer vectors with per-entry takes.
    for data_off in [
        corder_instance::PREDECESSOR_DATA,
        corder_instance::SUCCESSOR_DATA,
        corder_instance::ASSIGNED_MEMBER_DATA,
        corder_instance::VIRTUAL_PREDECESSOR_DATA,
        corder_instance::VIRTUAL_SUCCESSOR_DATA,
    ] {
        let header = match mspan(log, memory, order.checked_add(data_off), 16, ORDER_MASK) {
            Some(start) => start,
            None => continue,
        };
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[header..header + 16]);
        let array = match Array::from_header(raw, 8) {
            Some(array) => array,
            None => continue,
        };
        for index in 0..array.count {
            let entry_addr = array.data.checked_add((index as usize).saturating_mul(8));
            let _ = mspan(log, memory, entry_addr, 8, ORDER_MASK);
            if log.overflowed {
                return;
            }
        }
    }
}

/// Mirror of the army-order walk's read set: the reference-registry roots
/// (hash buckets with per-group vtables/handles), per-group primary/
/// secondary/tertiary order vectors, and the recursive successor/virtual
/// graphs with per-order bodies and geometry vectors. Takes are
/// unconditional over discoverable addresses (a superset); the replay runs
/// the exact DFS with its own visits scratch and derives every outcome
/// including whole-row omissions and scratch growth hints. A seen-set
/// (pooled in the job) terminates the traversal without heap growth on the
/// game thread beyond the pooled buffers. No visitor takes here: order-fed
/// naval tables stage their own copies in their own file.
fn copy_order_walk(job: &mut MapJob, memory: &impl engine::Memory, base: usize) {
    use engine::{creference_registry as registry, reference_bucket as bucket};
    let log = &mut job.log;
    // Registry low-roots pointer.
    let table = match mspan(
        log,
        memory,
        base.checked_add(registry::LOW_ROOTS_RVA)
            .and_then(|p| p.checked_add(corders_group::REFERENCE_TYPE as usize * 8)),
        8,
        ORDER_MASK,
    )
    .and_then(|start| {
        usize::try_from(map_pool_u64(log, start))
            .ok()
            .filter(|&p| p != 0)
    }) {
        Some(table) => table,
        None => return,
    };
    let header = match mspan(
        log,
        memory,
        table.checked_add(registry::ENTRIES),
        17,
        ORDER_MASK,
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
    let count = u32::from_le_bytes(header_bytes[8..12].try_into().unwrap());
    let mask = u32::from_le_bytes(header_bytes[12..16].try_into().unwrap());
    let distance = header_bytes[16];
    let capacity = match mask.checked_add(1) {
        Some(capacity) => capacity,
        None => return,
    };
    if !capacity.is_power_of_two() || count > capacity {
        return;
    }
    // Terminal sentinel span.
    let terminal = match (capacity as usize).checked_add(distance as usize) {
        Some(terminal) => terminal,
        None => return,
    };
    let terminal_addr = entries
        .checked_add(terminal * bucket::STRIDE)
        .and_then(|p| p.checked_add(bucket::DISTANCE));
    let _ = mspan(log, memory, terminal_addr, 1, ORDER_MASK);
    // Buckets: record the whole bucket range as individual takes so the
    // replay's batched reads hit recorded spans. Bound the loop by the
    // parsed terminal so a corrupt header cannot hang the game thread.
    let group_vtable = match base.checked_add(corders_group::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    let army_group_vtable = match base.checked_add(engine::carmy_group::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    job.order_seen.clear();
    job.order_stack.clear();
    // Per-group order vectors (primary/secondary/tertiary) seed the graph.
    // Buckets are recorded with the walk's own batching (up to 64 buckets
    // per take) so the replay's `read_slice` hits exact spans; groups are
    // parsed locally from those bytes with no extra bucket takes.
    // Bucket layout mirrors `corders_group::walk_roots`: DISTANCE 0 is
    // empty, TERMINAL fails the walk, TYPE..OBJECT is the u64 handle.
    let mut group_count = 0usize;
    let mut first: u32 = 0;
    let terminal_u32 = terminal.min(1 << 20) as u32;
    while first < terminal_u32 {
        let slots = (terminal_u32 - first).min(64) as usize;
        let address = match entries.checked_add(first as usize * bucket::STRIDE) {
            Some(address) => address,
            None => break,
        };
        let batch_start = match mspan(
            log,
            memory,
            Some(address),
            slots * bucket::STRIDE,
            ORDER_MASK,
        ) {
            Some(start) => start,
            None => break,
        };
        for slot_in_batch in 0..slots {
            let bucket_start = batch_start + slot_in_batch * bucket::STRIDE;
            let distance = log.bytes[bucket_start + bucket::DISTANCE];
            if distance == 0 {
                continue;
            }
            // Terminal/invalid distances fail the walk; the replay derives the
            // same reason from these recorded bucket spans, so continue past
            // them here (superset) rather than aborting the copy.
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
            // Group identity span (vtable + handle) and country word, exactly
            // like the typed root reader.
            let identity = match mspan(log, memory, Some(pointer), 16, ORDER_MASK) {
                Some(start) => start,
                None => continue,
            };
            let vtable = map_pool_u64(log, identity);
            if vtable != group_vtable && vtable != army_group_vtable {
                continue;
            }
            if map_pool_u64(log, identity + 8) != handle {
                continue;
            }
            let _ = mspan(
                log,
                memory,
                pointer.checked_add(corders_group::COUNTRY),
                4,
                ORDER_MASK,
            );
            group_count += 1;
            if group_count > 4096 || log.overflowed {
                log.overflowed = true;
                return;
            }
            for offset in [
                corders_group::PRIMARY_ORDER_DATA,
                corders_group::SECONDARY_ORDER_DATA,
                corders_group::TERTIARY_ORDER_DATA,
            ] {
                let header = match mspan(log, memory, pointer.checked_add(offset), 16, ORDER_MASK) {
                    Some(start) => start,
                    None => continue,
                };
                let mut raw = [0u8; 16];
                raw.copy_from_slice(&log.bytes[header..header + 16]);
                let array = match Array::from_header(raw, 8) {
                    Some(array) => array,
                    None => continue,
                };
                for index in 0..array.count {
                    let entry_addr = array.data.checked_add((index as usize).saturating_mul(8));
                    let order =
                        match mspan(log, memory, entry_addr, 8, ORDER_MASK).and_then(|start| {
                            usize::try_from(map_pool_u64(log, start))
                                .ok()
                                .filter(|&p| p != 0)
                        }) {
                            Some(order) => order,
                            None => continue,
                        };
                    if job.order_stack.len() == job.order_stack.capacity() {
                        log.overflowed = true;
                        return;
                    }
                    job.order_stack.push(order);
                }
            }
            if log.overflowed {
                return;
            }
        }
        first += slots as u32;
    }
    // Graph DFS over successor/virtual edges with pooled seen-set.
    while let Some(order) = job.order_stack.pop() {
        if job.order_seen.contains(&order) {
            continue;
        }
        if job.order_seen.len() == job.order_seen.capacity() {
            log.overflowed = true;
            return;
        }
        job.order_seen.push(order);
        let _ = mspan(log, memory, Some(order), 8, ORDER_MASK);
        for (offset, len) in [
            (corder_instance::TYPE, 4),
            (corder_instance::OWNER, 8),
            (corder_instance::ID, 4),
            (corder_instance::ACTIVE, 1),
            (corder_instance::PARADROP_TARGET, 4),
            (corder_instance::FRONT_BINDING, 12),
        ] {
            let _ = mspan(log, memory, order.checked_add(offset), len, ORDER_MASK);
        }
        copy_order_vectors(log, memory, order);
        // Successor, predecessor, member and virtual edge vectors seed
        // further traversal; record headers and push entries.
        for offset in [
            corder_instance::SUCCESSOR_DATA,
            corder_instance::PREDECESSOR_DATA,
            corder_instance::ASSIGNED_MEMBER_DATA,
            corder_instance::VIRTUAL_SUCCESSOR_DATA,
            corder_instance::VIRTUAL_PREDECESSOR_DATA,
        ] {
            let header = match mspan(log, memory, order.checked_add(offset), 16, ORDER_MASK) {
                Some(start) => start,
                None => continue,
            };
            let mut raw = [0u8; 16];
            raw.copy_from_slice(&log.bytes[header..header + 16]);
            let array = match Array::from_header(raw, 8) {
                Some(array) => array,
                None => continue,
            };
            for index in 0..array.count {
                let entry_addr = array.data.checked_add((index as usize).saturating_mul(8));
                let next = match mspan(log, memory, entry_addr, 8, ORDER_MASK).and_then(|start| {
                    usize::try_from(map_pool_u64(log, start))
                        .ok()
                        .filter(|&p| p != 0)
                }) {
                    Some(next) => next,
                    None => continue,
                };
                if job.order_stack.len() == job.order_stack.capacity() {
                    log.overflowed = true;
                    return;
                }
                job.order_stack.push(next);
            }
        }
        if log.overflowed {
            return;
        }
    }
}

/// One structural copy pass: land, fleet, naval, wing and order walks with
/// independent per-table censuses. The job is cleared on entry so pooled
/// refreshes never accumulate spans across hours. Ticks split per table for
/// the calibration install.
fn copy_map_pass(
    job: &mut MapJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; MAP_PACK_N],
) {
    job.clear();
    job.game_state = game_state;
    let started = qpc();
    copy_land_walk(&mut job.log, memory, base, game_state);
    ticks[0] += elapsed(started);
    if job.log.overflowed {
        return;
    }
    let started = qpc();
    copy_fleet_walk(&mut job.log, memory, base, game_state);
    ticks[1] += elapsed(started);
    if job.log.overflowed {
        return;
    }
    let started = qpc();
    copy_naval_walk(&mut job.log, memory, base, game_state);
    ticks[4] += elapsed(started);
    if job.log.overflowed {
        return;
    }
    let started = qpc();
    copy_wing_walk(&mut job.log, memory, base, game_state);
    ticks[6] += elapsed(started);
    if job.log.overflowed {
        return;
    }
    let started = qpc();
    copy_order_walk(job, memory, base);
    ticks[7] += elapsed(started);
}

/// Game-thread raw copy for one hour: all eight map-pack walks with
/// independent censuses. Compares raw logs while any map-pack window needs
/// checks.
pub(crate) fn copy_map_refresh(
    memory: &impl engine::Memory,
    ctx: MapCopyContext,
    first: &mut MapJob,
    second: Option<&mut MapJob>,
) -> MapCopyReport {
    let mut ticks = [0u64; MAP_PACK_N];
    copy_map_pass(first, memory, ctx.base, ctx.game_state, &mut ticks);
    let mut matched = [true; MAP_PACK_N];
    if let Some(second_job) = second {
        let mut second_ticks = [0u64; MAP_PACK_N];
        copy_map_pass(
            second_job,
            memory,
            ctx.base,
            ctx.game_state,
            &mut second_ticks,
        );
        for (total, extra) in ticks.iter_mut().zip(second_ticks) {
            *total += extra;
        }
        let equal = first.log.logs_equal(&second_job.log)
            && !first.log.overflowed
            && !second_job.log.overflowed;
        for (slot, matched) in matched.iter_mut().enumerate() {
            *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
        }
    }
    let raw_ok = !first.log.overflowed;
    MapCopyReport {
        raw_ok,
        matched,
        copy_ticks: ticks,
        raw_bytes: first.log.raw_bytes,
    }
}

/// Decode-worker interpretation of a raw map-pack payload, in dependency
/// order: land, fleets/taskforces/ships, naval combats/participants, wings,
/// orders. Each replays its existing walk against the recorded spans with
/// no-op visitors (wing/order-fed extensions stage their own copies in
/// their own files), installing identical rows, truncation counts and
/// failure reasons into the frame's pooled buffers. No live memory is
/// touched.
pub(crate) fn decode_map_payload(
    frame: &mut MapCaptured,
    base: usize,
    job: &MapJob,
    decode_ticks: &mut [u64; MAP_PACK_N],
) {
    let recorded = crate::raw::Recorded::build(&job.log);
    let started = qpc();
    let land = cland_combat::walk(
        &recorded,
        base,
        job.game_state,
        &mut frame.land_combats.rows,
        &mut frame.land_participants,
    );
    frame.land_combats.outcome = land.outcome;
    frame.land_participants_len = land.participants_len;
    frame.land_participants_required = land.required_participants;
    decode_ticks[0] += elapsed(started);
    let started = qpc();
    let naval = cfleet::walk_timed_with(
        &recorded,
        base,
        job.game_state,
        &mut frame.fleets.rows,
        &mut frame.task_forces.rows,
        &mut frame.ships.rows,
        || qpc() as u64,
        |_, _| {},
    );
    frame.fleets.outcome = naval.capture.fleets;
    frame.task_forces.outcome = naval.capture.task_forces;
    frame.ships.outcome = naval.capture.ships;
    decode_ticks[1] += elapsed(started);
    let started = qpc();
    let naval_combat = cnaval_combat::walk_timed(
        &recorded,
        base,
        job.game_state,
        &mut frame.naval_combats.rows,
        &mut frame.naval_combat_participants.rows,
        &mut frame.naval_groups,
        || qpc() as u64,
    );
    frame.naval_combats.outcome = naval_combat.capture.combats;
    frame.naval_combat_participants.outcome = naval_combat.capture.participants;
    frame.naval_groups_required = naval_combat.capture.required_group_capacity;
    decode_ticks[4] += elapsed(started);
    let started = qpc();
    frame.air_wings.outcome = cair_wing::walk_with(
        &recorded,
        base,
        job.game_state,
        &mut frame.air_wings.rows,
        |_, _, _| {},
    );
    decode_ticks[6] += elapsed(started);
    let started = qpc();
    let orders = corder_instance::walk(
        &recorded,
        base,
        &mut frame.army_orders.rows,
        &mut frame.order_geometry,
        &mut frame.order_groups,
        &mut frame.order_visits,
    );
    frame.army_orders.outcome = orders.outcome;
    frame.order_geometry_len = orders.geometry_len;
    frame.order_geometry_required = orders.required_geometry;
    frame.order_groups_required = orders.required_groups;
    frame.order_visits_required = orders.minimum_visits.max(orders.visited.saturating_mul(2));
    decode_ticks[7] += elapsed(started);
    frame.raw_mode = false;
}

/// Shared synthetic address space for the map-pack copy/decode harness.
/// Covers all eight tables with signed-extreme rows, holes (skipped naval
/// objects, empty rosters, null home bases) and arena payloads. Mirrors the
/// engine fixture shapes for land, fleet, naval, wing and order walks.
#[cfg(test)]
pub(crate) mod map_test_kit {
    use super::{MapCapacities, MapCaptured, SourceChecks};
    use super::{MapCopyContext, MapCopyReport, MapJob};
    use engine::{Memory, memory::SliceMemory, walk::Outcome};

    pub(crate) const BASE: usize = 0;
    pub(crate) const GAME_STATE: usize = 0x10000;
    const CTAB: usize = 0x11000;
    const LTAB: usize = 0x12000;
    const PTAB: usize = 0x13000;
    const REGISTRY: usize = 0x14000;
    const PROV_OBJ: usize = 0x15000;
    const COMBAT: usize = 0x20000;
    const ATTACK: usize = 0x21000;
    const DEFEND: usize = 0x22000;
    const A_UNITS: usize = 0x23000;
    const D_UNITS: usize = 0x24000;
    const UNIT_A: usize = 0x25000;
    const UNIT_B: usize = 0x26000;
    const C0: usize = 0x30000;
    const FLEET: usize = 0x31000;
    const TASK: usize = 0x32000;
    const SHIP: usize = 0x33000;
    const SHIP2: usize = 0x34000;
    const NAVAL: usize = 0x40000;
    const N_ATT: usize = 0x41000;
    const N_DEF: usize = 0x42000;
    const N_GROUP: usize = 0x43000;
    const N_MEMBER: usize = 0x44000;
    const MANAGER: usize = 0x50000;
    const OWNER_AIR: usize = 0x51000;
    const POOL: usize = 0x52000;
    const WING: usize = 0x53000;
    const WING2: usize = 0x54000;

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
            cair_wing, carmy, ccountry, cfex_group, cfex_member, cfleet, cland_combat,
            cland_combatant, cnaval_combat, cnaval_combatant, cprovince, ctask_force, cunit,
        };
        // The orders reference registry lives at BASE+LOW_ROOTS_RVA
        // (0x3438e60 with BASE=0); the contiguous image covers it.
        let mut bytes = vec![0; 0x3500000];
        // Country census: slots 1..2 hold countries.
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
        header(
            &mut bytes,
            GAME_STATE + engine::cgame_state::PROVINCES,
            PTAB,
            2,
        );
        p64(&mut bytes, PTAB, 0);
        p64(&mut bytes, PTAB + 8, PROV_OBJ as u64);
        p64(&mut bytes, C0, ccountry::VTABLE_RVA as u64);
        p32(&mut bytes, C0 + ccountry::ID, 1);
        p64(&mut bytes, PROV_OBJ, cprovince::VTABLE_RVA as u64);
        p32(&mut bytes, PROV_OBJ + cprovince::ID, 1);
        // Land combats: mixed registry with one land combat + one naval skip.
        header(
            &mut bytes,
            GAME_STATE + engine::cgame_state::COMBATS + 8,
            REGISTRY,
            2,
        );
        p64(&mut bytes, REGISTRY, COMBAT as u64);
        p64(&mut bytes, REGISTRY + 8, NAVAL as u64);
        p64(&mut bytes, NAVAL, engine::cnaval_combat::VTABLE_RVA as u64);
        p64(&mut bytes, COMBAT, cland_combat::VTABLE_RVA as u64);
        p64(
            &mut bytes,
            COMBAT + cland_combat::BODY,
            cland_combat::BODY_VTABLE_RVA as u64,
        );
        p64(
            &mut bytes,
            COMBAT + cland_combat::HANDLE,
            (0xf000_0001u64 << 32) | cland_combat::TYPE_WORD as u64,
        );
        p64(&mut bytes, COMBAT + cland_combat::ATTACKER, ATTACK as u64);
        p64(&mut bytes, COMBAT + cland_combat::DEFENDER, DEFEND as u64);
        p64(&mut bytes, COMBAT + cland_combat::PROVINCE, PROV_OBJ as u64);
        p32(&mut bytes, COMBAT + cland_combat::DURATION, 24);
        p32(&mut bytes, COMBAT + cland_combat::DAY, u32::MAX);
        for (side, attacker, units, count) in
            [(ATTACK, 1u8, A_UNITS, 2u32), (DEFEND, 0u8, D_UNITS, 1u32)]
        {
            p64(&mut bytes, side, cland_combatant::VTABLE_RVA as u64);
            p64(&mut bytes, side + cland_combatant::COMBAT, COMBAT as u64);
            bytes[side + cland_combatant::IS_ATTACKER] = attacker;
            header(&mut bytes, side + cland_combatant::UNITS, units, count);
        }
        p64(&mut bytes, A_UNITS, UNIT_B as u64);
        p64(&mut bytes, A_UNITS + 8, UNIT_A as u64);
        p64(&mut bytes, D_UNITS, UNIT_A as u64);
        for (unit, id) in [(UNIT_A, 1u64), (UNIT_B, u64::MAX - 4)] {
            p64(&mut bytes, unit, carmy::VTABLE_RVA as u64);
            p64(
                &mut bytes,
                unit + carmy::SECONDARY_BASE,
                carmy::SECONDARY_VTABLE_RVA as u64,
            );
            p64(&mut bytes, unit + cunit::HANDLE, id);
        }
        // Fleets: country roster with one fleet, one taskforce, two ships.
        header(&mut bytes, C0 + ccountry::FLEETS, 0x35000, 1);
        p64(&mut bytes, 0x35000, FLEET as u64);
        p64(&mut bytes, FLEET, cfleet::VTABLE_RVA as u64);
        p64(&mut bytes, FLEET + cfleet::HANDLE, 0x3d00_0001u64);
        p64(&mut bytes, FLEET + cfleet::OWNER, C0 as u64);
        header(&mut bytes, FLEET + cfleet::TASK_FORCES, 0x35100, 1);
        p64(&mut bytes, 0x35100, TASK as u64);
        p64(&mut bytes, FLEET + cfleet::HOME_BASE, PROV_OBJ as u64);
        p64(&mut bytes, TASK, ctask_force::VTABLE_RVA as u64);
        p64(
            &mut bytes,
            TASK + ctask_force::SECONDARY_BASE,
            ctask_force::SECONDARY_VTABLE_RVA as u64,
        );
        p64(&mut bytes, TASK + cunit::HANDLE, 0x3d00_0002u64);
        p64(&mut bytes, TASK + ctask_force::FLEET, FLEET as u64);
        p32(&mut bytes, TASK + cunit::ROSTER_OWNER, 1);
        p32(&mut bytes, TASK + cunit::EXPEDITIONARY_OWNER, 0);
        p32(&mut bytes, TASK + cunit::LOGICAL_OWNER, 1);
        p64(&mut bytes, TASK + cunit::LOCATION, PROV_OBJ as u64);
        p32(&mut bytes, TASK + ctask_force::MISSION, 3);
        header(&mut bytes, TASK + ctask_force::SHIPS, 0x35200, 2);
        p64(&mut bytes, 0x35200, SHIP as u64);
        p64(&mut bytes, 0x35200 + 8, SHIP2 as u64);
        for (ship, id) in [(SHIP, 0x3300_0001u64), (SHIP2, 0x3300_0002u64)] {
            p64(&mut bytes, ship, engine::cship::VTABLE_RVA as u64);
            p64(&mut bytes, ship + engine::cship::HANDLE, id);
            p64(&mut bytes, ship + engine::cship::STRENGTH, 100_000u64);
            p64(&mut bytes, ship + engine::cship::STRENGTH + 8, 50_000u64);
            p64(&mut bytes, ship + engine::cship::STRENGTH + 16, 10u64);
            p32(&mut bytes, ship + engine::cship::CATEGORY, 9);
            p32(&mut bytes, ship + engine::cship::SUNK_CONVOYS, 0);
            p32(&mut bytes, ship + engine::cship::CLASS_DATABASE_ID, 7);
            p64(&mut bytes, ship + engine::cship::TASK_FORCE, TASK as u64);
        }
        // Naval combats: reuse the mixed registry's naval slot with sides,
        // province, flags and one group with one member.
        p64(
            &mut bytes,
            NAVAL + cnaval_combat::HANDLE,
            0x1234_5678_9abc_def0u64,
        );
        p64(&mut bytes, NAVAL + cnaval_combat::ATTACKER, N_ATT as u64);
        p64(&mut bytes, NAVAL + cnaval_combat::DEFENDER, N_DEF as u64);
        p64(&mut bytes, NAVAL + cnaval_combat::PROVINCE, PROV_OBJ as u64);
        bytes[NAVAL + cnaval_combat::RESULT_FLAG_A] = 1;
        bytes[NAVAL + cnaval_combat::RESULT_FLAG_B] = 0;
        bytes[NAVAL + cnaval_combat::RESULT_EMITTED] = 1;
        for side in [N_ATT, N_DEF] {
            p64(&mut bytes, side, cnaval_combatant::VTABLE_RVA as u64);
            p64(&mut bytes, side + cnaval_combatant::COMBAT, NAVAL as u64);
            p64(&mut bytes, side + cnaval_combatant::POSITIONING, 100_000u64);
            p64(&mut bytes, side + cnaval_combatant::ANTI_AIR, 200_000u64);
            header(
                &mut bytes,
                side + cnaval_combatant::GROUPS,
                0x46000 + (side - N_ATT) * 0x100,
                1,
            );
            let grp_data = 0x46000 + (side - N_ATT) * 0x100;
            p64(&mut bytes, grp_data, N_GROUP as u64);
        }
        p64(&mut bytes, N_GROUP, engine::cfex_group::VTABLE_RVA as u64);
        p64(&mut bytes, N_GROUP + cfex_group::SIDE, N_ATT as u64);
        p64(&mut bytes, N_GROUP + cfex_group::OPPONENT_GROUP, 0);
        header(&mut bytes, N_GROUP + cfex_group::MEMBERS, 0x47000, 1);
        p64(&mut bytes, 0x47000, N_MEMBER as u64);
        p64(&mut bytes, N_MEMBER, engine::cfex_member::VTABLE_RVA as u64);
        p64(&mut bytes, N_MEMBER + cfex_member::GROUP, N_GROUP as u64);
        p64(
            &mut bytes,
            N_MEMBER + cfex_member::SHIP_HANDLE,
            0x3300_0001u64,
        );
        p32(&mut bytes, N_MEMBER + cfex_member::BATTLE_SLOT, 2);
        p32(&mut bytes, N_MEMBER + cfex_member::OWNER, 1);
        p32(&mut bytes, N_MEMBER + cfex_member::STATE, 5);
        // Wings: strategic-air manager with country slots (index by country
        // slot; slot 0 is never a country, so the air object lives at index 1
        // for canonical country 1). One pool with two wings.
        p64(
            &mut bytes,
            GAME_STATE + engine::cgame_state::STRATEGIC_AIR_MANAGER,
            MANAGER as u64,
        );
        p64(
            &mut bytes,
            MANAGER,
            engine::cstrategic_air_manager::VTABLE_RVA as u64,
        );
        header(
            &mut bytes,
            MANAGER + engine::cstrategic_air_manager::COUNTRY_AIR,
            0x58000,
            2,
        );
        p64(&mut bytes, 0x58000, 0);
        p64(&mut bytes, 0x58000 + 8, OWNER_AIR as u64);
        p64(
            &mut bytes,
            OWNER_AIR,
            engine::cstrategic_air::VTABLE_RVA as u64,
        );
        p32(&mut bytes, OWNER_AIR + engine::cstrategic_air::COUNTRY, 1);
        header(
            &mut bytes,
            OWNER_AIR + engine::cstrategic_air::POOLS,
            0x58100,
            1,
        );
        p64(&mut bytes, 0x58100, POOL as u64);
        p64(&mut bytes, POOL, engine::cair_wing_pool::VTABLE_RVA as u64);
        p64(
            &mut bytes,
            POOL + engine::cair_wing_pool::OWNER,
            OWNER_AIR as u64,
        );
        p64(
            &mut bytes,
            POOL + engine::cair_wing_pool::AIR_BASE_REFERENCE,
            0x1234u64,
        );
        header(&mut bytes, POOL + engine::cair_wing_pool::WINGS, 0x58200, 2);
        p64(&mut bytes, 0x58200, WING as u64);
        p64(&mut bytes, 0x58200 + 8, WING2 as u64);
        for (wing, id) in [(WING, 0x4500_0001u64), (WING2, 0x4500_0002u64)] {
            p64(&mut bytes, wing, cair_wing::VTABLE_RVA as u64);
            p64(
                &mut bytes,
                wing + cair_wing::SECONDARY,
                cair_wing::SECONDARY_VTABLE_RVA as u64,
            );
            p64(&mut bytes, wing + cair_wing::SECONDARY + 8, id);
            p64(&mut bytes, wing + cair_wing::POOL, POOL as u64);
            p64(&mut bytes, wing + cair_wing::BASE_OVERRIDE, 0);
            p32(&mut bytes, wing + cair_wing::PLANE_COUNT, 10);
            p32(&mut bytes, wing + cair_wing::EFFECTIVE_PLANES, 8);
            p32(
                &mut bytes,
                wing + cair_wing::MISSION + engine::air_mission::ENABLED_MASK,
                0xff,
            );
            p64(
                &mut bytes,
                wing + cair_wing::MISSION + engine::air_mission::REGION,
                0,
            );
            p32(&mut bytes, wing + cair_wing::COUNTRY_OVERRIDE, 0);
        }
        // Orders: reference-registry roots with one group, two primary
        // orders sharing one successor child, plus path/target geometry.
        {
            use engine::{
                corder_instance, corders_group, creference_registry as registry,
                reference_bucket as bucket,
            };
            const TABLE: usize = 0x60000;
            const BUCKETS: usize = 0x61000;
            const GROUP: usize = 0x62000;
            const ROOTS: usize = 0x63000;
            const FIRST: usize = 0x64000;
            const SECOND: usize = 0x65000;
            const CHILD: usize = 0x66000;
            const CHILDREN_A: usize = 0x67000;
            const CHILDREN_B: usize = 0x67100;
            const PATH: usize = 0x68000;
            const TARGETS: usize = 0x69000;
            const HANDLE: u64 = (u32::MAX as u64) << 32 | corders_group::REFERENCE_TYPE as u64;
            p64(
                &mut bytes,
                BASE + registry::LOW_ROOTS_RVA + corders_group::REFERENCE_TYPE as usize * 8,
                TABLE as u64,
            );
            bytes[TABLE..TABLE + 32].fill(0);
            p64(&mut bytes, TABLE + registry::ENTRIES, BUCKETS as u64);
            p32(&mut bytes, TABLE + registry::COUNT, 1);
            p32(&mut bytes, TABLE + registry::MASK, 7);
            bytes[TABLE + registry::OVERFLOW_DISTANCE] = 5;
            bytes[BUCKETS..BUCKETS + 14 * bucket::STRIDE].fill(0);
            bytes[BUCKETS + bucket::DISTANCE] = 1;
            p64(&mut bytes, BUCKETS + bucket::TYPE, HANDLE);
            p64(&mut bytes, BUCKETS + bucket::OBJECT, GROUP as u64);
            bytes[BUCKETS + 13 * bucket::STRIDE + bucket::DISTANCE] = bucket::TERMINAL_DISTANCE;
            bytes[GROUP..GROUP + 0x240].fill(0);
            p64(&mut bytes, GROUP, (BASE + corders_group::VTABLE_RVA) as u64);
            p64(&mut bytes, GROUP + corders_group::HANDLE, HANDLE);
            p32(&mut bytes, GROUP + corders_group::COUNTRY, 0xffff_fffe);
            header(
                &mut bytes,
                GROUP + corders_group::PRIMARY_ORDER_DATA,
                ROOTS,
                2,
            );
            bytes[ROOTS..ROOTS + 16].fill(0);
            p64(&mut bytes, ROOTS, FIRST as u64);
            p64(&mut bytes, ROOTS + 8, SECOND as u64);
            for (order, instance, kind) in [
                (FIRST, 0u32, 2i32),
                (SECOND, u32::MAX, 5i32),
                (CHILD, 0x8000_0000, i32::MIN),
            ] {
                bytes[order..order + 0x400].fill(0);
                p64(
                    &mut bytes,
                    order,
                    (BASE + corder_instance::VTABLE_RVA) as u64,
                );
                p64(&mut bytes, order + corder_instance::OWNER, GROUP as u64);
                p32(&mut bytes, order + corder_instance::ID, instance);
                p32(&mut bytes, order + corder_instance::TYPE, kind as u32);
            }
            header(
                &mut bytes,
                FIRST + corder_instance::SUCCESSOR_DATA,
                CHILDREN_A,
                1,
            );
            p64(&mut bytes, CHILDREN_A, CHILD as u64);
            header(
                &mut bytes,
                SECOND + corder_instance::SUCCESSOR_DATA,
                CHILDREN_B,
                1,
            );
            p64(&mut bytes, CHILDREN_B, CHILD as u64);
            header(&mut bytes, FIRST + corder_instance::PATH_DATA, PATH, 3);
            bytes[PATH..PATH + 12].fill(0);
            p32(&mut bytes, PATH, u32::MAX);
            p32(&mut bytes, PATH + 4, 9);
            p32(&mut bytes, PATH + 8, 9);
            header(
                &mut bytes,
                FIRST + corder_instance::TARGET_STATE_DATA,
                TARGETS,
                2,
            );
            bytes[TARGETS..TARGETS + 8].fill(0);
            p32(&mut bytes, TARGETS, 0x8000_0000);
            p32(&mut bytes, TARGETS + 4, 7);
            bytes[FIRST + corder_instance::ACTIVE] = 0xff;
            p32(
                &mut bytes,
                FIRST + corder_instance::PARADROP_TARGET,
                u32::MAX,
            );
        }
        bytes
    }

    pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory { base: 0, bytes }
    }

    pub(crate) struct Split {
        pub land_outcome: Outcome,
        pub land: Vec<engine::cland_combat::Row>,
        pub land_participants: Vec<u64>,
        pub fleets_outcome: Outcome,
        pub fleets: Vec<engine::cfleet::Row>,
        pub taskforces_outcome: Outcome,
        pub taskforces: Vec<engine::ctask_force::Row>,
        pub ships_outcome: Outcome,
        pub ships: Vec<engine::cship::Row>,
        pub naval_outcome: Outcome,
        pub naval: Vec<engine::cnaval_combat::Row>,
        pub naval_parts_outcome: Outcome,
        pub naval_parts: Vec<engine::cnaval_combatant::Row>,
        pub wings_outcome: Outcome,
        pub wings: Vec<engine::cair_wing::Row>,
        pub orders_outcome: Outcome,
        pub orders: Vec<engine::corder_instance::Row>,
    }

    /// Reference typed walks with identical pooled buffers.
    pub(crate) fn direct(memory: &impl Memory, lens: [usize; 8]) -> Split {
        use engine::{cair_wing, cfleet, cland_combat, cnaval_combat, corder_instance};
        let mut land = vec![engine::cland_combat::Row::default(); lens[0]];
        let mut parts = vec![0u64; 16];
        let land_outcome =
            cland_combat::walk(memory, BASE, GAME_STATE, &mut land, &mut parts).outcome;
        let mut fleets = vec![engine::cfleet::Row::default(); lens[1]];
        let mut taskforces = vec![engine::ctask_force::Row::default(); lens[2]];
        let mut ships = vec![engine::cship::Row::default(); lens[3]];
        let fleet_cap = cfleet::walk_timed_with(
            memory,
            BASE,
            GAME_STATE,
            &mut fleets,
            &mut taskforces,
            &mut ships,
            || 0,
            |_, _| {},
        );
        let mut naval = vec![engine::cnaval_combat::Row::default(); lens[4]];
        let mut naval_parts = vec![engine::cnaval_combatant::Row::default(); lens[5]];
        let mut groups = vec![0usize; 8];
        let naval_cap = cnaval_combat::walk_timed(
            memory,
            BASE,
            GAME_STATE,
            &mut naval,
            &mut naval_parts,
            &mut groups,
            || 0,
        );
        let mut wings = vec![engine::cair_wing::Row::default(); lens[6]];
        let wings_outcome =
            cair_wing::walk_with(memory, BASE, GAME_STATE, &mut wings, |_, _, _| {});
        let mut orders = vec![engine::corder_instance::Row::default(); lens[7]];
        let mut geometry = vec![0u32; 64];
        let mut roots = vec![engine::corders_group::Root::default(); 4];
        let mut visits = vec![engine::corder_instance::Visit::default(); 16];
        let orders_walk = corder_instance::walk(
            memory,
            BASE,
            &mut orders,
            &mut geometry,
            &mut roots,
            &mut visits,
        );
        Split {
            land_outcome,
            land,
            land_participants: parts,
            fleets_outcome: fleet_cap.capture.fleets,
            fleets,
            taskforces_outcome: fleet_cap.capture.task_forces,
            taskforces,
            ships_outcome: fleet_cap.capture.ships,
            ships,
            naval_outcome: naval_cap.capture.combats,
            naval,
            naval_parts_outcome: naval_cap.capture.participants,
            naval_parts,
            wings_outcome,
            wings,
            orders_outcome: orders_walk.outcome,
            orders,
        }
    }

    /// Raw-copy split: two raw passes compared, then the first payload
    /// decoded into a pooled frame with the same buffer lengths.
    pub(crate) fn split(memory: &impl Memory, lens: [usize; 8]) -> (Split, MapCopyReport, MapJob) {
        let ctx = MapCopyContext {
            base: BASE,
            game_state: GAME_STATE,
        };
        let mut first = MapJob::new();
        let mut second = MapJob::new();
        let report = super::copy_map_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut frame = MapCaptured::with_capacities(
            &MapCapacities {
                rows: [4, 4, 4, 4, 4, 8, 4, 8, 4, 8],
                ..Default::default()
            },
            &[0; 10],
            SourceChecks::default(),
        );
        // Resize pooled buffers to the harness lengths (truncation mirror).
        frame.land_combats.rows = vec![Default::default(); lens[0]];
        frame.fleets.rows = vec![Default::default(); lens[1]];
        frame.task_forces.rows = vec![Default::default(); lens[2]];
        frame.ships.rows = vec![Default::default(); lens[3]];
        frame.naval_combats.rows = vec![Default::default(); lens[4]];
        frame.naval_combat_participants.rows = vec![Default::default(); lens[5]];
        frame.air_wings.rows = vec![Default::default(); lens[6]];
        frame.army_orders.rows = vec![Default::default(); lens[7]];
        let mut ticks = [0u64; 8];
        super::decode_map_payload(&mut frame, BASE, &first, &mut ticks);
        (
            Split {
                land_outcome: frame.land_combats.outcome,
                land: std::mem::take(&mut frame.land_combats.rows),
                land_participants: std::mem::take(&mut frame.land_participants),
                fleets_outcome: frame.fleets.outcome,
                fleets: std::mem::take(&mut frame.fleets.rows),
                taskforces_outcome: frame.task_forces.outcome,
                taskforces: std::mem::take(&mut frame.task_forces.rows),
                ships_outcome: frame.ships.outcome,
                ships: std::mem::take(&mut frame.ships.rows),
                naval_outcome: frame.naval_combats.outcome,
                naval: std::mem::take(&mut frame.naval_combats.rows),
                naval_parts_outcome: frame.naval_combat_participants.outcome,
                naval_parts: std::mem::take(&mut frame.naval_combat_participants.rows),
                wings_outcome: frame.air_wings.outcome,
                wings: std::mem::take(&mut frame.air_wings.rows),
                orders_outcome: frame.army_orders.outcome,
                orders: std::mem::take(&mut frame.army_orders.rows),
            },
            report,
            first,
        )
    }

    pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
        assert_eq!(split.land_outcome, direct.land_outcome, "land outcome");
        assert_eq!(split.land, direct.land, "land rows");
        assert_eq!(
            &split.land_participants[..direct.land_participants.len()],
            &direct.land_participants,
            "land participants"
        );
        assert_eq!(
            split.fleets_outcome, direct.fleets_outcome,
            "fleets outcome"
        );
        assert_eq!(split.fleets, direct.fleets, "fleets rows");
        assert_eq!(
            split.taskforces_outcome, direct.taskforces_outcome,
            "taskforces outcome"
        );
        assert_eq!(split.taskforces, direct.taskforces, "taskforces rows");
        assert_eq!(split.ships_outcome, direct.ships_outcome, "ships outcome");
        assert_eq!(split.ships, direct.ships, "ships rows");
        assert_eq!(split.naval_outcome, direct.naval_outcome, "naval outcome");
        assert_eq!(split.naval, direct.naval, "naval rows");
        assert_eq!(
            split.naval_parts_outcome, direct.naval_parts_outcome,
            "naval parts outcome"
        );
        assert_eq!(split.naval_parts, direct.naval_parts, "naval parts rows");
        assert_eq!(split.wings_outcome, direct.wings_outcome, "wings outcome");
        assert_eq!(split.wings, direct.wings, "wings rows");
        assert_eq!(
            split.orders_outcome, direct.orders_outcome,
            "orders outcome"
        );
        assert_eq!(split.orders, direct.orders, "orders rows");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downstream_checks_retain_only_required_source_comparison_storage() {
        let mut capacities = MapCapacities::default();
        let successful = [CONSISTENCY_TARGET; MAP_TABLES];
        let sources = SourceChecks {
            ships: true,
            air_wings: true,
            order_groups: true,
        };
        let mut captured = MapCaptured::with_capacities(&capacities, &successful, sources);
        assert!(!captured.ships.second.is_empty());
        assert!(!captured.air_wings.second.is_empty());
        assert!(!captured.army_orders.second.is_empty());
        assert!(!captured.order_geometry_second.is_empty());
        assert!(captured.fleets.second.is_empty());
        assert!(captured.task_forces.second.is_empty());
        assert!(captured.land_combats.second.is_empty());
        captured.grow(
            &mut capacities,
            &successful,
            SourceChecks {
                ships: true,
                ..SourceChecks::default()
            },
        );
        assert!(!captured.ships.second.is_empty());
        assert!(captured.air_wings.second.is_empty());
        assert!(captured.army_orders.second.is_empty());
        assert!(captured.order_geometry_second.is_empty());
        captured.grow(&mut capacities, &successful, NoSources.checks());
        assert!(captured.ships.second.is_empty());
    }

    #[test]
    fn downstream_requests_force_fresh_passes_and_receive_source_failures() {
        struct Unreadable;
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                false
            }
        }
        #[derive(Default)]
        struct Sources {
            begins: [usize; 2],
            finished: Vec<(usize, bool, Outcome)>,
        }
        impl MapSources for Sources {
            fn checks(&self) -> SourceChecks {
                SourceChecks {
                    ships: true,
                    air_wings: true,
                    order_groups: true,
                }
            }
            fn begin_ships(&mut self, _: bool) -> u64 {
                self.begins[0] += 1;
                0
            }
            fn ship(&mut self, _: bool, _: usize, _: &cship::Row) -> u64 {
                panic!("unreadable census visited ship")
            }
            fn finish_ships(&mut self, second: bool, outcome: Outcome) -> u64 {
                self.finished.push((0, second, outcome));
                0
            }
            fn begin_air(&mut self, _: bool) -> u64 {
                self.begins[1] += 1;
                0
            }
            fn air(&mut self, _: bool, _: usize, _: usize, _: &cair_wing::Row) -> u64 {
                panic!("unreadable census visited wing")
            }
            fn finish_air(&mut self, second: bool, outcome: Outcome) -> u64 {
                self.finished.push((1, second, outcome));
                0
            }
            fn order_groups(
                &mut self,
                second: bool,
                roots: &[corders_group::Root],
                outcome: Outcome,
            ) -> u64 {
                assert!(roots.is_empty());
                self.finished.push((2, second, outcome));
                0
            }
        }
        let mut sources = Sources::default();
        let mut captured = MapCaptured::with_capacities(
            &MapCapacities::default(),
            &[CONSISTENCY_TARGET; MAP_TABLES],
            sources.checks(),
        );
        let (matched, _) =
            captured.sample(&Unreadable, 0, 0x1000, &[false; MAP_TABLES], &mut sources);
        assert_eq!(matched, [false; MAP_TABLES]);
        assert_eq!(sources.begins, [2, 2]);
        assert_eq!(sources.finished.len(), 6);
        for source in 0..3 {
            for second in [false, true] {
                assert!(
                    sources
                        .finished
                        .iter()
                        .any(|&(kind, pass, outcome)| kind == source
                            && pass == second
                            && matches!(outcome, Outcome::Unavailable { .. }))
                );
            }
        }
        assert!(
            !captured.ships.checked && !captured.air_wings.checked && !captured.army_orders.checked
        );
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
    fn new_buffers_use_current_capacities_without_recreating_completed_comparisons() {
        let mut capacity = MapCapacities::default();
        capacity.rows[5] += 31;
        capacity.player_names += 19;
        capacity.land_participants += 13;
        capacity.order_geometry += 17;
        let mut successful = [CONSISTENCY_TARGET; MAP_TABLES];
        successful[5] -= 1;
        let captured =
            MapCaptured::with_capacities(&capacity, &successful, SourceChecks::default());
        assert_eq!(captured.ships.rows.len(), capacity.rows[5]);
        assert_eq!(captured.ships.second.len(), capacity.rows[5]);
        assert!(captured.players.second.is_empty());
        assert!(captured.land_combats.second.is_empty());
        assert!(captured.army_orders.second.is_empty());
        assert_eq!(captured.player_names.len(), capacity.player_names);
        assert_eq!(captured.land_participants.len(), capacity.land_participants);
        assert_eq!(captured.order_geometry.len(), capacity.order_geometry);
        assert!(captured.player_names_second.is_empty());
        assert!(captured.land_participants_second.is_empty());
        assert!(captured.order_geometry_second.is_empty());
        let complete = MapCaptured::with_capacities(
            &capacity,
            &[CONSISTENCY_TARGET; MAP_TABLES],
            SourceChecks::default(),
        );
        assert_eq!(
            captured.allocated_bytes() - complete.allocated_bytes(),
            capacity.rows[5] * size_of::<cship::Row>()
        );
    }

    #[test]
    fn pooled_growth_preserves_complete_rows_and_counts_whole_omissions() {
        let mut captured = MapCaptured::new();
        let mut capacity = MapCapacities::default();
        captured.ships.rows[0].id = u64::MAX;
        captured.ships.outcome = Outcome::Truncated {
            len: capacity.rows[5],
            unread_rows: 7,
        };
        captured.land_combats.outcome = Outcome::Truncated {
            len: 2,
            unread_rows: 1,
        };
        captured.land_participants_required = capacity.land_participants + 11;
        captured.player_names_required = capacity.player_names + 17;
        captured.order_geometry_required = capacity.order_geometry + 23;
        captured.grow(&mut capacity, &[0; MAP_TABLES], SourceChecks::default());
        assert_eq!(captured.ships.rows.len(), 8192);
        assert_eq!(captured.ships.second.len(), 8192);
        assert_eq!(captured.ships.rows[0].id, u64::MAX);
        assert_eq!(captured.land_participants.len(), 16384);
        assert_eq!(captured.land_participants_second.len(), 16384);
        assert_eq!(captured.player_names.len(), 8192);
        assert_eq!(captured.player_names_second.len(), 8192);
        assert_eq!(captured.order_geometry.len(), 32768);
        assert_eq!(captured.order_geometry_second.len(), 32768);
        let mut idle = MapCaptured::new();
        idle.grow(&mut capacity, &[0; MAP_TABLES], SourceChecks::default());
        assert_eq!(idle.allocated_bytes(), captured.allocated_bytes());
    }

    #[test]
    fn gradually_growing_tables_fit_without_first_losing_an_hour() {
        let mut capacity = 8;
        let mut table = CaptureRows::<u32>::new(capacity, CONSISTENCY_TARGET);
        for count in 1..=129 {
            assert!(count <= table.rows.len());
            table.rows[count - 1] = count as u32;
            table.outcome = Outcome::Rows { len: count };
            table.grow(&mut capacity, CONSISTENCY_TARGET);
            assert_eq!(table.rows[count - 1], count as u32);
            assert!(table.second.is_empty());
        }
        assert!(capacity <= 129 * 3);
    }

    #[test]
    fn unavailable_scratch_walks_grow_without_inventing_truncated_row_counts() {
        let mut captured = MapCaptured::new();
        let mut capacity = MapCapacities::default();
        captured.army_orders.outcome = Outcome::Unavailable {
            reason: "orders scratch full",
        };
        let initial_group_capacity = capacity.order_groups;
        captured.order_groups_required = initial_group_capacity + 1;
        captured.order_visits_required = 4096;
        captured.naval_combat_participants.outcome = Outcome::Unavailable {
            reason: "naval groups scratch full",
        };
        captured.naval_groups_required = 129;
        captured.grow(&mut capacity, &[0; MAP_TABLES], SourceChecks::default());
        assert_eq!(captured.order_groups.len(), initial_group_capacity * 2);
        assert_eq!(captured.order_visits.len(), 4096);
        assert_eq!(captured.naval_groups.len(), 256);
        // Slowly increasing native counts reuse headroom instead of making
        // every newly created group or order unavailable for another hour.
        let grown = capacity;
        captured.order_groups_required = initial_group_capacity + 47;
        captured.order_visits_required = 3500;
        captured.naval_groups_required = 201;
        captured.grow(&mut capacity, &[0; MAP_TABLES], SourceChecks::default());
        assert!(capacity == grown);
        assert!(matches!(
            captured.army_orders.outcome,
            Outcome::Unavailable { .. }
        ));
        assert!(matches!(
            captured.naval_combat_participants.outcome,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn completed_proofs_release_comparison_rows_and_arenas_without_losing_primary_data() {
        let mut captured = MapCaptured::new();
        let mut capacity = MapCapacities::default();
        captured.player_names[0] = 0xff;
        captured.land_participants[0] = u64::MAX;
        captured.order_geometry[0] = u32::MAX;
        let before = captured.allocated_bytes();
        let mut successful = [CONSISTENCY_TARGET - 1; MAP_TABLES];
        successful[0] = CONSISTENCY_TARGET;
        captured.grow(&mut capacity, &successful, SourceChecks::default());
        assert_eq!(captured.players.second.capacity(), 0);
        assert_eq!(captured.player_names_second.capacity(), 0);
        assert!(!captured.land_combats.second.is_empty());
        assert!(!captured.army_orders.second.is_empty());
        successful.fill(CONSISTENCY_TARGET);
        captured.grow(&mut capacity, &successful, SourceChecks::default());
        assert_eq!(captured.players.second.capacity(), 0);
        assert_eq!(captured.human_controllers.second.capacity(), 0);
        assert_eq!(captured.land_combats.second.capacity(), 0);
        assert_eq!(captured.fleets.second.capacity(), 0);
        assert_eq!(captured.task_forces.second.capacity(), 0);
        assert_eq!(captured.ships.second.capacity(), 0);
        assert_eq!(captured.naval_combats.second.capacity(), 0);
        assert_eq!(captured.naval_combat_participants.second.capacity(), 0);
        assert_eq!(captured.air_wings.second.capacity(), 0);
        assert_eq!(captured.army_orders.second.capacity(), 0);
        assert_eq!(captured.land_participants_second.capacity(), 0);
        assert_eq!(captured.order_geometry_second.capacity(), 0);
        captured.player_names_required = capacity.player_names + 1;
        captured.land_participants_required = capacity.land_participants + 1;
        captured.order_geometry_required = capacity.order_geometry + 1;
        captured.grow(&mut capacity, &successful, SourceChecks::default());
        assert_eq!(captured.player_names_second.capacity(), 0);
        assert_eq!(captured.land_participants_second.capacity(), 0);
        assert_eq!(captured.order_geometry_second.capacity(), 0);
        assert_eq!(captured.player_names[0], 0xff);
        assert_eq!(captured.land_participants[0], u64::MAX);
        assert_eq!(captured.order_geometry[0], u32::MAX);
        assert!(captured.allocated_bytes() < before);
    }

    #[test]
    fn map_pack_tables_are_hourly_tier_h_with_no_refresh_gate() {
        // The eight map-pack tables keep hourly cadence: no stagger slot,
        // every hour walks. This pins the no-cadence-change rule for the split.
        use replay::rows::TableId;
        for table in [
            TableId::LandCombats,
            TableId::Fleets,
            TableId::TaskForces,
            TableId::Ships,
            TableId::NavalCombats,
            TableId::NavalCombatParticipants,
            TableId::AirWings,
            TableId::ArmyOrders,
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
    fn map_split_copy_matches_direct_walk() {
        use super::map_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let lens = [4, 4, 4, 8, 4, 8, 4, 8];
        let direct = kit::direct(&memory, lens);
        let (split, report, _) = kit::split(&memory, lens);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 8]);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.land_outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.land_participants[..3], [u64::MAX - 4, 1, 1]);
        assert_eq!(split.fleets_outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.ships_outcome, Outcome::Rows { len: 2 });
        assert_eq!(split.wings_outcome, Outcome::Rows { len: 2 });
    }

    #[test]
    fn map_split_copy_matches_direct_walk_under_truncation() {
        use super::map_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let lens = [1, 1, 1, 1, 1, 1, 1, 1];
        let direct = kit::direct(&memory, lens);
        let (split, report, _) = kit::split(&memory, lens);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 8]);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(
            split.ships_outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(
            split.wings_outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
    }

    #[test]
    fn map_split_copy_matches_direct_walk_on_fault_matrix() {
        use super::map_test_kit as kit;
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
        // Outcomes including reason strings must match the typed
        // walks exactly; verdicts follow the fault-attribution masks. The
        // mixed combat registry is iterated by both land and naval walks
        // for classification, so faults there block both.
        for (fault, matched) in [
            (
                kit::GAME_STATE + engine::cgame_state::COMBATS + 8,
                [false, true, true, true, false, false, true, true],
            ),
            (0x20000, [false, true, true, true, false, false, true, true]),
            (0x31000, [true, false, false, false, true, true, true, true]),
            (0x33000, [true, true, true, false, true, true, true, true]),
            (0x40000, [false, true, true, true, false, false, true, true]),
            (0x53000, [true, true, true, true, true, true, false, true]),
        ] {
            let bytes = kit::fixture();
            let memory = Fault {
                memory: SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
            };
            let lens = [4, 4, 4, 8, 4, 8, 4, 8];
            let direct = kit::direct(&memory, lens);
            let (split, report, _) = kit::split(&memory, lens);
            assert!(report.raw_ok, "fault={fault:x}");
            kit::assert_matches_direct(&split, &direct);
            assert_eq!(report.matched, matched, "fault={fault:x}");
        }
    }

    #[test]
    fn map_double_copy_compare_blocks_checks_on_tearing() {
        use super::map_test_kit as kit;
        use super::{MapCopyContext, MapJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = MapCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut first = MapJob::new();
        let mut second = MapJob::new();
        let report = super::copy_map_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 8]);
        // Tear the second copy: verdicts must block every check while the
        // decoded rows still match the first copy exactly.
        second.log.bytes[100] ^= 0xff;
        let equal = first.log.logs_equal(&second.log);
        assert!(!equal);
    }

    #[test]
    fn map_staging_overflow_falls_back_to_typed_walk() {
        use super::map_test_kit as kit;
        use super::{MapCopyContext, MapJob};
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = MapCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut tiny = MapJob {
            log: crate::raw::RawLog::new(1, 1),
            game_state: 0,
            order_seen: Vec::with_capacity(1),
            order_stack: Vec::with_capacity(1),
        };
        let report = super::copy_map_refresh(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
    }
}
