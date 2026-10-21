use super::{Captured, INITIAL_CAPACITIES, elapsed, extended, qpc};
use engine_native::walk::Outcome;

/// One independently bounded owned division census, coverage counts and five detail tables.
/// No address/span replay or live memory access remains on the decode worker.
#[derive(Debug, Default)]
pub(crate) struct DivisionsJob {
    pub(crate) presentation: Option<std::sync::Arc<crate::acquisition_jobs::PresentationInputs>>,
    pub(crate) census: crate::division_census::Census,
    pub(super) details: crate::division_detail::OwnedDetails,
    pub(super) context: DivisionsCopyContext,
    pub(super) acquisition_ticks: [u64; 6],
    pub(super) inputs_captured: bool,
    pub(super) core_finished: bool,
    pub(super) overflowed: bool,
    pub(super) capacity_charge: crate::raw_memory::Charge,
    pub(super) source_packet: Vec<engine_native::cequipment_variant_pool::DivisionSource>,
    /// The country slots this hour's inputs walk, when parts walk the rest
    /// ([`crate::division_census::Census::capture_inputs_part`]); every slot
    /// otherwise.
    pub(crate) slots: Option<(u32, u32)>,
    /// The joined census's storage to reserve after walking the first part.
    pub(crate) join_capacity: crate::division_census::JoinCapacity,
}
impl DivisionsJob {
    pub(crate) fn new_worker() -> Self {
        Self {
            census: crate::division_census::Census::new_worker(),
            ..Self::default()
        }
    }
    #[cfg(test)]
    pub fn new() -> Self {
        let census = crate::division_census::Census::new();
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
    pub fn grow_for_next(&self) -> bool {
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
        self.capacity_charge.set(crate::raw_memory::Capacity {
            auxiliary_vectors: self.census.allocated_bytes()
                + self.details.allocated_bytes() as u64
                + crate::raw_memory::vector_bytes(&self.source_packet),
            ..Default::default()
        });
    }
    pub fn payload_bytes(&self) -> [u64; 6] {
        let mut bytes = [0; 6];
        bytes[0] = self.census.payload_bytes();
        bytes[1..].copy_from_slice(&self.details.acquired_bytes());
        bytes
    }
    #[cfg(test)]
    pub(super) fn source_outcome(&self, capacity: usize) -> Outcome {
        self.census.outcome(capacity)
    }
    pub(super) fn supply_sources(
        &mut self,
        captured: &mut extended::Captured,
        capacity: usize,
        failure: Option<&'static str>,
    ) {
        if let Some(reason) = failure.or((!self.core_finished || self.overflowed)
            .then_some("division source acquisition could not complete"))
        {
            captured
                .install_division_sources(&mut self.source_packet, Outcome::Unavailable { reason });
            return;
        }
        if self.census.growing() {
            let mut outcome = self.census.outcome(usize::MAX);
            if outcome.len() > self.source_packet.len() {
                outcome = Outcome::Unavailable {
                    reason: "division source packet incomplete",
                };
            }
            captured.install_division_sources(&mut self.source_packet, outcome);
        } else {
            supply_division_census_sources(&self.census, captured, capacity);
        }
    }
}

pub(super) fn supply_division_census_sources(
    census: &crate::division_census::Census,
    captured: &mut extended::Captured,
    capacity: usize,
) {
    let (sources, outcome) = census.sources(capacity);
    for &source in sources {
        captured.visit_division_source(source);
    }
    captured.finish_divisions(outcome);
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct DivisionsCopyReport {
    /// Ready owned payload. A core failure after seed leasing is terminal;
    /// only an input-stage shortage before leasing may use a fresh fallback.
    pub raw_ok: bool,
    pub copy_ticks: [u64; 6],
    pub raw_bytes: [u64; 6],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DivisionsCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub row_capacity: usize,
    pub source_capacity: usize,
    pub detail_capacities: [usize; 5],
    /// Frozen with the input lineage and retained through owned decoding.
    pub details_active: bool,
}
impl Default for DivisionsCopyContext {
    fn default() -> Self {
        let c = extended::production::Capacities::default().rows;
        Self {
            base: 0,
            game_state: 0,
            row_capacity: INITIAL_CAPACITIES[4],
            source_capacity: 8192,
            detail_capacities: [c[2], c[7], c[8], c[9], c[10]],
            details_active: true,
        }
    }
}
/// Initial source acquisition only. No input lease may outlive an earlier use
/// of this job. A native failure remains attached to this exact input attempt.
fn copy_divisions_inputs_pass(
    job: &mut DivisionsJob,
    memory: &impl engine_native::Memory,
    context: DivisionsCopyContext,
    growing: bool,
) {
    let _reads = crate::read_accounting::table(replay_data::rows::TableId::Divisions);
    job.clear();
    job.context = context;
    if !context.details_active {
        job.details.mark_skipped();
    }
    let started = qpc();
    if let (true, Some(slots)) = (growing, job.slots) {
        job.census
            .capture_inputs_part(memory, context.base, context.game_state, slots);
        // Off the game thread: room for the parts the callback joins.
        job.census.reserve_join(job.join_capacity);
    } else if growing {
        job.census
            .capture_inputs_growing(memory, context.base, context.game_state);
    } else {
        job.census
            .capture_inputs(memory, context.base, context.game_state);
    }
    job.acquisition_ticks[0] = elapsed(started);
    job.inputs_captured = true;
    // This is metadata-only capacity admission. capture_core may still discover
    // a private row/header capacity shortage, which cannot authorize a retry
    // after consumers receive a lease.
    job.overflowed = job.census.overflowed;
}

pub(crate) fn copy_divisions_inputs_refresh_growing(
    memory: &impl engine_native::Memory,
    ctx: DivisionsCopyContext,
    payload: &mut DivisionsJob,
) -> bool {
    copy_divisions_inputs_pass(payload, memory, ctx, true);
    !payload.overflowed
}

/// Retain one input packet before any core task leases it. A capacity miss
/// permits fallback only before any consumer has borrowed that packet.
#[cfg(test)]
pub(crate) fn copy_divisions_inputs_refresh(
    memory: &impl engine_native::Memory,
    ctx: DivisionsCopyContext,
    payload: &mut DivisionsJob,
) -> bool {
    copy_divisions_inputs_pass(payload, memory, ctx, false);
    !payload.overflowed
}

/// Consume only the retained input lineage. This stage never clears a Census,
/// reacquires its input packet, or retries an earlier core/detail acquisition.
fn copy_divisions_core_pass(
    job: &mut DivisionsJob,
    memory: &impl engine_native::Memory,
    context: DivisionsCopyContext,
) {
    let _reads = crate::read_accounting::table(replay_data::rows::TableId::Divisions);
    // Context mismatch is a terminal orchestration failure. Keep the original
    // input owner intact so its other consumers can finish.
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
    job.census
        .capture_core(memory, context.row_capacity.max(context.source_capacity));
    job.acquisition_ticks[0] += elapsed(started);
    job.overflowed = job.census.overflowed;
    if job.overflowed {
        return;
    }
    let (sources, outcome) = job.census.sources(if job.census.growing() {
        usize::MAX
    } else {
        context.source_capacity
    });
    if job.census.growing() {
        if engine_native::walk::Buffer::Growing(&mut job.source_packet)
            .ensure(sources.len())
            .is_err()
        {
            job.overflowed = true;
            return;
        }
        job.source_packet[..sources.len()].copy_from_slice(sources);
    }
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

/// Report the one owned acquisition without reading source memory.
fn divisions_copy_report(payload: &DivisionsJob) -> DivisionsCopyReport {
    DivisionsCopyReport {
        raw_ok: !payload.overflowed,
        copy_ticks: payload.acquisition_ticks,
        raw_bytes: payload.payload_bytes(),
    }
}

/// A failure after input leasing is final for this callback; no source retry.
pub(crate) fn copy_divisions_core_refresh(
    memory: &impl engine_native::Memory,
    ctx: DivisionsCopyContext,
    payload: &mut DivisionsJob,
) -> DivisionsCopyReport {
    copy_divisions_core_pass(payload, memory, ctx);
    divisions_copy_report(payload)
}

/// Single composed acquisition for synthetic callers.
#[cfg(test)]
pub(crate) fn copy_divisions_refresh(
    memory: &impl engine_native::Memory,
    ctx: DivisionsCopyContext,
    payload: &mut DivisionsJob,
) -> DivisionsCopyReport {
    copy_divisions_inputs_pass(payload, memory, ctx, false);
    copy_divisions_core_pass(payload, memory, ctx);
    divisions_copy_report(payload)
}

/// The hourly core and coverage counts fail even off the detail tables' slot.
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

/// Worker installation copies compact core rows and transfers owned detail
/// buffers. Callback presentation rows and their source packet survive.
pub(crate) fn decode_divisions_payload(
    frame: &mut Captured,
    _base: usize,
    job: &mut DivisionsJob,
    decode_ticks: &mut [u64; 6],
) {
    let started = qpc();
    frame.divisions_result = match engine_native::walk::Buffer::Growing(&mut frame.divisions)
        .ensure(job.census.outcome(usize::MAX).len())
    {
        Ok(_) => job.census.install_rows(&mut frame.divisions),
        Err(reason) => Outcome::Unavailable { reason },
    };
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
/// Shared synthetic address space for the divisions-group copy/decode
/// harness: two countries plus one alias slot, one province, three
/// divisions (null location, null template and null orders covered),
/// equipment pools with signed amounts, and upgrade groups with requests,
/// one live delivery and produced rows.
#[cfg(test)]
pub(crate) mod divisions_test_kit;
