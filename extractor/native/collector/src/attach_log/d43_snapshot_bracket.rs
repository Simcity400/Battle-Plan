use super::d43_snapshot_memory::{
    plan_refusal_code, Domain43AccessTrace, Domain43SnapshotCaptureReceipt,
    Domain43SnapshotLimits, Domain43SnapshotMemory, Domain43SnapshotPlan,
    Domain43TracingMemory, D43_SNAPSHOT_OS_REGION_CAP_CAUSE, D43_SNAPSHOT_TRACE_CAP_CAUSE,
};
use super::*;
use crate::x15_trade_activation_hook::ActivationSnapshot;
use std::cell::Cell;

pub(super) const D43_SNAPSHOT_MODE_LIVE: u64 = 0;
pub(super) const D43_SNAPSHOT_MODE_ARMING: u64 = 1;
pub(super) const D43_SNAPSHOT_MODE_SNAPSHOT: u64 = 2;

/// d43's snapshot custody across frames. NOT a delta pin: the plan is a
/// region table plus a reusable arena, the trace buffer is preallocated
/// capacity for the next arming walk, and `refusal` is the sticky cause that
/// keeps the reader on its live walk for the rest of the session.
#[derive(Default)]
pub(super) struct CompactMarketTradeSnapshotTracker {
    pub(super) limits: Domain43SnapshotLimits,
    plan: Option<Domain43SnapshotPlan>,
    trace_buffer: Option<Domain43AccessTrace>,
    trace_capacity: usize,
    refusal: Option<&'static str>,
}

impl CompactMarketTradeSnapshotTracker {
    pub(super) fn plan(&self) -> Option<&Domain43SnapshotPlan> {
        self.plan.as_ref()
    }

