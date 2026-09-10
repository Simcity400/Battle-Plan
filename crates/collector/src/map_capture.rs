//! Pooled map-table reads. Only the spool thread grows or releases allocations.

#[cfg(test)]
use crate::sampling::CONSISTENCY_TARGET;
use crate::sampling::{full_match, qpc};
use crate::table_capture::{CaptureRows, comparison, comparison_rows, grow_capacity, grow_vec};
use engine::{
    Memory, cair_wing, cfleet, chuman, cland_combat, cnaval_combat, cnaval_combatant,
    corder_instance, corders_group, cship, ctask_force, walk::Outcome,
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
}
