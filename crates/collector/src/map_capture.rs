//! Pooled map-table reads. Only the spool thread grows or releases allocations.
//!
//! Combat, fleet, wing and order censuses fill typed pooled rows on the callback.
//! Dependent Air/Naval consumers run only when their own raw payload did not stage;
//! the Players/HumanControllers pair retains its independent worker split.

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
    pub land_combats: bool,
    pub ships: bool,
    pub air_wings: bool,
    pub order_groups: bool,
}

impl SourceChecks {
    fn retained(self, successful: &[u16]) -> [u16; MAP_TABLES] {
        let mut retained: [u16; MAP_TABLES] = successful.try_into().expect("map comparison count");
        for (index, needed) in [
            (2, self.land_combats),
            (5, self.ships),
            (8, self.air_wings),
            (9, self.order_groups),
        ] {
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
    fn begin_land(&mut self, _second: bool) -> u64 {
        0
    }
    fn land(&mut self, _second: bool, _source: cland_combat::Source) -> u64 {
        0
    }
    fn finish_land(&mut self, _second: bool, _outcome: Outcome) -> u64 {
        0
    }
    fn begin_ships(&mut self, second: bool) -> u64;
    fn ship(&mut self, second: bool, pointer: usize, row: &cship::Row) -> u64;
    fn finish_ships(&mut self, second: bool, outcome: Outcome) -> u64;
    fn begin_air(&mut self, second: bool) -> u64;
    fn air(&mut self, second: bool, pointer: usize, pool: usize, row: &cair_wing::Row) -> u64;
    fn finish_air(&mut self, second: bool, outcome: Outcome) -> u64;
    fn plans(
        &mut self,
        _second: bool,
        _roots: &[corders_group::Root],
        _visits: &[corder_instance::Visit],
        _root_source: Outcome,
        _order_source: Outcome,
    ) -> u64 {
        0
    }
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
    /// Raw-copy split gate for the hourly players pair (Players,
    /// HumanControllers). Set by the game thread every hour with a valid raw
    /// payload; `sample` then skips both typed walks and the decode worker
    /// fills these rows before the ready handoff. Preserved across `reset`
    /// via explicit clearing (set per hour, cleared by the worker).
    pub(crate) players_raw_mode: bool,
    /// Current callback's shared complete wing source already installed this
    /// independently bounded map projection; never a cross-hour cache.
    owned_air_match: Option<bool>,
    /// An accepted wing acquisition failed this callback. Retain its terminal
    /// outcome until reset so neither the map nor its visitors retry the source.
    air_capture_failure: Option<&'static str>,
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
    pub(crate) fn install_owned_air(&mut self, job: &crate::extended::air::AirWingJob) -> u64 {
        if self.air_capture_failure.is_some() {
            return 0;
        }
        let begin = qpc();
        let len = job.map_outcome.len();
        self.air_wings.rows[..len].copy_from_slice(&job.map_rows[..len]);
        self.air_wings.outcome = job.map_outcome;
        self.owned_air_match = Some(job.map_matched);
        qpc().saturating_sub(begin).max(0) as u64
    }

    /// Call before `sample` after an accepted wing acquisition fails. The
    /// terminal gate survives sampling and clears only at the next frame reset.
    pub(crate) fn fail_air_capture(&mut self, needed: bool, reason: &'static str) {
        self.air_capture_failure = Some(reason);
        self.owned_air_match = None;
        self.air_wings.outcome = Outcome::Unavailable { reason };
        self.air_wings.checked = needed;
        self.air_wings.mismatches = usize::from(needed);
    }

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
            owned_air_match: None,
            air_capture_failure: None,
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
            players_raw_mode: false,
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
        self.owned_air_match = None;
        self.air_capture_failure = None;
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
        self.players_raw_mode = false;
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
        // Raw-copy split (hourly players pair): every hour the game thread
        // stages raw bytes via `copy_players_refresh` and the decode worker
        // fills both rows before the ready handoff. Skip both typed walks
        // here; matched flags and costs install from the copy report in the
        // tick. The eight typed map walks below run independently.
        if !self.players_raw_mode {
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
                let second = chuman::walk_controllers(
                    memory,
                    base,
                    state,
                    &mut self.human_controllers.second,
                );
                matched[1] = self.human_controllers.compare(second);
            }
            ticks[1] = (qpc() - begin).max(0) as u64;
        }

        let begin = qpc();
        let mut nested = sources.begin_land(false);
        let land = cland_combat::walk_with(
            memory,
            base,
            state,
            &mut self.land_combats.rows,
            &mut self.land_participants,
            |source| nested = nested.saturating_add(sources.land(false, source)),
        );
        nested = nested.saturating_add(sources.finish_land(false, land.outcome));
        self.land_combats.outcome = land.outcome;
        self.land_participants_len = land.participants_len;
        self.land_participants_required = land.required_participants;
        if needed[2] || source_checks.land_combats {
            nested = nested.saturating_add(sources.begin_land(true));
            let second = cland_combat::walk_with(
                memory,
                base,
                state,
                &mut self.land_combats.second,
                &mut self.land_participants_second,
                |source| nested = nested.saturating_add(sources.land(true, source)),
            );
            nested = nested.saturating_add(sources.finish_land(true, second.outcome));
            if needed[2] {
                self.land_combats.compare(second.outcome);
                self.land_combats.mismatches += usize::from(
                    land.participants_len != second.participants_len
                        || land.required_participants != second.required_participants
                        || self.land_participants[..land.participants_len]
                            != self.land_participants_second[..second.participants_len],
                );
                matched[2] = full_match(land.outcome, second.outcome, self.land_combats.mismatches);
            }
            self.land_participants_required = self
                .land_participants_required
                .max(second.required_participants);
        }
        ticks[2] = ((qpc() - begin).max(0) as u64).saturating_sub(nested);

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
        if let Some(reason) = self.air_capture_failure {
            self.air_wings.outcome = Outcome::Unavailable { reason };
            self.air_wings.mismatches = usize::from(needed[8]);
        } else if let Some(equal) = self.owned_air_match {
            matched[8] = equal;
            self.air_wings.mismatches = usize::from(needed[8] && !equal);
        } else {
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
        }

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
        nested = nested.saturating_add(sources.plans(
            false,
            &self.order_groups[..orders.root_outcome.len()],
            &self.order_visits,
            orders.root_outcome,
            orders.outcome,
        ));
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
            nested = nested.saturating_add(sources.plans(
                true,
                &self.order_groups[..second.root_outcome.len()],
                &self.order_visits,
                second.root_outcome,
                second.outcome,
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

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

// ===========================================================================
// Hourly players raw-copy split (Players, HumanControllers).
//
// The game thread copies raw bytes only: live roster header with per-player
// identity/flag/string spans plus heap name bytes, and the human-status
// array with an authenticated country census plus per-canonical count bytes.
// No row is decoded on the game thread. The single FIFO decode worker replays
// the existing `chuman::walk` and `walk_controllers` against the recorded
// spans (`crate::raw::Recorded`), so rows, ordinals, arenas, truncation and
// failure reasons are identical by construction (see `crate::raw`).
//
// The mirror traverses structurally and records every discoverable span,
// never aborting except on overflow: validation, precedence and error scope
// are local parses, so the replay derives every outcome from the same
// recorded spans and later extras are harmless.
// ===========================================================================

/// Fault-attribution bits, one per players table in family order.
const P_PLAYERS: u32 = 0x01;
const P_CONTROLLERS: u32 = 0x02;

/// One hour of raw players payload. Flat pools recycle capacities and the
/// steady state allocates on neither thread; growth happens on the decode
/// side only (see `crate::decode::recycle_players_job`).
#[derive(Debug, Default)]
pub(crate) struct PlayersJob {
    pub log: RawLog<2>,
    pub game_state: usize,
}

impl PlayersJob {
    pub fn new() -> Self {
        Self {
            // One player with two names plus 439 controllers with a country
            // census; both pools start above that so steady state never
            // overflows.
            log: RawLog::new(1 << 15, 4 << 20),
            game_state: 0,
        }
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    /// Double every pool off the game thread until copies fit. Converges like
    /// the typed capacity growth; overflowed hours fall back to the typed
    /// walk in the meantime, so sizing is a pure performance knob.
    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PlayersCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    /// Whole-log raw equality gated by live-fault conservatism, in
    /// [players, controllers] order.
    pub matched: [bool; 2],
    pub copy_ticks: [u64; 2],
    pub raw_bytes: [u64; 2],
}

/// Game-thread copy parameters, bundled to keep the tick entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlayersCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; 2],
}

fn ppool_u64(log: &RawLog<2>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn ppool_u32(log: &RawLog<2>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

/// One mirrored span read. Bulk spans serve enclosed sub-reads (e.g., 4-byte
/// keys inside roster elements) via `Recorded` containment.
fn pspan(
    log: &mut RawLog<2>,
    memory: &impl engine::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take_bulk(memory, addr?, len, mask)
}

/// Heap payload and terminator takes for one parsed player string. Inline
/// strings ride in the already-copied header span.
fn copy_player_string(
    log: &mut RawLog<2>,
    memory: &impl engine::Memory,
    len: usize,
    source: usize,
    inline: bool,
) {
    if inline {
        return;
    }
    if len != 0 {
        let _ = pspan(log, memory, Some(source), len, P_PLAYERS);
    }
    let _ = pspan(log, memory, source.checked_add(len), 1, P_PLAYERS);
}

/// Mirror of one player's read set: vtable, key, country, flags, 64-byte
/// string headers with heap payloads. Takes are unconditional; the replay
/// validates vtables, keys, duplicate keys and string headers itself.
fn copy_player(log: &mut RawLog<2>, memory: &impl engine::Memory, pointer: usize) {
    use engine::{chuman, msvc_string};
    let _ = pspan(log, memory, Some(pointer), 8, P_PLAYERS);
    let _ = pspan(
        log,
        memory,
        pointer.checked_add(chuman::PLAYER_KEY),
        4,
        P_PLAYERS,
    );
    let _ = pspan(
        log,
        memory,
        pointer.checked_add(chuman::COUNTRY_TAG),
        4,
        P_PLAYERS,
    );
    let _ = pspan(
        log,
        memory,
        pointer.checked_add(chuman::FLAGS),
        1,
        P_PLAYERS,
    );
    let strings = match pspan(
        log,
        memory,
        pointer.checked_add(chuman::FIRST_STRING),
        64,
        P_PLAYERS,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut headers = [0u8; 64];
    headers.copy_from_slice(&log.bytes[strings..strings + 64]);
    for chunk in [&headers[..32], &headers[32..]] {
        let len = u64::from_le_bytes(chunk[16..24].try_into().unwrap());
        let capacity = u64::from_le_bytes(chunk[24..32].try_into().unwrap());
        if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
            continue;
        }
        let Ok(len) = usize::try_from(len) else {
            continue;
        };
        let inline = capacity == msvc_string::INLINE_CAPACITY;
        if inline {
            continue;
        }
        let source = match usize::try_from(u64::from_le_bytes(chunk[..8].try_into().unwrap()))
            .ok()
            .filter(|&s| s != 0)
        {
            Some(s) => s,
            None => continue,
        };
        if source.checked_add(len).is_none() {
            continue;
        }
        copy_player_string(log, memory, len, source, false);
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of one link-table read for the controller census with local bounds
/// checks. Returns the resolved slot exactly when the typed resolve
/// succeeds; the replay derives its own reason from the same recorded span.
fn controller_link(
    log: &mut RawLog<2>,
    memory: &impl engine::Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<u32> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = pspan(
        log,
        memory,
        links.data.checked_add(offset),
        4,
        P_CONTROLLERS,
    )?;
    let slot = ppool_u32(log, start);
    if slot == 0 || slot >= countries {
        return None;
    }
    Some(slot)
}

/// One structural pass: roster with players, then country census with
/// controller counts. Stops at the first typed abort point; later extras
/// are harmless.
fn copy_players_pass(
    log: &mut RawLog<2>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
) {
    use engine::{ccountry, cgame_state, chuman, cpdx_array::Array};
    let _ = base;
    log.has_bulk = true;
    // Live roster.
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
    for index in 0..roster.count {
        let addr = roster
            .data
            .checked_add((index as usize).saturating_mul(chuman::STRIDE));
        // Roster elements are inline CHuman objects, not pointers.
        if let Some(addr) = addr {
            // Probe the vtable to discover live versus padding slots; the
            // replay authenticates the same span and skips padding itself.
            // Takes run unconditionally for every slot (a superset).
            let vtable = pspan(log, memory, Some(addr), 8, P_PLAYERS);
            if vtable.is_none() {
                continue;
            }
            copy_player(log, memory, addr);
            if log.overflowed {
                return;
            }
        }
        if log.overflowed {
            return;
        }
    }
    // Human-status array header plus country census for controllers.
    let counts_start = match pspan(
        log,
        memory,
        game_state.checked_add(cgame_state::HUMAN_STATUS),
        16,
        P_CONTROLLERS,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut counts_raw = [0u8; 16];
    counts_raw.copy_from_slice(&log.bytes[counts_start..counts_start + 16]);
    let counts = match Array::from_header(counts_raw, 1) {
        Some(a) => a,
        None => return,
    };
    let countries_start = match pspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        P_CONTROLLERS,
    ) {
        Some(start) => start,
        None => return,
    };
    let links_start = match pspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        P_CONTROLLERS,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut countries_raw = [0u8; 16];
    countries_raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let mut links_raw = [0u8; 16];
    links_raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let countries_array = match Array::from_header(countries_raw, 8) {
        Some(a) => a,
        None => return,
    };
    let links_array = match Array::from_header(links_raw, 4) {
        Some(a) => a,
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
        let ptr_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|o| countries_array.data.checked_add(o));
        let pointer = match pspan(log, memory, ptr_addr, 8, P_CONTROLLERS)
            .and_then(|s| usize::try_from(ppool_u64(log, s)).ok().filter(|&p| p != 0))
        {
            Some(p) => p,
            None => continue,
        };
        let vtable = match pspan(log, memory, Some(pointer), 8, P_CONTROLLERS) {
            Some(s) => s,
            None => continue,
        };
        if ppool_u64(log, vtable) != country_vtable {
            return;
        }
        let id = match pspan(
            log,
            memory,
            pointer.checked_add(ccountry::ID),
            4,
            P_CONTROLLERS,
        ) {
            Some(s) => ppool_u32(log, s),
            None => continue,
        };
        let canonical = match controller_link(log, memory, &links_array, countries_array.count, id)
        {
            Some(c) => c,
            None => return,
        };
        if controller_link(log, memory, &links_array, countries_array.count, slot)
            != Some(canonical)
        {
            return;
        }
        if controller_link(log, memory, &links_array, countries_array.count, canonical)
            != Some(canonical)
        {
            return;
        }
        let canon_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|o| countries_array.data.checked_add(o));
        let canon_ptr = match pspan(log, memory, canon_addr, 8, P_CONTROLLERS) {
            Some(s) => s,
            None => continue,
        };
        if ppool_u64(log, canon_ptr) != pointer as u64 {
            return;
        }
        // Controller count byte for the canonical slot. Null slots publish
        // no row; the replay derives the same skip from the census.
        if slot == canonical {
            let count_addr = counts.data.checked_add(canonical as usize);
            let _ = pspan(log, memory, count_addr, 1, P_CONTROLLERS);
        }
        if log.overflowed {
            return;
        }
    }
}

/// Raw copy for an hourly refresh. Copies `first` always and `second` while
/// either players table still needs double-read checks, compares the per-table
/// raw partitions, and records copy-only costs. Returns whether the payload is
/// complete (`raw_ok`); on staging overflow the caller falls back to the typed
/// walk and enqueues nothing.
pub(crate) fn copy_players_refresh(
    memory: &impl engine::Memory,
    ctx: PlayersCopyContext,
    first: &mut PlayersJob,
    mut second: Option<&mut PlayersJob>,
) -> PlayersCopyReport {
    let started = qpc();
    first.clear();
    first.game_state = ctx.game_state;
    copy_players_pass(&mut first.log, memory, ctx.base, ctx.game_state);
    let mut ticks = [0u64; 2];
    ticks[0] = elapsed(started);
    let mut matched = [true; 2];
    if let Some(second_job) = second.as_deref_mut() {
        let second_started = qpc();
        second_job.clear();
        second_job.game_state = ctx.game_state;
        copy_players_pass(&mut second_job.log, memory, ctx.base, ctx.game_state);
        ticks[0] += elapsed(second_started);
        let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
        for (slot, matched) in matched.iter_mut().enumerate() {
            let equal = raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
            *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
        }
    }
    let raw_ok = !first.log.overflowed && second.is_none_or(|job| !job.log.overflowed);
    PlayersCopyReport {
        raw_ok,
        matched,
        copy_ticks: ticks,
        raw_bytes: first.log.raw_bytes,
    }
}

impl MapCaptured {
    pub(crate) fn apply_players_report(&mut self, needed: [bool; 2], report: &PlayersCopyReport) {
        self.players.checked = needed[0];
        // Binary per refresh: tearing detected (0/1), not a row count.
        // Spool gates test `==0`/`!=0` only.
        self.players.mismatches = usize::from(needed[0] && !report.matched[0]);
        self.human_controllers.checked = needed[1];
        self.human_controllers.mismatches = usize::from(needed[1] && !report.matched[1]);
    }

    /// Apply a failed worker capture after `sample` has skipped the players
    /// walks with `players_raw_mode` set, preventing a retry on the callback.
    pub(crate) fn fail_players_capture(&mut self, needed: [bool; 2], reason: &'static str) {
        self.players.outcome = Outcome::Unavailable { reason };
        self.players.checked = needed[0];
        self.players.mismatches = usize::from(needed[0]);
        self.human_controllers.outcome = Outcome::Unavailable { reason };
        self.human_controllers.checked = needed[1];
        self.human_controllers.mismatches = usize::from(needed[1]);
        self.player_names_len = 0;
        self.players_raw_mode = false;
    }

    /// Decode-worker interpretation of a raw hourly payload into the pooled
    /// typed rows. Outcomes and arena bytes match the typed walks exactly
    /// because the identical walks run against the recorded spans;
    /// checked/mismatches set by the raw compare survive. Clears
    /// `players_raw_mode` so the next capture starts clean.
    pub(crate) fn decode_players_payload(
        &mut self,
        base: usize,
        job: &PlayersJob,
        decode_ticks: &mut [u64; 2],
    ) {
        let recorded = crate::raw::Recorded::build(&job.log);
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

/// Hourly players split harness: one country with one inline player and one
/// controller count, plus empty and fault-injection coverage.
#[cfg(test)]
pub(crate) mod players_split_tests {
    use super::SourceChecks;
    use super::{
        MapCapacities, MapCaptured, PlayersCopyContext, PlayersCopyReport, PlayersJob,
        copy_players_refresh,
    };
    use engine::{ccountry, cgame_state, chuman, memory::SliceMemory, walk::Outcome};

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
        let mut bytes = vec![0; 0x20000];
        // Country census: 1 canonical country (slot1, id1).
        array(&mut bytes, GS + cgame_state::COUNTRIES, COUNTRY_PTR, 2);
        put(&mut bytes, COUNTRY_PTR + 8, &(COUNTRY as u64).to_le_bytes());
        array(&mut bytes, GS + cgame_state::COUNTRY_LINKS, LINKS, 2);
        put(&mut bytes, LINKS + 4, &1_u32.to_le_bytes());
        put(
            &mut bytes,
            COUNTRY,
            &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
        array(&mut bytes, GS + cgame_state::HUMAN_STATUS, COUNTS, 2);
        put(&mut bytes, COUNTS + 1, &[7]);
        // One player with inline names.
        array(&mut bytes, GS + cgame_state::PLAYER_RECORDS, ROSTER, 1);
        put(
            &mut bytes,
            ROSTER,
            &(chuman::VTABLE_RVA as u64).to_le_bytes(),
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
        let mut captured = MapCaptured::with_capacities(
            &capacities(),
            &[0; super::MAP_TABLES],
            SourceChecks::default(),
        );
        // Walk only players/controllers via sample with empty sources.
        struct NoSources;
        impl super::MapSources for NoSources {
            fn checks(&self) -> super::SourceChecks {
                super::SourceChecks::default()
            }
            fn begin_ships(&mut self, _: bool) -> u64 {
                0
            }
            fn ship(&mut self, _: bool, _: usize, _: &engine::cship::Row) -> u64 {
                0
            }
            fn finish_ships(&mut self, _: bool, _: Outcome) -> u64 {
                0
            }
            fn begin_air(&mut self, _: bool) -> u64 {
                0
            }
            fn air(&mut self, _: bool, _: usize, _: usize, _: &engine::cair_wing::Row) -> u64 {
                0
            }
            fn finish_air(&mut self, _: bool, _: Outcome) -> u64 {
                0
            }
            fn order_groups(
                &mut self,
                _: bool,
                _: &[engine::corders_group::Root],
                _: Outcome,
            ) -> u64 {
                0
            }
        }
        let needed = [
            true, true, false, false, false, false, false, false, false, false,
        ];
        let (matched, _) = captured.sample(memory, BASE, GS, &needed, &mut NoSources);
        assert!(matched[0] && matched[1]);
        captured
    }

    fn split(memory: &SliceMemory<'_>) -> (MapCaptured, PlayersCopyReport) {
        let mut captured = MapCaptured::with_capacities(
            &capacities(),
            &[0; super::MAP_TABLES],
            SourceChecks::default(),
        );
        let ctx = PlayersCopyContext {
            base: BASE,
            game_state: GS,
            needed: [true, true],
        };
        let mut first = PlayersJob::new();
        let mut second = PlayersJob::new();
        let report = copy_players_refresh(memory, ctx, &mut first, Some(&mut second));
        captured.apply_players_report(ctx.needed, &report);
        assert!(report.raw_ok);
        captured.players_raw_mode = true;
        let mut ticks = [0u64; 2];
        captured.decode_players_payload(BASE, &first, &mut ticks);
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
        assert_eq!(report.matched, [true, true]);
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
    fn players_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/one/exact-fit/overflow-by-one/massive run typed AND
        // copy→decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        // Fixture holds 1 player and 1 controller census row, so cap 1 is
        // both the one-row and exact-fit arm, cap 0 the zero/overflow
        // (total 1 == cap+1)/massive arm. u32 unread saturation is not
        // reachable and is covered by the walk's own saturating arithmetic.
        struct NoSources;
        impl super::MapSources for NoSources {
            fn checks(&self) -> super::SourceChecks {
                super::SourceChecks::default()
            }
            fn begin_ships(&mut self, _: bool) -> u64 {
                0
            }
            fn ship(&mut self, _: bool, _: usize, _: &engine::cship::Row) -> u64 {
                0
            }
            fn finish_ships(&mut self, _: bool, _: Outcome) -> u64 {
                0
            }
            fn begin_air(&mut self, _: bool) -> u64 {
                0
            }
            fn air(&mut self, _: bool, _: usize, _: usize, _: &engine::cair_wing::Row) -> u64 {
                0
            }
            fn finish_air(&mut self, _: bool, _: Outcome) -> u64 {
                0
            }
            fn order_groups(
                &mut self,
                _: bool,
                _: &[engine::corders_group::Root],
                _: Outcome,
            ) -> u64 {
                0
            }
        }
        let bytes = fixture();
        let memory = SliceMemory {
            base: GS,
            bytes: &bytes,
        };
        let needed = [
            true, true, false, false, false, false, false, false, false, false,
        ];
        for (players, controllers) in [(0, 0), (1, 1), (0, 1), (1, 0), (64, 64)] {
            let mut caps = capacities();
            caps.rows[0] = players;
            caps.rows[1] = controllers;
            let mut direct = MapCaptured::with_capacities(
                &caps,
                &[0; super::MAP_TABLES],
                SourceChecks::default(),
            );
            let _ = direct.sample(&memory, BASE, GS, &needed, &mut NoSources);
            let mut captured = MapCaptured::with_capacities(
                &caps,
                &[0; super::MAP_TABLES],
                SourceChecks::default(),
            );
            let ctx = PlayersCopyContext {
                base: BASE,
                game_state: GS,
                needed: [true, true],
            };
            let mut first = PlayersJob::new();
            let mut second = PlayersJob::new();
            let report = copy_players_refresh(&memory, ctx, &mut first, Some(&mut second));
            captured.apply_players_report(ctx.needed, &report);
            assert!(report.raw_ok, "players={players} controllers={controllers}");
            captured.players_raw_mode = true;
            let mut ticks = [0u64; 2];
            captured.decode_players_payload(BASE, &first, &mut ticks);
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
        // Exact-fit arm pins Rows on both tables.
        let mut caps = capacities();
        caps.rows[0] = 1;
        caps.rows[1] = 1;
        let mut direct =
            MapCaptured::with_capacities(&caps, &[0; super::MAP_TABLES], SourceChecks::default());
        let _ = direct.sample(&memory, BASE, GS, &needed, &mut NoSources);
        assert_eq!(direct.players.outcome, Outcome::Rows { len: 1 });
    }

    #[test]
    fn players_bulk_string_pair_serves_enclosed_sub_reads() {
        // S3 containment: the 64 B player string pair is a bulk span, so the
        // walk's enclosed 32 B halves and unaligned sub-reads inside it are
        // served from the same recorded bytes. Proves bulk containment plus
        // bulk-wins-for-sub-ranges; straddling reads fail.
        use engine::Memory;
        let bytes = fixture();
        let memory = SliceMemory {
            base: GS,
            bytes: &bytes,
        };
        let mut job = PlayersJob::new();
        super::copy_players_pass(&mut job.log, &memory, BASE, GS);
        assert!(job.log.has_bulk);
        let recorded = crate::raw::Recorded::build(&job.log);
        let pair = job
            .log
            .spans
            .iter()
            .find(|span| span.ok && span.bulk && span.len == 64)
            .expect("bulk string pair");
        // Enclosed aligned 32 B half.
        let mut half = [0u8; 32];
        assert!(recorded.copy(pair.addr, &mut half));
        let mut expected = [0u8; 32];
        assert!(memory.copy(pair.addr, &mut expected));
        assert_eq!(half, expected);
        // Deliberately unaligned enclosed sub-range.
        let mut unaligned = [0u8; 7];
        assert!(recorded.copy(pair.addr + 3, &mut unaligned));
        let mut expected7 = [0u8; 7];
        assert!(memory.copy(pair.addr + 3, &mut expected7));
        assert_eq!(unaligned, expected7);
        // Straddling the pair edge fails.
        let mut straddle = [0u8; 32];
        assert!(!recorded.copy(pair.addr + 48, &mut straddle));
    }

    #[test]
    fn players_split_copy_matches_direct_walk_under_truncation() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: GS,
            bytes: &bytes,
        };
        let mut capacities = capacities();
        capacities.rows[0] = 0;
        let mut direct = MapCaptured::with_capacities(
            &capacities,
            &[0; super::MAP_TABLES],
            SourceChecks::default(),
        );
        struct NoSources;
        impl super::MapSources for NoSources {
            fn checks(&self) -> super::SourceChecks {
                super::SourceChecks::default()
            }
            fn begin_ships(&mut self, _: bool) -> u64 {
                0
            }
            fn ship(&mut self, _: bool, _: usize, _: &engine::cship::Row) -> u64 {
                0
            }
            fn finish_ships(&mut self, _: bool, _: Outcome) -> u64 {
                0
            }
            fn begin_air(&mut self, _: bool) -> u64 {
                0
            }
            fn air(&mut self, _: bool, _: usize, _: usize, _: &engine::cair_wing::Row) -> u64 {
                0
            }
            fn finish_air(&mut self, _: bool, _: Outcome) -> u64 {
                0
            }
            fn order_groups(
                &mut self,
                _: bool,
                _: &[engine::corders_group::Root],
                _: Outcome,
            ) -> u64 {
                0
            }
        }
        let needed = [
            true, true, false, false, false, false, false, false, false, false,
        ];
        let (matched, _) = direct.sample(&memory, BASE, GS, &needed, &mut NoSources);
        assert!(!matched[0]);
        let mut captured = MapCaptured::with_capacities(
            &capacities,
            &[0; super::MAP_TABLES],
            SourceChecks::default(),
        );
        let ctx = PlayersCopyContext {
            base: BASE,
            game_state: GS,
            needed: [true, true],
        };
        let mut first = PlayersJob::new();
        let report = copy_players_refresh(&memory, ctx, &mut first, None);
        captured.apply_players_report(ctx.needed, &report);
        assert!(report.raw_ok);
        captured.players_raw_mode = true;
        let mut ticks = [0u64; 2];
        captured.decode_players_payload(BASE, &first, &mut ticks);
        assert_eq!(captured.players.outcome, direct.players.outcome);
    }

    #[test]
    fn players_double_copy_compare_blocks_checks_on_tearing() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: GS,
            bytes: &bytes,
        };
        let mut captured = MapCaptured::with_capacities(
            &capacities(),
            &[0; super::MAP_TABLES],
            SourceChecks::default(),
        );
        let ctx = PlayersCopyContext {
            base: BASE,
            game_state: GS,
            needed: [true, true],
        };
        let mut first = PlayersJob::new();
        let mut second = PlayersJob::new();
        let report = copy_players_refresh(&memory, ctx, &mut first, Some(&mut second));
        captured.apply_players_report(ctx.needed, &report);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true, true]);
        let mut torn = fixture();
        put(
            &mut torn,
            ROSTER + chuman::PLAYER_KEY,
            &99_i32.to_le_bytes(),
        );
        let mut after = PlayersJob::new();
        super::copy_players_pass(&mut after.log, &memory, BASE, GS);
        let mut after_torn = PlayersJob::new();
        super::copy_players_pass(
            &mut after_torn.log,
            &SliceMemory {
                base: GS,
                bytes: &torn,
            },
            BASE,
            GS,
        );
        assert!(!after.log.logs_equal(&after_torn.log));
    }

    #[test]
    fn players_partition_equal_isolates_players_private_bytes() {
        // Tear exactly the players table's private bytes (P_PLAYERS 0x01):
        // only slot 0 flips; controllers (0x02) stay true. The two walks
        // share no census spans, so partitions are fully independent.
        let bytes = fixture();
        let memory = SliceMemory {
            base: GS,
            bytes: &bytes,
        };
        let mut captured = MapCaptured::with_capacities(
            &capacities(),
            &[0; super::MAP_TABLES],
            SourceChecks::default(),
        );
        let ctx = PlayersCopyContext {
            base: BASE,
            game_state: GS,
            needed: [true, true],
        };
        let mut first = PlayersJob::new();
        let mut second = PlayersJob::new();
        let report = copy_players_refresh(&memory, ctx, &mut first, Some(&mut second));
        captured.apply_players_report(ctx.needed, &report);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true, true]);
        let players_idx = second
            .log
            .spans
            .iter()
            .position(|span| span.ok && span.mask == 0x01)
            .expect("players-private span");
        let start = second.log.spans[players_idx].start as usize;
        second.log.bytes[start] ^= 0xff;
        assert!(!first.log.partition_equal(&second.log, 0));
        assert!(first.log.partition_equal(&second.log, 1));
    }

    #[test]
    fn players_staging_overflow_falls_back_to_typed_walk() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: GS,
            bytes: &bytes,
        };
        let mut captured = MapCaptured::with_capacities(
            &capacities(),
            &[0; super::MAP_TABLES],
            SourceChecks::default(),
        );
        let ctx = PlayersCopyContext {
            base: BASE,
            game_state: GS,
            needed: [true, true],
        };
        let mut tiny = PlayersJob {
            log: crate::raw::RawLog::new(8, 8),
            game_state: 0,
        };
        let report = copy_players_refresh(&memory, ctx, &mut tiny, None);
        captured.apply_players_report(ctx.needed, &report);
        assert!(!report.raw_ok);
        assert!(tiny.log.overflowed);
        assert!(tiny.grow_for_next());
    }