    pub(super) fn refusal(&self) -> Option<&'static str> {
        self.refusal
    }

    pub(super) fn trace_buffer_reads(&self) -> Option<usize> {
        self.trace_buffer.as_ref().map(Domain43AccessTrace::capacity)
    }

    fn next_trace_capacity(&self) -> usize {
        if self.trace_capacity == 0 {
            self.limits.trace_reserve_reads
        } else {
            self.trace_capacity
        }
    }

    fn prepare_trace_buffer(&mut self) {
        if self.refusal.is_some() || self.plan.is_some() || self.trace_buffer.is_some() {
            return;
        }
        let capacity = self.next_trace_capacity().min(self.limits.max_trace_reads).max(1);
        self.trace_capacity = capacity;
        self.trace_buffer = Some(Domain43AccessTrace::with_capacity(capacity));
    }

    fn refuse(&mut self, cause: &'static str) {
        self.refusal = Some(cause);
        self.plan = None;
        self.trace_buffer = None;
    }

    /// Test seam: a plan the next bracket copies from, in place of one armed
    /// from a trace.
    #[cfg(test)]
    pub(super) fn install_plan(&mut self, plan: Domain43SnapshotPlan) {
        self.plan = Some(plan);
        self.trace_buffer = None;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Domain43SnapshotDiag {
    pub mode: u64,
    pub region_count: u64,
    pub byte_count: u64,
    pub copied_region_count: u64,
    pub copied_byte_count: u64,
    pub capture_us: u64,
    pub plan_us: u64,
    pub arm_trace_reads: u64,
    pub trace_capacity: u64,
    pub plan_refused: u64,
    pub plan_refusal_cause: u64,
    pub copy_refused: u64,
    pub region_miss: u64,
    pub miss_reads: u64,
    pub miss_reads_in_span: u64,
    pub first_miss: Option<(u64, usize)>,
    pub widen_slack: u64,
    pub engine_call_mismatch: u64,
    pub probe_regions: u64,
    pub probe_unreadable: u64,
    pub gate_queries: u64,
    pub gate_budget_exhausted: u64,
    pub os_regions: u64,
    pub live_fallback: u64,
}

pub(super) const D43_SNAPSHOT_UNIT_NAMES: [&str; 23] = [
    "d43_snapshot_mode",
    "d43_snapshot_region_count",
    "d43_snapshot_byte_count",
    "d43_snapshot_copied_region_count",
    "d43_snapshot_copied_byte_count",
    "d43_snapshot_capture_us",
    "d43_snapshot_plan_us",
    "d43_snapshot_arm_trace_reads",
    "d43_snapshot_trace_capacity",
    "d43_snapshot_plan_refused",
    "d43_snapshot_plan_refusal_cause",
    "d43_snapshot_copy_refused",
    "d43_snapshot_region_miss",
    "d43_snapshot_miss_reads",
    "d43_snapshot_miss_reads_in_span",
    "d43_snapshot_widen_slack",
    "d43_snapshot_engine_call_mismatch",
    "d43_snapshot_probe_regions",
    "d43_snapshot_probe_unreadable",
    "d43_snapshot_gate_queries",
    "d43_snapshot_gate_budget_exhausted",
    "d43_snapshot_os_regions",
    "d43_snapshot_live_fallback",
];

impl Domain43SnapshotDiag {
    pub(super) fn values(&self) -> [u64; 23] {
        [
            self.mode,
            self.region_count,
            self.byte_count,
            self.copied_region_count,
            self.copied_byte_count,
            self.capture_us,
            self.plan_us,
            self.arm_trace_reads,
            self.trace_capacity,
            self.plan_refused,
            self.plan_refusal_cause,
            self.copy_refused,
            self.region_miss,
            self.miss_reads,
            self.miss_reads_in_span,
            self.widen_slack,
            self.engine_call_mismatch,
            self.probe_regions,
            self.probe_unreadable,
            self.gate_queries,
            self.gate_budget_exhausted,
            self.os_regions,
            self.live_fallback,
        ]
    }

    pub(super) fn units(&self) -> impl Iterator<Item = CompactDomainPerfDiagnosticUnit> {
        D43_SNAPSHOT_UNIT_NAMES
            .into_iter()
            .zip(self.values())
            .map(|(name, value)| CompactDomainPerfDiagnosticUnit { name, value })
            .chain(self.first_miss.into_iter().flat_map(|(address, bytes)| [
                CompactDomainPerfDiagnosticUnit { name: "d43_snapshot_miss_address_sample", value: address },
                CompactDomainPerfDiagnosticUnit { name: "d43_snapshot_miss_width_sample", value: bytes as u64 },
            ]))
    }

    fn with_refusal(mut self, refusal: Option<&'static str>) -> Self {
        if let Some(cause) = refusal {
            self.plan_refused = 1;
            self.plan_refusal_cause = plan_refusal_code(cause);
        }
        self
    }
}

/// The copy's gate: the same question the live reader asks of every read
/// (`range_confirmed_good`: the whole range inside ONE committed region that
/// is neither no-access nor a guard page), asked with no allocation. Regions
/// arrive in address order, so one remembered operating-system region
/// answers every range inside it and a query is spent only when the next
/// region begins; the budget is the plan-time count, so a layout that
/// fragmented after planning refuses the copy instead of running past it.
pub(super) struct Domain43RegionGate {
    start: usize,
    end: usize,
    good: bool,
    queries: u32,
    budget: u32,
    exhausted: bool,
}

impl Domain43RegionGate {
    pub(super) fn new(budget: u32) -> Self {
        Self {
            start: 0,
            end: 0,
            good: false,
            queries: 0,
            budget,
            exhausted: false,
        }
    }

    pub(super) fn queries(&self) -> u32 {
        self.queries
    }

    /// The ordinal of the region the last successful `confirm` was answered
    /// from: one per VirtualQuery, so two spans confirmed inside one region
    /// share it and spans in different regions never do.
    pub(super) fn region_ordinal(&self) -> u32 {
        self.queries
    }

    pub(super) fn exhausted(&self) -> bool {
        self.exhausted
    }

    pub(super) fn confirm(&mut self, address: usize, len: usize) -> bool {
        if len == 0 {
            return false;
        }
        let Some(end) = address.checked_add(len) else {
            return false;
        };
        if self.start <= address && end <= self.end {
            return self.good;
        }
        if self.queries >= self.budget {
            self.exhausted = true;
            return false;
        }
        self.queries += 1;
        let Some(region) = query_region(address) else {
            return false;
        };
        self.start = region.start;
        self.end = region.start.saturating_add(region.size);
        self.good = region.state == MEM_COMMIT
            && region.protect & (PAGE_NOACCESS | PAGE_GUARD) == 0;
        self.good && end <= self.end
    }
}

/// The one engine CALL inside d43's walk, taken in the bracket and served to
/// the decode: `compact_market_trade_modifier_value`'s eligibility callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Domain43EngineCallCapture {
    pub(super) callback: usize,
    pub(super) mask: u32,
}

