use super::{MAP_TABLES, MapCaptured, elapsed, qpc};
use collector_land_events::extended::combat_progress;
use collector_primitives::table_capture::CaptureRows;
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::{cland_combat, cnaval_combat, cnaval_combatant};
use replay_data::rows::TableId;

#[derive(Clone, Copy, Default)]
pub struct CombatReport {
    pub map_ticks: [u64; MAP_TABLES],
    pub progress_ticks: u64,
}

pub struct CombatRequest {
    base: usize,
    state: usize,
    land: CaptureRows<cland_combat::Row>,
    land_participants: Vec<u64>,
    land_classes: Vec<u32>,
    land_participants_len: usize,
    land_participants_required: usize,
    naval: CaptureRows<cnaval_combat::Row>,
    naval_participants: CaptureRows<cnaval_combatant::Row>,
    naval_groups: Vec<usize>,
    naval_groups_required: usize,
    progress: combat_progress::Captured,
    progress_recorded: bool,
    land_finished: bool,
    naval_finished: bool,
}

impl CombatRequest {
    /// # Safety
    /// Run only this admitted hour's unique combat job against the session image. Settle its daily-
    /// observer loan into the originating frame before admission ends.
    pub unsafe fn capture(&mut self, memory: &impl Memory) -> CombatReport {
        let mut report = CombatReport::default();
        let mut reads = collector_memory::read_accounting::table(TableId::LandCombats);
        let begin = qpc();
        let progress_begin = qpc();
        let recorded = self.progress_recorded;
        if recorded {
            // SAFETY: Admitted combat job owns the daily observer baseline loan until `finish()`.
            unsafe { self.progress.begin() };
        }
        report.progress_ticks = elapsed(progress_begin);
        let land = cland_combat::walk_with(
            memory,
            self.base,
            self.state,
            &mut self.land.rows,
            &mut self.land_participants,
            &mut self.land_classes,
            |source| {
                if recorded {
                    let begin = qpc();
                    self.progress.visit(memory, self.base, source);
                    report.progress_ticks = report.progress_ticks.saturating_add(elapsed(begin));
                }
            },
        );
        let progress_begin = qpc();
        // SAFETY: Admitted combat job settles the daily observer baseline and reports sizing demand.
        unsafe {
            self.progress.finish(if recorded {
                land.outcome
            } else {
                Outcome::Unavailable {
                    reason: collector_extended_support::outcomes::NOT_RECORDED,
                }
            });
        }
        report.progress_ticks = report
            .progress_ticks
            .saturating_add(elapsed(progress_begin));
        self.land.outcome = land.outcome;
        self.land_participants_len = land.participants_len;
        self.land_participants_required = land.required_participants;
        self.land_finished = true;
        report.map_ticks[2] = elapsed(begin).saturating_sub(report.progress_ticks);

        let tables = [TableId::NavalCombats, TableId::NavalCombatParticipants];
        reads.select(tables[0]);
        let naval = cnaval_combat::walk_timed(
            memory,
            self.base,
            self.state,
            &mut self.naval.rows,
            &mut self.naval_participants.rows,
            &mut self.naval_groups,
            |phase| {
                reads.select(tables[phase]);
                qpc() as u64
            },
        );
        self.naval.outcome = naval.capture.combats;
        self.naval_participants.outcome = naval.capture.participants;
        self.naval_groups_required = naval.capture.required_group_capacity;
        self.naval_finished = true;
        report.map_ticks[6..8].copy_from_slice(&naval.read_ticks);
        report
    }

    fn fail_pending(&mut self, reason: &'static str) {
        let outcome = Outcome::Unavailable { reason };
        if !self.land_finished {
            self.land.fail(outcome);
            self.land_participants_len = 0;
            // SAFETY: Admitted combat job failed before completing land capture; settle baseline.
            unsafe { self.progress.finish(outcome) };
        }
        if !self.naval_finished {
            self.naval.fail(outcome);
            self.naval_participants.fail(outcome);
        }
    }
}

impl MapCaptured {
    /// # Safety
    /// Take the combat buffers once, from the frame exclusively owned by the admitted hour. Return
    /// the request only to that frame, including on failure.
    pub unsafe fn take_combat_request(
        &mut self,
        base: usize,
        state: usize,
        progress: combat_progress::Captured,
        progress_recorded: bool,
    ) -> CombatRequest {
        CombatRequest {
            base,
            state,
            progress_recorded,
            land: std::mem::replace(&mut self.land_combats, CaptureRows::new(0)),
            land_participants: std::mem::take(&mut self.land_participants),
            land_classes: std::mem::take(&mut self.land_classes),
            land_participants_len: 0,
            land_participants_required: 0,
            naval: std::mem::replace(&mut self.naval_combats, CaptureRows::new(0)),
            naval_participants: std::mem::replace(
                &mut self.naval_combat_participants,
                CaptureRows::new(0),
            ),
            naval_groups: std::mem::take(&mut self.naval_groups),
            naval_groups_required: 0,
            progress,
            land_finished: false,
            naval_finished: false,
        }
    }

    /// # Safety
    /// Return this frame's own request exactly once. A successful report must come from executing
    /// it in this hour. A failed restoration may clear the observer baseline, so only the admitted
    /// combat owner may perform it.
    pub unsafe fn restore_combat_request(
        &mut self,
        mut request: CombatRequest,
        report: CombatReport,
        failure: Option<&'static str>,
    ) -> combat_progress::Captured {
        if let Some(reason) = failure {
            request.fail_pending(reason);
        }
        self.land_combats = request.land;
        self.land_participants = request.land_participants;
        self.land_classes = request.land_classes;
        self.land_participants_len = request.land_participants_len;
        self.land_participants_required = request.land_participants_required;
        self.naval_combats = request.naval;
        self.naval_combat_participants = request.naval_participants;
        self.naval_groups = request.naval_groups;
        self.naval_groups_required = request.naval_groups_required;
        self.combat_ticks = Some(report.map_ticks);
        request.progress
    }
}

#[cfg(test)]
mod tests;
