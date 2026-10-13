//! Hourly observer handoffs grow only on an admitted worker, in boundary order.
use super::{
    Captured, construction_ic, event_feed, fuel_awards, land_events, line_output,
    operation_outcomes, peace_settlements, politics,
};
use crate::sampling::qpc;
use replay::rows::TableId;
use std::sync::atomic::{AtomicU64, Ordering};

const N: usize = 8;
const FAMILIES: [&str; N] = [
    "land_events",
    "event_feed",
    "line_output",
    "fuel_awards",
    "construction_ic",
    "peace_settlements",
    "operation_outcomes",
    "politics",
];
static LOST_DRAINS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];

#[derive(Clone, Copy, Default)]
pub(crate) struct ObserverBoundaryReport {
    pub ticks: [u64; N],
}
/// One worker job: the observer drains, then the completion and political
/// lane seals. Both phases ran as separate jobs, the second dispatched and
/// joined back to back after the country snapshot. Both seal before the
/// hourly trampoline returns to the native update, where the lanes' writers
/// Run  (the operation completion writer from pre-hourly update);
/// the snapshot and `prepare_hour` between them call no native code that
/// appends to a lane. One job seals the same events with one worker wake,
/// behind the snapshot.
pub(crate) struct ObserverBoundaryRequest {
    base: usize,
    hour: i32,
    continuous: bool,
    recovery: bool,
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
    pub(crate) fn allocated_bytes(&self) -> u64 {
        (self.land.allocated_bytes()
            + self.feed.allocated_bytes()
            + self.lines.allocated_bytes()
            + self.fuel.allocated_bytes()
            + self.construction.allocated_bytes()
            + self.peace.allocated_bytes()
            + self.operations.allocated_bytes()
            + self.politics.allocated_bytes()) as u64
    }
    pub(crate) fn capture(&mut self) -> ObserverBoundaryReport {
        let mut report = ObserverBoundaryReport::default();
        crate::relation_changes::boundary(!self.continuous || self.recovery);
        crate::history_changes::boundary(!self.continuous || self.recovery);
        crate::flag_changes::boundary(!self.continuous || self.recovery);
        crate::building_changes::boundary(!self.continuous || self.recovery);
        crate::advisor_pool_changes::boundary(!self.continuous || self.recovery);
        crate::party_leader_changes::boundary(!self.continuous || self.recovery);
        crate::variable_changes::boundary(!self.continuous || self.recovery);
        crate::party_changes::boundary(!self.continuous || self.recovery);
        crate::roster_changes::boundary(!self.continuous || self.recovery);
        crate::political_status_changes::boundary(!self.continuous || self.recovery);
        crate::wing_changes::boundary(!self.continuous || self.recovery);
        crate::mission_changes::boundary(!self.continuous || self.recovery);
        crate::rail_changes::boundary(!self.continuous || self.recovery);
        crate::territory_changes::boundary(!self.continuous || self.recovery);
        crate::army_stat_changes::boundary(!self.continuous || self.recovery);
        crate::country_input_changes::boundary(!self.continuous || self.recovery);
        crate::raid_changes::boundary(!self.continuous || self.recovery);
        crate::naval_tree_changes::boundary(!self.continuous || self.recovery);
        crate::unit_leader_changes::boundary(!self.continuous || self.recovery);
        crate::census_changes::boundary(!self.continuous || self.recovery);
        crate::unit_leader_trait_changes::boundary(!self.continuous || self.recovery);
        crate::world_input_changes::boundary(!self.continuous || self.recovery);
        crate::retreat_ban_changes::boundary(!self.continuous || self.recovery);
        crate::local_subunit_changes::boundary(!self.continuous || self.recovery);
        crate::template_changes::boundary(!self.continuous || self.recovery);
        crate::ship_model_changes::boundary(!self.continuous || self.recovery);
        crate::province_unit_changes::boundary(!self.continuous || self.recovery);
        crate::air_base_carry_changes::boundary(!self.continuous || self.recovery);
        crate::character_changes::boundary(!self.continuous || self.recovery);
        crate::war_changes::boundary(!self.continuous || self.recovery);
        let begin = qpc();
        self.land.drain_growing(self.hour, !self.continuous);
        self.finished[0] = true;
        report.ticks[0] = elapsed(begin);
        let begin = qpc();
        self.feed
            .drain_growing(self.hour, !self.continuous || self.recovery);
        self.finished[1] = true;
        report.ticks[1] = elapsed(begin);
        let begin = qpc();
        self.lines.drain_growing();
        self.finished[2] = true;
        report.ticks[2] = elapsed(begin);
        let begin = qpc();
        self.fuel.drain_growing();
        crate::fuel_awards::install_offered();
        self.finished[3] = true;
        report.ticks[3] = elapsed(begin);
        let begin = qpc();
        self.construction.drain_growing();
        if prepare_outputs_at(self.hour, self.continuous, self.recovery) {
            let memory = unsafe { engine::memory::LiveMemory::new() };
            prepare_outputs(&memory, self.base);
        }
        crate::construction_ic::install_offered();
        crate::line_output::install_offered();
        self.finished[4] = true;
        report.ticks[4] = elapsed(begin);
        let begin = qpc();
        self.peace.drain_growing(self.continuous && !self.recovery);
        crate::peace_settlements::install_offered();
        self.finished[5] = true;
        report.ticks[5] = elapsed(begin);
        report.ticks[6] = self.operations.capture_growing();
        self.finished[6] = true;
        report.ticks[7] = self.politics.capture_growing();
        self.finished[7] = true;
        report
    }
    fn fail_pending(&mut self, reason: &'static str) {
        if !self.finished[0] {
            crate::relation_changes::boundary(true);
            crate::history_changes::boundary(true);
            crate::flag_changes::boundary(true);
            crate::building_changes::boundary(true);
            crate::advisor_pool_changes::boundary(true);
            crate::party_leader_changes::boundary(true);
            crate::variable_changes::boundary(true);
            crate::party_changes::boundary(true);
            crate::roster_changes::boundary(true);
            crate::political_status_changes::boundary(true);
            crate::wing_changes::boundary(true);
            crate::mission_changes::boundary(true);
            crate::rail_changes::boundary(true);
            crate::territory_changes::boundary(true);
            crate::army_stat_changes::boundary(true);
            crate::country_input_changes::boundary(true);
            crate::raid_changes::boundary(true);
            crate::naval_tree_changes::boundary(true);
            crate::unit_leader_changes::boundary(true);
            crate::census_changes::boundary(true);
            crate::unit_leader_trait_changes::boundary(true);
            crate::world_input_changes::boundary(true);
            crate::retreat_ban_changes::boundary(true);
            crate::local_subunit_changes::boundary(true);
            crate::template_changes::boundary(true);
            crate::ship_model_changes::boundary(true);
            crate::province_unit_changes::boundary(true);
            crate::air_base_carry_changes::boundary(true);
            crate::character_changes::boundary(true);
            crate::war_changes::boundary(true);
        }
        let lane_reason = lane_reason(&self.finished, reason);
        for (index, finished) in self.finished.into_iter().enumerate() {
            if !finished {
                LOST_DRAINS[index].fetch_add(1, Ordering::Relaxed);
                match index {
                    0 => self.land.fail(self.hour, reason),
                    1 => self.feed.fail(self.hour, reason),
                    2 => {
                        crate::line_output::invalidate();
                        self.lines.fail(reason);
                    }
                    3 => self.fuel.fail(reason),
                    4 => {
                        crate::construction_ic::invalidate();
                        self.construction.fail(reason);
                    }
                    5 => {
                        crate::peace_settlements::lost_drain();
                        self.peace.fail(reason);
                    }
                    6 => self.operations.fail(lane_reason),
                    7 => self.politics.fail(lane_reason),
                    _ => unreachable!(),
                }
            }
        }
    }
}
// The hourly update enters the daily update at hour %24==23;
// daily production then visits the current production queues.
// Prepare at that parked boundary, and on first/load/recovery boundaries,
// rather than relying on a wall-clock poll while the simulation runs rapidly.
fn prepare_outputs_at(hour: i32, continuous: bool, recovery: bool) -> bool {
    !continuous || recovery || hour.rem_euclid(24) == 23
}
fn prepare_outputs(memory: &impl engine::Memory, base: usize) {
    crate::line_output::collect();
    crate::construction_ic::collect();
    crate::line_output::presize(
        crate::country_scale::observe_production_output(memory, base).unwrap_or(0),
    );
    let _ = crate::construction_ic::presize_from_live(memory, base);
}
/// The reason a failed job gives its unsealed lanes. They were their own
/// job, dispatched after the drains joined: a drain that failed stopped the
/// pool first, which refused that job as stopped. Lanes the job reached
/// after every drain keep its own reason.
fn lane_reason(finished: &[bool; N], reason: &'static str) -> &'static str {
    if finished[..6].iter().all(|&finished| finished) {
        reason
    } else {
        crate::sampling::acquisition_failure_reason(crate::acquisition::Failure::Stopped)
    }
}
fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}
pub(crate) fn status() -> crate::status::Object<impl crate::status::Fields> {
    crate::status::status!({
        "lost_worker_drains": crate::status::map(FAMILIES.into_iter().zip(LOST_DRAINS.iter().map(|count| count.load(Ordering::Relaxed))).filter(|(_, count)| *count != 0)),
        "scope": "rejected or unfinished hourly observer worker handoffs; finished sibling intervals retain their original outcomes",
    })
}
impl Captured {
    pub(crate) fn take_observer_boundary_request(
        &mut self,
        base: usize,
        hour: i32,
        continuous: bool,
        recovery: bool,
    ) -> ObserverBoundaryRequest {
        ObserverBoundaryRequest {
            base,
            hour,
            continuous,
            recovery,
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
        crate::allocation_failure_tests::with_allocations_denied(|| {
            let request = captured.take_observer_boundary_request(0, 41, true, false);
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
    fn native_daily_boundary_sizes_cold_output_logs_without_a_wall_clock_poll() {
        use super::super::production::production_rest_test_kit as kit;
        use engine::{Memory, cgame_state, cproduction_status};
        let _construction = crate::construction_ic::test_lock();
        let _lines = crate::line_output::test_lock();
        struct Loaded(Vec<u8>);
        impl Memory for Loaded {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == cgame_state::SINGLETON_RVA && output.len() == 8 {
                    output.copy_from_slice(&(kit::BASE as u64).to_le_bytes());
                    true
                } else {
                    engine::memory::SliceMemory {
                        base: kit::BASE,
                        bytes: &self.0,
                    }
                    .copy(address, output)
                }
            }
        }
        for population in [1500u32, 5000] {
            crate::construction_ic::reset_log_for_test(0);
            crate::line_output::reset_log_for_test(0);
            let mut memory = Loaded(kit::fixture());
            let set = |bytes: &mut [u8], address: usize, value: &[u8]| {
                bytes[address - kit::BASE..address - kit::BASE + value.len()]
                    .copy_from_slice(value);
            };
            // The production reader needs only current roster counts. The
            // construction reader visits every current queue occurrence.
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
                &(engine::cbuilding_production_line::VTABLE_RVA as u64).to_le_bytes(),
            );
            assert!(prepare_outputs_at(11, false, false));
            assert!(prepare_outputs_at(23, true, false));
            assert!(!prepare_outputs_at(22, true, false));
            prepare_outputs(&memory, 0);
            crate::line_output::install_offered();
            crate::construction_ic::install_offered();
            assert!(
                serde_json::to_value(crate::line_output::status()).unwrap()["capacity"]
                    .as_u64()
                    .unwrap()
                    >= u64::from(population) * 2
            );
            assert!(crate::construction_ic::capacity() >= 6);
        }
    }

    /// A drain that fails refuses the lanes not yet sealed as stopped, the
    /// reason their own later job had when the failure stopped the pool;
    /// lanes reached after every drain keep the job's reason.
    #[test]
    fn lanes_after_a_failed_drain_keep_their_stopped_reason() {
        // The reason alone: failing a pack would touch the drains' and
        // lanes' process-wide state, which other tests hold.
        let panicked =
            crate::sampling::acquisition_failure_reason(crate::acquisition::Failure::Panicked);
        let stopped =
            crate::sampling::acquisition_failure_reason(crate::acquisition::Failure::Stopped);
        let mut finished = [false; N];
        for drains in 0..6 {
            finished[..drains].fill(true);
            assert_eq!(lane_reason(&finished, panicked), stopped, "{drains} drains");
        }
        finished[..6].fill(true);
        assert_eq!(lane_reason(&finished, panicked), panicked);
        // A lane already sealed does not change the reason of the other.
        finished[6] = true;
        assert_eq!(lane_reason(&finished, panicked), panicked);
    }

    /// The completion and political lanes travel with the observer drains in
    /// one job: their packs leave and return with it, and their seal costs
    /// are charged to their own tables.
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
        crate::allocation_failure_tests::with_allocations_denied(|| {
            let request = captured.take_observer_boundary_request(0, 41, true, false);
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