thread_local! {
    static ENGINE_CALL_OVERRIDE: Cell<Option<Option<Domain43EngineCallCapture>>> =
        const { Cell::new(None) };
    static ENGINE_CALL_MISMATCHES: Cell<u64> = const { Cell::new(0) };
}

/// The eligibility mask for `callback`: the captured value under a snapshot
/// decode, the live call otherwise. A captured value for a DIFFERENT callback
/// (or none, where the decode reached a call the bracket did not) refuses.
pub(super) fn domain43_eligibility_mask(callback: usize) -> Option<u32> {
    match ENGINE_CALL_OVERRIDE.with(Cell::get) {
        None => {
            let callback: unsafe extern "system" fn() -> u32 =
                unsafe { std::mem::transmute(callback) };
            Some(unsafe { callback() })
        }
        Some(Some(captured)) if captured.callback == callback => Some(captured.mask),
        Some(_) => {
            ENGINE_CALL_MISMATCHES.with(|count| count.set(count.get().saturating_add(1)));
            None
        }
    }
}

pub(super) fn with_engine_call_override<R>(
    captured: Option<Domain43EngineCallCapture>,
    f: impl FnOnce() -> R,
) -> (R, u64) {
    struct Restore(Option<Option<Domain43EngineCallCapture>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ENGINE_CALL_OVERRIDE.with(|slot| slot.set(self.0));
        }
    }
    let _restore = Restore(ENGINE_CALL_OVERRIDE.with(|slot| slot.replace(Some(captured))));
    ENGINE_CALL_MISMATCHES.with(|count| count.set(0));
    let out = f();
    (out, ENGINE_CALL_MISMATCHES.with(Cell::get))
}

/// The same resolution `compact_market_trade_modifier_value` performs before
/// it calls -- slot read, zero check, executable check -- over the COPY just
/// taken: the view allocates nothing, where the live read path's span cache
/// can grow inside the bracket. A slot the plan does not cover is no call,
/// and the decode over the same copy reaches none. Then the one call.
fn capture_engine_call(
    state: &CollectorState,
    view: &Domain43SnapshotMemory<'_>,
) -> Option<Domain43EngineCallCapture> {
    let callback = with_d81_memory_override(view, || {
        let callback =
            read_u64_stable(state.base.checked_add(MODIFIER_ELIGIBILITY_CALLBACK_RVA)?)? as usize;
        (callback != 0
            && compact_market_trade_module_executable_address(state, callback) == Some(true))
        .then_some(callback)
    })?;
    let mask = domain43_eligibility_mask(callback)?;
    Some(Domain43EngineCallCapture { callback, mask })
}

pub(super) enum Domain43CapturedArm {
    Plain(CompactMarketTradeRead),
    Traced {
        read: CompactMarketTradeRead,
        trace: Domain43AccessTrace,
    },
    Snapshot {
        plan: Domain43SnapshotPlan,
        receipt: Domain43SnapshotCaptureReceipt,
        gate_queries: u32,
        gate_budget_exhausted: bool,
        activation: Option<ActivationSnapshot>,
        engine_call: Option<Domain43EngineCallCapture>,
        capture_us: u64,
    },
}

pub(super) struct Domain43CapturedRead<'t> {
    state: &'t CollectorState,
    status_flags: u32,
    market: &'t mut CompactMarketTradeTracker,
    snapshot: &'t mut CompactMarketTradeSnapshotTracker,
    started: Instant,
    arm: Domain43CapturedArm,
}

#[cfg(test)]
impl Domain43CapturedRead<'_> {
    /// The engine call the bracket captured, on a snapshot arm.
    pub(super) fn engine_call(&self) -> Option<Option<Domain43EngineCallCapture>> {
        match &self.arm {
            Domain43CapturedArm::Snapshot { engine_call, .. } => Some(*engine_call),
            _ => None,
        }
    }

    /// Stands in for the race the decode guards against: the slot the bracket
    /// read is not the slot the copy holds. Snapshot arms only.
    pub(super) fn override_engine_call(
        &mut self,
        captured: Option<Domain43EngineCallCapture>,
    ) -> bool {
        match &mut self.arm {
            Domain43CapturedArm::Snapshot { engine_call, .. } => {
                *engine_call = captured;
                true
            }
            _ => false,
        }
    }
}

