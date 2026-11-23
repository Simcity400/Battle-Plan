use super::{
    Captured, construction_ic, event_feed, fuel_awards, line_output, peace_settlements, politics,
};
use collector_land_events::extended::land_events;
use collector_operation_outcomes::extended::operation_outcomes;
use collector_primitives::clock::qpc;
use replay_data::rows::TableId;

const N: usize = 8;

#[derive(Clone, Copy, Default)]
pub(crate) struct ObserverBoundaryReport {
    pub ticks: [u64; N],
}
pub(crate) struct ObserverBoundaryRequest {
    hour: i32,
    continuous: bool,
    recovery: bool,
    pub(crate) market_clock: Option<engine_native::cgame_state::Clock<u64>>,
    pub(crate) market_refresh: bool,
    land: land_events::Captured,
    feed: event_feed::Captured,
    lines: line_output::Captured,
    fuel: fuel_awards::Captured,
    construction: construction_ic::Captured,
    peace: peace_settlements::Captured,
    operations: operation_outcomes::Captured,
    politics: politics::Captured,
    finished: [bool; N],
}
impl ObserverBoundaryRequest {
    #[cfg(test)]
    pub fn allocated_bytes(&self) -> u64 {
        (self.land.allocated_bytes()
            + self.feed.allocated_bytes()
            + self.lines.allocated_bytes()
            + self.fuel.allocated_bytes()
            + self.construction.allocated_bytes()
            + self.peace.allocated_bytes()
            + self.operations.allocated_bytes()
            + self.politics.allocated_bytes()) as u64
    }
    pub fn capture(&mut self) -> ObserverBoundaryReport {
        let mut report = ObserverBoundaryReport::default();
        let boundaries = qpc();
        // SAFETY: Runs on the admitted hour's sole observer boundary while market observation lives.
        unsafe {
            collector_observers::market_changes::boundary(
                self.market_clock,
                !self.continuous || self.recovery || self.market_refresh,
            );
        }
        collector_air_relations::relation_changes::boundary(!self.continuous || self.recovery);
        collector_air_histories::history_changes::boundary(!self.continuous || self.recovery);
        collector_intel::flag_changes::boundary(!self.continuous || self.recovery);
        collector_intel::propaganda_changes::boundary(!self.continuous || self.recovery);
        collector_intel::network_auxiliary::boundary(!self.continuous || self.recovery);
        collector_intel::source_changes::boundary(!self.continuous || self.recovery);
        collector_intel::intel::network_summary_boundary(!self.continuous || self.recovery);
        collector_construction::building_changes::boundary(!self.continuous || self.recovery);
        collector_observers::advisor_pool_changes::boundary(!self.continuous || self.recovery);
        collector_governance::party_leader_changes::boundary(!self.continuous || self.recovery);
        collector_intel::variable_changes::boundary(!self.continuous || self.recovery);
        collector_governance::party_changes::boundary(!self.continuous || self.recovery);
        collector_observers::roster_changes::boundary(!self.continuous || self.recovery);
        collector_observers::political_status_changes::boundary(!self.continuous || self.recovery);
        collector_observers::wing_changes::boundary(!self.continuous || self.recovery);
        collector_missions::mission_changes::boundary(!self.continuous || self.recovery);
        collector_observers::rail_changes::boundary(!self.continuous || self.recovery);
        collector_observers::territory_changes::boundary(!self.continuous || self.recovery);
        collector_division_parts::army_stat_changes::boundary(!self.continuous || self.recovery);
        collector_observers::country_input_changes::boundary(!self.continuous || self.recovery);
        collector_observers::raid_changes::boundary(!self.continuous || self.recovery);
        collector_observers::naval_tree_changes::boundary(!self.continuous || self.recovery);
        collector_observers::unit_leader_changes::boundary(!self.continuous || self.recovery);
        // SAFETY: Runs on the admitted hour's sole observer boundary while naval watches live.
        unsafe {
            collector_naval_management::changes::boundary(!self.continuous || self.recovery);
        }
        collector_observers::census_changes::boundary(!self.continuous || self.recovery);
        collector_observers::orders_group_changes::boundary(!self.continuous || self.recovery);
        collector_observers::unit_leader_trait_changes::boundary(!self.continuous || self.recovery);
        collector_observers::world_input_changes::boundary(!self.continuous || self.recovery);
        collector_observers::retreat_ban_changes::boundary(!self.continuous || self.recovery);
        collector_model_inputs::local_subunit_changes::boundary(!self.continuous || self.recovery);
        collector_production::production::design_changes::boundary(
            !self.continuous || self.recovery,
        );
        collector_division_templates::template_changes::boundary(!self.continuous || self.recovery);
        // SAFETY: Runs on the admitted hour's sole observer boundary while faction watches live.
        unsafe {
            collector_factions::faction_goal_changes::boundary(!self.continuous || self.recovery);
            collector_factions::faction_companion_changes::boundary(
                !self.continuous || self.recovery,
            );
        }
        collector_ship_models::ship_model_changes::boundary(!self.continuous || self.recovery);
        collector_divisions::province_unit_changes::boundary(!self.continuous || self.recovery);
        collector_observers::province_combat_changes::boundary(!self.continuous || self.recovery);
        collector_observers::air_base_carry_changes::boundary(!self.continuous || self.recovery);
        collector_observers::character_changes::boundary(!self.continuous || self.recovery);
        collector_observers::war_changes::boundary(!self.continuous || self.recovery);
        // SAFETY: Runs on the admitted hour's sole observer boundary while land-event storage lives.
        unsafe { self.land.drain(self.hour, !self.continuous) };
        self.finished[0] = true;
        report.ticks[0] = elapsed(boundaries);
        let begin = qpc();
        // SAFETY: Runs on the admitted hour's sole observer boundary with teardown excluded
        // (event-feed hooks may be stably disabled/absent).
        unsafe {
            self.feed
                .drain(self.hour, !self.continuous || self.recovery);
        }
        self.finished[1] = true;
        report.ticks[1] = elapsed(begin);
        let begin = qpc();
        // SAFETY: Admitted hour's sole observer boundary drains the line-output interval once.
        unsafe { self.lines.drain() };
        self.finished[2] = true;
        report.ticks[2] = elapsed(begin);
        let begin = qpc();
        self.fuel.drain();
        collector_fuel_awards::fuel_awards::install_offered();
        self.finished[3] = true;
        report.ticks[3] = elapsed(begin);
        let begin = qpc();
        // SAFETY: Admitted hour's sole observer boundary drains construction-IC and installs
        // offered output logs under the capture gate.
        unsafe {
            self.construction.drain();
            collector_output::construction_ic::install_offered();
            collector_output::line_output::install_offered();
        }
        self.finished[4] = true;
        report.ticks[4] = elapsed(begin);
        let begin = qpc();
        self.peace.drain(self.continuous && !self.recovery);
        collector_peace::peace_settlements::install_offered();
        self.finished[5] = true;
        report.ticks[5] = elapsed(begin);
        // SAFETY: Admitted hour's sole observer boundary seals operation-outcome and politics
        // handoffs while their global staging logs remain live.
        report.ticks[6] = unsafe { self.operations.capture() };
        self.finished[6] = true;
        // SAFETY: Admitted hour's sole observer boundary seals the politics handoff while staging lives.
        report.ticks[7] = unsafe { self.politics.capture() };
        self.finished[7] = true;
        report
    }
    fn fail_pending(&mut self, reason: &'static str) {
        if !self.finished[0] {
            collector_air_relations::relation_changes::boundary(true);
            collector_air_histories::history_changes::boundary(true);
            collector_intel::flag_changes::boundary(true);
            collector_intel::propaganda_changes::boundary(true);
            collector_intel::source_changes::boundary(true);
            collector_intel::network_auxiliary::boundary(true);
            collector_intel::intel::network_summary_boundary(true);
            collector_construction::building_changes::boundary(true);
            collector_observers::advisor_pool_changes::boundary(true);
            collector_governance::party_leader_changes::boundary(true);
            collector_intel::variable_changes::boundary(true);
            collector_governance::party_changes::boundary(true);
            collector_observers::roster_changes::boundary(true);
            collector_observers::political_status_changes::boundary(true);
            collector_observers::wing_changes::boundary(true);
            collector_missions::mission_changes::boundary(true);
            collector_observers::rail_changes::boundary(true);
            collector_observers::territory_changes::boundary(true);
            collector_division_parts::army_stat_changes::boundary(true);
            collector_observers::country_input_changes::boundary(true);
            collector_observers::raid_changes::boundary(true);
            collector_observers::naval_tree_changes::boundary(true);
            collector_observers::unit_leader_changes::boundary(true);
            // SAFETY: Admitted hour's sole observer boundary invalidates naval watches on failure.
            unsafe { collector_naval_management::changes::boundary(true) };
            collector_observers::census_changes::boundary(true);
            collector_observers::orders_group_changes::boundary(true);
            collector_observers::unit_leader_trait_changes::boundary(true);
            collector_observers::world_input_changes::boundary(true);
            collector_observers::retreat_ban_changes::boundary(true);
            collector_model_inputs::local_subunit_changes::boundary(true);
            collector_production::production::design_changes::boundary(true);
            collector_division_templates::template_changes::boundary(true);
            // SAFETY: Admitted hour's sole observer boundary invalidates faction watches on failure.
            unsafe {
                collector_factions::faction_goal_changes::boundary(true);
                collector_factions::faction_companion_changes::boundary(true);
            }
            collector_ship_models::ship_model_changes::boundary(true);
            collector_divisions::province_unit_changes::boundary(true);
            collector_observers::province_combat_changes::boundary(true);
            collector_observers::air_base_carry_changes::boundary(true);
            collector_observers::character_changes::boundary(true);
            collector_observers::war_changes::boundary(true);
        }
        let lane_reason = lane_reason(&self.finished, reason);
        for (index, finished) in self.finished.into_iter().enumerate() {
            if !finished {
                // SAFETY: Observer-boundary failure path owns invalidating unfinished live lanes.
                match index {
                    0 => unsafe { self.land.fail(self.hour, reason) },
                    1 => unsafe { self.feed.fail(self.hour, reason) },
                    2 => {
                        unsafe { collector_output::line_output::invalidate() };
                        self.lines.fail(reason);
                    }
                    3 => self.fuel.fail(reason),
                    4 => {
                        unsafe { collector_output::construction_ic::invalidate() };
                        self.construction.fail(reason);
                    }
                    5 => {
                        collector_peace::peace_settlements::lost_drain();
                        self.peace.fail(reason);
                    }
                    6 => unsafe { self.operations.fail(lane_reason) },
                    7 => unsafe { self.politics.fail(lane_reason) },
                    _ => unreachable!(),
                }
            }
        }
    }
}
fn lane_reason(finished: &[bool; N], reason: &'static str) -> &'static str {
    if finished[..6].iter().all(|&finished| finished) {
        reason
    } else {
        crate::sampling::acquisition_failure_reason(
            collector_workers::acquisition::Failure::Stopped,
        )
    }
}
fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}
impl Captured {
    pub(crate) fn take_observer_boundary_request(
        &mut self,
        hour: i32,
        continuous: bool,
        recovery: bool,
    ) -> ObserverBoundaryRequest {
        ObserverBoundaryRequest {
            hour,
            continuous,
            recovery,
            market_clock: None,
            market_refresh: false,
            land: std::mem::replace(&mut self.land_events, land_events::Captured::vacant()),
            feed: std::mem::replace(&mut self.event_feed, event_feed::Captured::vacant()),
            lines: std::mem::replace(&mut self.line_output, line_output::Captured::vacant()),
            fuel: std::mem::replace(&mut self.fuel_awards, fuel_awards::Captured::vacant()),
            construction: std::mem::replace(
                &mut self.construction_ic,
                construction_ic::Captured::vacant(),
            ),
            peace: std::mem::replace(
                &mut self.peace_settlements,
                peace_settlements::Captured::vacant(),
            ),
            operations: std::mem::replace(
                &mut self.operation_outcomes,
                operation_outcomes::Captured::vacant(),
            ),
            politics: std::mem::replace(&mut self.politics, politics::Captured::vacant()),
            finished: [false; N],
        }
    }
    pub(crate) fn restore_observer_boundary_request(
        &mut self,
        mut request: ObserverBoundaryRequest,
        report: ObserverBoundaryReport,
        failure: Option<&'static str>,
    ) {
        if let Some(reason) = failure {
            request.fail_pending(reason);
        }
        self.land_events = request.land;
        self.event_feed = request.feed;
        self.line_output = request.lines;
        self.fuel_awards = request.fuel;
        self.construction_ic = request.construction;
        self.peace_settlements = request.peace;
        self.operation_outcomes = request.operations;
        self.politics = request.politics;
        for (table, ticks) in [
            (TableId::LandEventCoverage, report.ticks[0]),
            (TableId::EventFeedHistory, report.ticks[1]),
            (TableId::ProductionLineOutput, report.ticks[2]),
            (TableId::ConstructionLineOutput, report.ticks[4]),
            (TableId::PeaceSettlements, report.ticks[5]),
            (TableId::OperationCompletions, report.ticks[6]),
            (TableId::PoliticsCaptureCoverage, report.ticks[7]),
        ] {
            self.overhead[table.index()] = self.overhead[table.index()].saturating_add(ticks);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observer_take_restore_keeps_all_owned_arenas_without_callback_allocation() {
        let mut captured = Captured::with_capacities(&super::super::Capacities::default());
        let original = captured.land_events.allocated_bytes()
            + captured.event_feed.allocated_bytes()
            + captured.line_output.allocated_bytes()
            + captured.fuel_awards.allocated_bytes()
            + captured.construction_ic.allocated_bytes()
            + captured.peace_settlements.allocated_bytes();
        collector_test_support::with_allocations_denied(|| {
            let request = captured.take_observer_boundary_request(41, true, false);
            assert_eq!(request.allocated_bytes(), original as u64);
            assert_eq!(captured.land_events.allocated_bytes(), 0);
            assert_eq!(captured.event_feed.allocated_bytes(), 0);
            assert_eq!(captured.line_output.allocated_bytes(), 0);
            assert_eq!(captured.fuel_awards.allocated_bytes(), 0);
            assert_eq!(captured.construction_ic.allocated_bytes(), 0);
            assert_eq!(captured.peace_settlements.allocated_bytes(), 0);
            captured.restore_observer_boundary_request(
                request,
                ObserverBoundaryReport::default(),
                None,
            );
        });
        assert_eq!(
            captured.land_events.allocated_bytes()
                + captured.event_feed.allocated_bytes()
                + captured.line_output.allocated_bytes()
                + captured.fuel_awards.allocated_bytes()
                + captured.construction_ic.allocated_bytes()
                + captured.peace_settlements.allocated_bytes(),
            original
        );
    }

    #[test]
    fn the_sizing_thread_sizes_cold_output_logs_and_the_boundary_only_installs_them() {
        use super::super::production::production_rest_test_kit as kit;
        use engine_native_memory::Memory;
        use engine_native::{cgame_state, cproduction_status};
        let _construction = collector_output::construction_ic::test_lock();
        let _lines = collector_output::line_output::test_lock();
        struct Loaded(Vec<u8>);
        impl Memory for Loaded {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
                    output.copy_from_slice(&(kit::BASE as u64).to_le_bytes());
                    true
                } else {
                    engine_native_memory::memory::SliceMemory {
                        base: kit::BASE,
                        bytes: &self.0,
                    }
                    .copy(address, output)
                }
            }
        }
        for population in [1500u32, 5000] {
            // SAFETY: Test holds both `construction_ic::test_lock()` and `line_output::test_lock()`
            // with no live hooks installed.
            unsafe {
                collector_output::construction_ic::reset_log_for_test(0);
                collector_output::line_output::reset_log_for_test(0);
            }
            let mut memory = Loaded(kit::fixture());
            let set = |bytes: &mut [u8], address: usize, value: &[u8]| {
                bytes[address - kit::BASE..address - kit::BASE + value.len()]
                    .copy_from_slice(value);
            };
            set(
                &mut memory.0,
                0x10000 + cproduction_status::LINES + 8,
                &population.to_le_bytes(),
            );
            set(
                &mut memory.0,
                0x10000 + cproduction_status::LINES + 12,
                &population.to_le_bytes(),
            );
            set(
                &mut memory.0,
                0x10000 + cproduction_status::CONSTRUCTION_LINES,
                &0x11800u64.to_le_bytes(),
            );
            set(
                &mut memory.0,
                0x10000 + cproduction_status::CONSTRUCTION_LINES + 8,
                &1u32.to_le_bytes(),
            );
            set(
                &mut memory.0,
                0x10000 + cproduction_status::CONSTRUCTION_LINES + 12,
                &1u32.to_le_bytes(),
            );
            set(&mut memory.0, 0x11800, &0x11900u64.to_le_bytes());
            set(
                &mut memory.0,
                0x11900,
                &(engine_production::cbuilding_production_line::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            collector_output::line_output::collect();
            collector_output::construction_ic::collect();
            collector_output::line_output::presize(
                crate::country_scale::observe_production_output(&memory, 0).unwrap(),
            );
            collector_output::construction_ic::presize_from_live(&memory, 0).unwrap();
            collector_test_support::with_allocations_denied(|| {
                // SAFETY: Test holds output test locks with no live hooks running.
                unsafe {
                    collector_output::line_output::install_offered();
                    collector_output::construction_ic::install_offered();
                }
            });
            assert!(
                serde_json::to_value(collector_output::line_output::status()).unwrap()["capacity"]
                    .as_u64()
                    .unwrap()
                    >= u64::from(population) * 2
            );
            assert!(collector_output::construction_ic::capacity() >= 6);
        }
    }

    #[test]
    fn lanes_after_a_failed_drain_keep_their_stopped_reason() {
        let panicked = crate::sampling::acquisition_failure_reason(
            collector_workers::acquisition::Failure::Panicked,
        );
        let stopped = crate::sampling::acquisition_failure_reason(
            collector_workers::acquisition::Failure::Stopped,
        );
        let mut finished = [false; N];
        for drains in 0..6 {
            finished[..drains].fill(true);
            assert_eq!(lane_reason(&finished, panicked), stopped, "{drains} drains");
        }
        finished[..6].fill(true);
        assert_eq!(lane_reason(&finished, panicked), panicked);
        finished[6] = true;
        assert_eq!(lane_reason(&finished, panicked), panicked);
    }

    #[test]
    fn lanes_travel_with_the_observer_drains_in_one_job() {
        let mut captured = Captured::with_capacities(&super::super::Capacities::default());
        let early_bytes = captured.land_events.allocated_bytes()
            + captured.event_feed.allocated_bytes()
            + captured.line_output.allocated_bytes()
            + captured.fuel_awards.allocated_bytes()
            + captured.construction_ic.allocated_bytes()
            + captured.peace_settlements.allocated_bytes();
        let lane_bytes =
            captured.operation_outcomes.allocated_bytes() + captured.politics.allocated_bytes();
        assert_eq!(lane_bytes, 0);
        collector_test_support::with_allocations_denied(|| {
            let request = captured.take_observer_boundary_request(41, true, false);
            assert_eq!(request.allocated_bytes(), (early_bytes + lane_bytes) as u64);
            assert_eq!(captured.operation_outcomes.allocated_bytes(), 0);
            assert_eq!(captured.politics.allocated_bytes(), 0);
            let mut report = ObserverBoundaryReport::default();
            report.ticks[6] = 17;
            report.ticks[7] = 23;
            captured.restore_observer_boundary_request(request, report, None);
        });
        assert_eq!(captured.overhead[TableId::OperationCompletions.index()], 17);
        assert_eq!(
            captured.overhead[TableId::PoliticsCaptureCoverage.index()],
            23
        );
        assert_eq!(
            early_bytes,
            captured.land_events.allocated_bytes()
                + captured.event_feed.allocated_bytes()
                + captured.line_output.allocated_bytes()
                + captured.fuel_awards.allocated_bytes()
                + captured.construction_ic.allocated_bytes()
                + captured.peace_settlements.allocated_bytes()
        );
    }
}