    #[test]
    fn players_worst_case_pools_start_above_late_campaign_caps() {
        let job = PlayersJob::new();
        assert!(job.log.spans.capacity() >= 16384);
        assert!(job.log.bytes.capacity() >= (1 << 20));
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
            land_combats: false,
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
            begins: [usize; 3],
            finished: Vec<(usize, bool, Outcome)>,
        }
        impl MapSources for Sources {
            fn checks(&self) -> SourceChecks {
                SourceChecks {
                    ships: true,
                    air_wings: true,
                    order_groups: true,
                    land_combats: true,
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
            fn begin_land(&mut self, _: bool) -> u64 {
                self.begins[2] += 1;
                0
            }
            fn land(&mut self, _: bool, _: cland_combat::Source) -> u64 {
                panic!("unreadable census visited combat")
            }
            fn finish_land(&mut self, second: bool, outcome: Outcome) -> u64 {
                self.finished.push((3, second, outcome));
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
        let memory = Unreadable::default();
        let (matched, _) = captured.sample(&memory, 0, 0x1000, &[false; MAP_TABLES], &mut sources);
        assert_eq!(matched, [false; MAP_TABLES]);
        assert_eq!(sources.begins, [2, 2, 2]);
        assert_eq!(sources.finished.len(), 8);
        for source in 0..4 {
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
        assert!(!captured.land_combats.checked);

        let ordinary_reads = memory.0.get();
        let ordinary_outcomes = captured.outcomes();
        memory.0.set(0);
        cair_wing::walk_with(
            &memory,
            0,
            0x1000,
            &mut captured.air_wings.rows,
            |_, _, _| {},
        );
        let wing_reads = memory.0.get();
        assert!(wing_reads > 0);
        for needed in [false, true] {
            captured.reset();
            let reason = "accepted wing worker failed";
            captured.fail_air_capture(needed, reason);
            memory.0.set(0);
            let mut sources = Sources::default();
            let mut checks = [false; MAP_TABLES];
            checks[8] = needed;
            let (matched, _) = captured.sample(&memory, 0, 0x1000, &checks, &mut sources);
            assert!(!matched[8]);
            assert_eq!(captured.air_wings.outcome, Outcome::Unavailable { reason });
            assert_eq!(captured.air_wings.checked, needed);
            assert_eq!(captured.air_wings.mismatches, usize::from(needed));
            assert_eq!(memory.0.get(), ordinary_reads - 2 * wing_reads);
            assert_eq!(sources.begins, [2, 0, 2]);
            assert_eq!(sources.finished.len(), 6);
            for (index, outcome) in captured.outcomes().into_iter().enumerate() {
                if index != 8 {
                    assert_eq!(outcome, ordinary_outcomes[index]);
                }
            }
        }
        // The terminal decision owns just this callback, never the next one.
        captured.reset();
        memory.0.set(0);
        let mut sources = Sources::default();
        captured.sample(&memory, 0, 0x1000, &[false; MAP_TABLES], &mut sources);
        assert_eq!(memory.0.get(), ordinary_reads);
        assert_eq!(sources.begins, [2, 2, 2]);
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
        // every hour walks. This pins the unchanged typed cadence.
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
    fn typed_map_keeps_its_own_checks_when_air_and_naval_payloads_are_staged() {
        use crate::extended;
        use replay::frame::TABLE_COUNT;
        #[derive(Default)]
        struct Reads(std::cell::RefCell<Vec<(usize, usize)>>);
        impl engine::Memory for Reads {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.0.borrow_mut().push((address, destination.len()));
                false
            }
        }
        let mut extensions = extended::Captured::with_capacities(
            &extended::Capacities::default(),
            &[0; TABLE_COUNT],
        );
        let mut actual = MapCaptured::with_capacities(
            &MapCapacities::default(),
            &[0; MAP_TABLES],
            SourceChecks::default(),
        );
        let mut reference = MapCaptured::with_capacities(
            &MapCapacities::default(),
            &[0; MAP_TABLES],
            SourceChecks::default(),
        );
        // AirWings and ArmyOrders have their own core checks independent of
        // the downstream consumers' already-staged raw jobs.
        for (wings, orders) in [(false, false), (true, false), (false, true), (true, true)] {
            actual.reset();
            reference.reset();
            extensions.reset();
            let mut needed = [false; MAP_TABLES];
            needed[8] = wings;
            needed[9] = orders;
            let memory = Reads::default();
            let reference_memory = Reads::default();
            let (expected, _) =
                reference.sample(&reference_memory, 0, 0x1000, &needed, &mut NoSources);
            let mut extension_checks = [true; TABLE_COUNT];
            // This test isolates the already-staged air/naval consumers. The
            // independent live combat-progress window is covered separately.
            extension_checks[replay::rows::TableId::LandCombatProgress.index()] = false;
            extension_checks[replay::rows::TableId::ShipModels.index()] = false;
            for table in crate::extended::plans::TABLES {
                extension_checks[table.index()] = false;
            }
            let (matched, _) = actual.sample(
                &memory,
                0,
                0x1000,
                &needed,
                &mut extensions.sources(&memory, 0, &extension_checks, true, true),
            );
            assert_eq!(*memory.0.borrow(), *reference_memory.0.borrow());
            assert_eq!(matched, expected);
            assert_eq!(actual.outcomes(), reference.outcomes());
            assert_eq!(actual.air_wings.checked, wings);
            assert_eq!(actual.army_orders.checked, orders);
        }
    }
}