/// The bracket half. With a plan: copy its regions, nothing else. Without one:
/// the live walk, traced when a buffer for the next plan is ready.
pub(super) fn capture_compact_market_trade_snapshot<'t>(
    state: &'t CollectorState,
    status_flags: u32,
    market: &'t mut CompactMarketTradeTracker,
    snapshot: &'t mut CompactMarketTradeSnapshotTracker,
) -> Domain43CapturedRead<'t> {
    let started = Instant::now();
    let arm = if compact_contract::compact_status_is_failure(status_flags) {
        Domain43CapturedArm::Plain(read_compact_market_trade(state, status_flags, market))
    } else if let Some(plan) = snapshot.plan.take() {
        capture_from_plan(state, plan, snapshot.limits.max_os_regions, started)
    } else if let Some(trace) = snapshot.trace_buffer.take() {
        let tracing = Domain43TracingMemory::new(&D81LiveMemory, trace);
        let read = with_d81_memory_override(&tracing, || {
            read_compact_market_trade(state, status_flags, market)
        });
        Domain43CapturedArm::Traced {
            read,
            trace: tracing.into_trace(),
        }
    } else {
        Domain43CapturedArm::Plain(read_compact_market_trade(state, status_flags, market))
    };
    Domain43CapturedRead {
        state,
        status_flags,
        market,
        snapshot,
        started,
        arm,
    }
}

fn capture_from_plan(
    state: &CollectorState,
    mut plan: Domain43SnapshotPlan,
    gate_budget: u32,
    started: Instant,
) -> Domain43CapturedArm {
    let mut gate = Domain43RegionGate::new(gate_budget);
    let receipt = plan.capture_with(|address, out| {
        let address = usize::try_from(address).ok()?;
        (gate.confirm(address, out.len())
            && unsafe { seh_read_exact(address, out.as_mut_ptr(), out.len()) })
        .then(|| gate.region_ordinal())
    });
    let activation = crate::x15_trade_activation_hook::snapshot();
    let engine_call = capture_engine_call(state, &plan.view());
    Domain43CapturedArm::Snapshot {
        plan,
        receipt,
        gate_queries: gate.queries(),
        gate_budget_exhausted: gate.exhausted(),
        activation,
        engine_call,
        capture_us: duration_micros_u64(started.elapsed()),
    }
}

/// Must run before the reader clock guard closes: an incomplete copy needs one live walk.
pub(super) fn resolve_compact_market_trade_snapshot(
    captured: Domain43CapturedRead<'_>,
) -> CompactMarketTradeRead {
    let Domain43CapturedRead {
        state,
        status_flags,
        market,
        snapshot,
        started,
        arm,
    } = captured;
    let mut read = match arm {
        Domain43CapturedArm::Plain(mut read) => {
            snapshot.prepare_trace_buffer();
            read.snapshot = Domain43SnapshotDiag {
                mode: D43_SNAPSHOT_MODE_LIVE,
                trace_capacity: snapshot.trace_buffer_reads().unwrap_or(0) as u64,
                ..Domain43SnapshotDiag::default()
            }
            .with_refusal(snapshot.refusal);
            read
        }
        Domain43CapturedArm::Traced { mut read, trace } => {
            read.snapshot = arm_plan(snapshot, trace);
            read
        }
        Domain43CapturedArm::Snapshot {
            plan,
            receipt,
            gate_queries,
            gate_budget_exhausted,
            activation,
            engine_call,
            capture_us,
        } => decode_from_plan(
            state,
            status_flags,
            market,
            snapshot,
            plan,
            receipt,
            (gate_queries, gate_budget_exhausted),
            activation,
            engine_call,
            capture_us,
        ),
    };
    read.read_time_us = duration_micros_u64(started.elapsed());
    read
}

/// Outside the bracket: the committed operating-system region that holds
/// `address`, so a merged range straddling two regions is cut where they meet,
/// the plan's slack stops at the same line, and the copy asks the gate the
/// question the live reader asked of each read. A region the live gate would
/// refuse -- not committed, no-access, or a guard page -- answers `None`, so
/// the plan never widens into memory the copy cannot take.
fn os_region_bounds(address: u64) -> Option<(u64, u64)> {
    let region = query_region(usize::try_from(address).ok()?)?;
    if region.state != MEM_COMMIT || region.protect & (PAGE_NOACCESS | PAGE_GUARD) != 0 {
        return None;
    }
    let end = region.start.checked_add(region.size)? as u64;
    (end > address).then_some((region.start as u64, end))
}

