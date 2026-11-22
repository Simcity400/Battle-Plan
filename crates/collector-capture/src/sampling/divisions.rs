use super::{Captured, elapsed, extended, qpc};
use engine_native::walk::Outcome;

#[derive(Debug, Default)]
pub struct DivisionsJob {
    pub(crate) presentation: Option<std::sync::Arc<crate::acquisition_jobs::PresentationInputs>>,
    pub census: collector_divisions::division_census::Census,
    pub(super) details: crate::division_detail::OwnedDetails,
    pub(super) context: DivisionsCopyContext,
    pub(super) acquisition_ticks: [u64; 6],
    pub(super) inputs_captured: bool,
    pub(super) core_finished: bool,
    pub(super) overflowed: bool,
    pub(super) capacity_charge: collector_primitives::raw_memory::Charge,
    pub(super) source_packet: Vec<engine_native::cequipment_variant_pool::DivisionSource>,
    pub slots: Option<(u32, u32)>,
    pub(crate) join_capacity: collector_divisions::division_census::JoinCapacity,
}
impl DivisionsJob {
    pub(crate) fn new_worker() -> Self {
        Self {
            census: collector_divisions::division_census::Census::new_worker(),
            ..Self::default()
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn new() -> Self {
        let census = collector_divisions::division_census::Census::new();
        let mut job = Self {
            details: crate::division_detail::OwnedDetails::new(census.source_requirement()),
            census,
            ..Self::default()
        };
        job.update_capacity_charge();
        job
    }
    pub fn clear(&mut self) {
        self.census.clear();
        self.details.clear();
        self.overflowed = false;
        self.acquisition_ticks = [0; 6];
        self.inputs_captured = false;
        self.core_finished = false;
    }
    pub fn initialized(&self) -> bool {
        self.census.initialized()
    }
    pub(crate) fn grow_for_next(&self) -> bool {
        self.overflowed || self.census.overflowed || self.census.counts_need_growth()
    }
    pub fn grow(&mut self) {
        engine_allocation::advisory(|| self.grow_hints());
    }

    fn grow_hints(&mut self) {
        self.census.grow();
        self.details.grow(self.census.source_requirement());
        self.update_capacity_charge();
    }
    pub(super) fn update_capacity_charge(&mut self) {
        self.capacity_charge
            .set(collector_primitives::raw_memory::Capacity {
                auxiliary_vectors: self.census.allocated_bytes()
                    + self.details.allocated_bytes() as u64
                    + collector_primitives::raw_memory::vector_bytes(&self.source_packet),
                ..Default::default()
            });
    }
    #[cfg(test)]
    pub(super) fn source_outcome(&self) -> Outcome {
        self.census.outcome()
    }
    pub(super) fn supply_sources(
        &mut self,
        captured: &mut extended::Captured,
        failure: Option<&'static str>,
    ) {
        if let Some(reason) = failure.or((!self.core_finished || self.overflowed)
            .then_some("division source acquisition could not complete"))
        {
            captured
                .install_division_sources(&mut self.source_packet, Outcome::Unavailable { reason });
            return;
        }
        let mut outcome = self.census.outcome();
        if outcome.len() > self.source_packet.len() {
            outcome = Outcome::Unavailable {
                reason: "division source packet incomplete",
            };
        }
        captured.install_division_sources(&mut self.source_packet, outcome);
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DivisionsCopyReport {
    pub(crate) raw_ok: bool,
    pub(crate) copy_ticks: [u64; 6],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DivisionsCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub source_capacity: usize,
    pub detail_capacities: [usize; 5],
    pub details_active: bool,
}
impl Default for DivisionsCopyContext {
    fn default() -> Self {
        let c = collector_production::production::Capacities::default().rows;
        Self {
            base: 0,
            game_state: 0,
            source_capacity: 8192,
            detail_capacities: [c[2], c[7], c[8], c[9], c[10]],
            details_active: true,
        }
    }
}
fn copy_divisions_inputs_pass(
    job: &mut DivisionsJob,
    memory: &impl engine_native::Memory,
    context: DivisionsCopyContext,
) {
    let _reads = collector_memory::read_accounting::table(replay_data::rows::TableId::Divisions);
    job.clear();
    job.context = context;
    if !context.details_active {
        job.details.mark_skipped();
    }
    let started = qpc();
    if let Some(slots) = job.slots {
        job.census
            .capture_inputs_part(memory, context.base, context.game_state, slots);
        job.census.reserve_join(job.join_capacity);
    } else {
        job.census
            .capture_inputs(memory, context.base, context.game_state);
    }
    job.acquisition_ticks[0] = elapsed(started);
    job.inputs_captured = true;
    job.overflowed = job.census.overflowed;
}

pub(crate) fn copy_divisions_inputs_refresh(
    memory: &impl engine_native::Memory,
    ctx: DivisionsCopyContext,
    payload: &mut DivisionsJob,
) -> bool {
    copy_divisions_inputs_pass(payload, memory, ctx);
    !payload.overflowed
}

fn copy_divisions_core_pass(
    job: &mut DivisionsJob,
    memory: &impl engine_native::Memory,
    context: DivisionsCopyContext,
) {
    let _reads = collector_memory::read_accounting::table(replay_data::rows::TableId::Divisions);
    if !job.inputs_captured || job.context != context {
        job.overflowed = true;
        job.core_finished = true;
        return;
    }
    if job.core_finished {
        return;
    }
    job.core_finished = true;
    let started = qpc();
    job.census.capture_core(memory);
    job.acquisition_ticks[0] += elapsed(started);
    job.overflowed = job.census.overflowed;
    if job.overflowed {
        return;
    }
    let (sources, outcome) = job.census.sources();
    if engine_native::walk::Buffer::Growing(&mut job.source_packet)
        .ensure(sources.len())
        .is_err()
    {
        job.overflowed = true;
        return;
    }
    job.source_packet[..sources.len()].copy_from_slice(sources);

    if !context.details_active {
        return;
    }
    let detail_ticks = job.details.capture(
        memory,
        context.base,
        sources,
        outcome,
        context.detail_capacities,
    );
    for (total, extra) in job.acquisition_ticks[1..].iter_mut().zip(detail_ticks) {
        *total += extra;
    }
}

fn divisions_copy_report(payload: &DivisionsJob) -> DivisionsCopyReport {
    DivisionsCopyReport {
        raw_ok: !payload.overflowed,
        copy_ticks: payload.acquisition_ticks,
    }
}

pub(crate) fn copy_divisions_core_refresh(
    memory: &impl engine_native::Memory,
    ctx: DivisionsCopyContext,
    payload: &mut DivisionsJob,
) -> DivisionsCopyReport {
    copy_divisions_core_pass(payload, memory, ctx);
    divisions_copy_report(payload)
}

#[cfg(any(test, feature = "test-support"))]
/// # Safety
/// Run only in an isolated fixture that owns the collector's process-wide read accounting. No live
/// attachment may consume the fixture's Divisions verdicts.
pub unsafe fn copy_divisions_refresh(
    memory: &impl engine_native::Memory,
    ctx: DivisionsCopyContext,
    payload: &mut DivisionsJob,
) -> DivisionsCopyReport {
    copy_divisions_inputs_pass(payload, memory, ctx);
    copy_divisions_core_pass(payload, memory, ctx);
    divisions_copy_report(payload)
}

pub(super) fn fail_divisions_capture(
    frame: &mut Captured,
    reason: &'static str,
    details_active: bool,
) {
    frame.divisions_result = Outcome::Unavailable { reason };
    frame.extended.install_failed_division_counts(reason);
    if details_active {
        frame.extended.fail_division_details(reason);
    }
}

pub fn decode_divisions_payload(
    frame: &mut Captured,
    _base: usize,
    job: &mut DivisionsJob,
    decode_ticks: &mut [u64; 6],
) {
    let started = qpc();
    frame.divisions_result = job.census.install_rows(&mut frame.divisions);
    frame.extended.install_division_counts(&job.census);
    decode_ticks[0] += elapsed(started);
    if job.context.details_active {
        let started = qpc();
        frame.extended.install_division_detail(&mut job.details);
        job.update_capacity_charge();
        decode_ticks[1] += elapsed(started);
    } else {
        frame.extended.set_divisions_raw_mode(false);
        job.update_capacity_charge();
    }
}
#[cfg(any(test, feature = "test-support"))]
pub mod divisions_test_kit;