fn arm_plan(
    snapshot: &mut CompactMarketTradeSnapshotTracker,
    trace: Domain43AccessTrace,
) -> Domain43SnapshotDiag {
    let plan_started = Instant::now();
    let mut diag = Domain43SnapshotDiag {
        mode: D43_SNAPSHOT_MODE_ARMING,
        arm_trace_reads: trace.reads().len() as u64,
        trace_capacity: trace.capacity() as u64,
        ..Domain43SnapshotDiag::default()
    };
    if trace.overflowed() {
        let doubled = trace.capacity().saturating_mul(2);
        if doubled <= snapshot.limits.max_trace_reads {
            snapshot.trace_capacity = doubled;
            snapshot.trace_buffer = Some(Domain43AccessTrace::with_capacity(doubled));
            diag.plan_refused = 1;
            diag.plan_refusal_cause = plan_refusal_code(D43_SNAPSHOT_TRACE_CAP_CAUSE);
        } else {
            snapshot.refuse(D43_SNAPSHOT_TRACE_CAP_CAUSE);
            diag = diag.with_refusal(snapshot.refusal);
        }
    } else {
        let limits = snapshot.limits;
        let planned = Domain43SnapshotPlan::from_trace(trace, limits)
            .and_then(|plan| plan.fit_to_os_regions(limits, os_region_bounds))
            .and_then(|mut plan| {
                // The distinct operating-system regions the plan spans,
                // counted with the bracket's own gate so the two agree; the
                // bracket's query budget is the cap.
                let mut gate = Domain43RegionGate::new(limits.max_os_regions);
                for spec in plan.region_specs() {
                    if let Ok(address) = usize::try_from(spec.address) {
                        gate.confirm(address, spec.len);
                    }
                    if gate.exhausted() {
                        return Err(super::d43_snapshot_memory::Domain43SnapshotPlanRefusal {
                            cause: D43_SNAPSHOT_OS_REGION_CAP_CAUSE,
                            region_count: plan.region_count() as u64,
                            byte_count: plan.byte_count() as u64,
                        });
                    }
                }
                plan.note_os_region_count(gate.queries());
                Ok(plan)
            });
        match planned {
            Ok(plan) => {
                diag.region_count = plan.region_count() as u64;
                diag.byte_count = plan.byte_count() as u64;
                diag.probe_regions = u64::from(plan.probe_region_count());
                diag.os_regions = u64::from(plan.os_region_count());
                diag.widen_slack = plan.widen_slack();
                snapshot.plan = Some(plan);
            }
            Err(refusal) => {
                // The attempted demand rides the refusal, so a capture says
                // what the walk asked for and not only that it was refused.
                diag.region_count = refusal.region_count;
                diag.byte_count = refusal.byte_count;
                snapshot.refuse(refusal.cause);
                diag = diag.with_refusal(snapshot.refusal);
            }
        }
    }
    diag.plan_us = duration_micros_u64(plan_started.elapsed());
    diag
}

#[allow(clippy::too_many_arguments)]
fn decode_from_plan(
    state: &CollectorState,
    status_flags: u32,
    market: &mut CompactMarketTradeTracker,
    snapshot: &mut CompactMarketTradeSnapshotTracker,
    plan: Domain43SnapshotPlan,
    receipt: Domain43SnapshotCaptureReceipt,
    (gate_queries, gate_budget_exhausted): (u32, bool),
    activation: Option<ActivationSnapshot>,
    engine_call: Option<Domain43EngineCallCapture>,
    capture_us: u64,
) -> CompactMarketTradeRead {
    let mut diag = Domain43SnapshotDiag {
        mode: D43_SNAPSHOT_MODE_SNAPSHOT,
        region_count: u64::from(receipt.region_count),
        byte_count: receipt.byte_count,
        copied_region_count: u64::from(receipt.copied_region_count),
        copied_byte_count: receipt.copied_byte_count,
        capture_us,
        probe_regions: u64::from(receipt.probe_region_count),
        probe_unreadable: u64::from(receipt.probe_regions_unreadable),
        gate_queries: u64::from(gate_queries),
        gate_budget_exhausted: u64::from(gate_budget_exhausted),
        os_regions: u64::from(plan.os_region_count()),
        widen_slack: plan.widen_slack(),
        ..Domain43SnapshotDiag::default()
    };
    if receipt.failure_region_ordinal.is_some() || gate_budget_exhausted {
        diag.copy_refused = u64::from(receipt.failure_region_ordinal.is_some());
        snapshot.prepare_trace_buffer();
        drop(plan);
        return live_fallback(state, status_flags, market, diag);
    }
    let mut attempt_market = market.clone();
    let view = plan.view();
    let (read, mismatches) = crate::x15_trade_activation_hook::with_snapshot_override(
        activation,
        || {
            with_engine_call_override(engine_call, || {
                with_d81_memory_override(&view, || {
                    read_compact_market_trade(state, status_flags, &mut attempt_market)
                })
            })
        },
    );
    diag.engine_call_mismatch = mismatches;
    diag.first_miss = view.first_miss();
    diag.miss_reads = view.misses();
    diag.miss_reads_in_span = view.misses_in_span();
    diag.region_miss = u64::from(diag.miss_reads > 0);
    drop(view);
    if mismatches > 0 || diag.miss_reads > 0 {
        drop(read);
        drop(plan);
        snapshot.prepare_trace_buffer();
        return live_fallback(state, status_flags, market, diag);
    }
    *market = attempt_market;
    snapshot.plan = Some(plan);
    let mut read = read;
    read.snapshot = diag;
    read
}

fn live_fallback(
    state: &CollectorState,
    status_flags: u32,
    market: &mut CompactMarketTradeTracker,
    mut diag: Domain43SnapshotDiag,
) -> CompactMarketTradeRead {
    domain43_reset_frame_words();
    let live = Domain43FallbackMemory { deadline: Instant::now() + Duration::from_secs(1) };
    let mut attempt_market = market.clone();
    let mut read = with_d81_memory_override(&live, || {
        read_compact_market_trade(state, status_flags, &mut attempt_market)
    });
    diag.live_fallback = 1;
    if Instant::now() >= live.deadline {
        drop(read);
        domain43_reset_frame_words();
        return domain43_snapshot_unavailable_read(
            status_flags, domain43_degraded_cause::LIVE_FALLBACK_DEADLINE, diag,
        );
    }
    *market = attempt_market;
    read.snapshot = diag;
    read
}

struct Domain43FallbackMemory {
    deadline: Instant,
}

impl super::d81_snapshot_memory::Domain81Memory for Domain43FallbackMemory {
    fn read_into(&self, address: u64, out: &mut [u8]) -> bool {
        Instant::now() < self.deadline
            && super::d81_snapshot_memory::Domain81Memory::read_into(&D81LiveMemory, address, out)
    }
}

fn domain43_snapshot_unavailable_read(
    status_flags: u32,
    cause: u32,
    diag: Domain43SnapshotDiag,
) -> CompactMarketTradeRead {
    domain43_record_degraded_cause(cause);
    CompactMarketTradeRead {
        rows: Vec::new(),
        phase: Domain43PhaseTimes::default(),
        detail_rows: Vec::new(),
        resource_flow_rows: Vec::new(),
        resource_flow_present: false,
        factory_header_rows: Vec::new(),
        factory_header_present: false,
        resource_rows: Vec::new(),
        resource_rows_present: false,
        resource_row_resource_count: 0,
        resource_tokens: Vec::new(),
        classic_deal_rows: Vec::new(),
        classic_deal_country_witnesses: Vec::new(),
        classic_deal_incomplete_owners: Vec::new(),
        classic_deals_present: false,
        activation: None,
        stockpile_countries: Vec::new(),
        stockpile_items: Vec::new(),
        stockpile_expected_country_count: 0,
        stockpile_present: false,
        detail_skipped_rows: 0,
        read_time_us: 0,
        changed_rows: 0,
        status_flags: status_flags | COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
        observed_contract_rows: 0,
        emitted_contract_rows: 0,
        buyer_ptr_valid_rows: 0,
        variant_valid_rows: 0,
        amount_valid_rows: 0,
        partial_field_rows: 0,
        snapshot: diag,
    }
}

#[cfg(test)]
mod fallback_tests {
    use super::*;
    use super::super::d81_snapshot_memory::Domain81Memory;

    #[test]
    fn expired_fallback_refuses_a_read_without_touching_the_destination() {
        let value = 123u64;
        let live = Domain43FallbackMemory { deadline: Instant::now() };
        let mut out = [0xA5; 8];
        assert!(!live.read_into(&value as *const u64 as u64, &mut out));
        assert_eq!(out, [0xA5; 8]);
    }
}
