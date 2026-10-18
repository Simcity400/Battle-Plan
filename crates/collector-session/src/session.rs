//! World-tension acquisition: the CWorldThreat manager word, its ordered
//! contribution vector and its per-country aggregate cache, read every hour.
//! An hour whose tension domain is quiet ([`crate::world_input_changes`]:
//! The world tension total update, which every source writer ends in, did not
//! run since the held read) reads nothing: the acquisition worker plans the
//! hour against the held read and the decode worker publishes it.
//!
//! The hourly game speed is not a table here: it is already read
//! by the authenticated clock read in the same guarded batch as the hour, so it
//! rides on the existing Clock row instead of paying a second read and a second
//! availability outcome. Pause use is not recorded.
//!
//! Worker-owned capture growth and replay conversion allocate.

use crate::world_input_changes::{self as changes, Changes, Domain, Held, Plan};
use crate::{
    clock::qpc,
    raw::RawLog,
    read_accounting,
    table_capture::{Arena, CaptureRows, bounded},
    table_conversion::{table, unavailable},
};
use engine_allocation::{copy_vec, owned_vec};
use engine_native::{Memory, walk::Outcome};
use engine_stability::{cthreat_source, cworld_threat};
use replay_data::{rows, rows::TableId, rows::tables};
use replay_frame::frame::TableView;
use std::sync::Mutex;

pub const N: usize = 3;
pub const TABLES: [TableId; N] = [
    TableId::WorldTension,
    TableId::ThreatSources,
    TableId::ThreatCountryAggregates,
];

/// Root authentication and both array headers are read once per pass and
/// charged to the manager row; the two vectors keep their own read times.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[(
    "world threat manager, sources and aggregates",
    0,
    &[0, 1, 2],
)];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub reason_bytes: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [1, 0, 0],
            reason_bytes: 0,
        }
    }
}
impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let reader = cworld_threat::Reader::new(memory, base, state)?;
        let [sources, aggregates] = reader.counts();
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[0], rows(1, 1));
        raise(&mut self.rows[1], rows(sources, 1));
        raise(&mut self.rows[2], rows(aggregates, 1));
        raise(&mut self.reason_bytes, reader.reason_demand(memory, base)?);
        Ok(())
    }

    /// ThreatCountryAggregates is the engine's dense per-country cache.
    pub fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[2], rows(slots.saturating_add(1), 1));
    }
}

/// The three tables' latest complete read and its reason bytes.
pub struct Tension {
    manager: Vec<cworld_threat::ManagerRow>,
    sources: Vec<cthreat_source::Row>,
    aggregates: Vec<cworld_threat::AggregateRow>,
    reasons: Vec<u8>,
}

impl Default for Tension {
    fn default() -> Self {
        Self::new()
    }
}

impl Tension {
    pub const fn new() -> Self {
        Self {
            manager: Vec::new(),
            sources: Vec::new(),
            aggregates: Vec::new(),
            reasons: Vec::new(),
        }
    }
}

/// The held read. The acquisition worker plans each hour against it; the
/// single FIFO decode worker publishes or replaces it in hour order.
static HELD: Mutex<Held<Tension>> = Mutex::new(Held::new(Tension::new()));

/// Where a job plans and publishes its hours: the hooks' domain and the held
/// read, or a test's own.
#[derive(Clone, Copy)]
pub struct Observed {
    pub changes: &'static dyn Changes,
    pub held: &'static Mutex<Held<Tension>>,
}

impl Default for Observed {
    fn default() -> Self {
        Self {
            changes: &changes::Observer,
            held: &HELD,
        }
    }
}

impl std::fmt::Debug for Observed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Observed")
    }
}

/// One acquisition pass: the three table outcomes plus its reason-arena use.
struct Pass {
    tables: [Outcome; N],
    bytes_len: usize,
    required_bytes: usize,
}

impl Pass {
    fn unavailable(reason: &'static str) -> Self {
        Self {
            tables: [Outcome::Unavailable { reason }; N],
            bytes_len: 0,
            required_bytes: 0,
        }
    }
}

pub struct Captured {
    manager: CaptureRows<cworld_threat::ManagerRow>,
    sources: CaptureRows<cthreat_source::Row>,
    aggregates: CaptureRows<cworld_threat::AggregateRow>,
    reason_bytes: Arena<u8>,
    active: bool,
    /// Raw-copy split gate for the scheduled session family (copy/decode split).
    /// Set when an acquisition worker supplies a valid raw payload; `capture`
    /// then skips the typed walks and the decode worker fills these rows
    /// before the ready handoff. Preserved across `reset` (set per capture,
    /// cleared by the worker after decode).
    pub raw_mode: bool,
}

impl Captured {
    pub fn fail_final_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.prepare_hour(hour, force_full);
        if !self.raw_mode {
            self.fail_session_capture(if self.active {
                reason
            } else {
                replay_data::cadence::SKIPPED_REASON
            });
        }
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; N],
            reason_bytes: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            manager: CaptureRows::new(capacity.rows[0]),
            sources: CaptureRows::new(capacity.rows[1]),
            aggregates: CaptureRows::new(capacity.rows[2]),
            reason_bytes: Arena::new(capacity.reason_bytes),
            active: true,
            raw_mode: false,
        }
    }

    pub fn reset(&mut self) {
        self.manager.reset();
        self.sources.reset();
        self.aggregates.reset();
        self.reason_bytes.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.manager.allocated_bytes()
            + self.sources.allocated_bytes()
            + self.aggregates.allocated_bytes()
            + self.reason_bytes.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.manager.grow(&mut capacity.rows[0]);
        self.sources.grow(&mut capacity.rows[1]);
        self.aggregates.grow(&mut capacity.rows[2]);
        self.reason_bytes.grow(&mut capacity.reason_bytes);
    }

    pub fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.active = replay_data::cadence::should_walk(TableId::WorldTension, hour, force_full);
    }

    /// Acquire the manager and both vectors from one authenticated snapshot.
    #[cfg(any(test, feature = "test-support"))]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.capture_storage(memory, base, game_state, hour, force_full, false)
    }

    /// Worker-owned rows and reasons grow from original headers before publication.
    pub fn capture_growing(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.capture_storage(memory, base, game_state, hour, force_full, true)
    }

    fn capture_storage(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        growing: bool,
    ) -> [u64; N] {
        self.prepare_hour(hour, force_full);
        self.reset();
        if !self.active {
            self.fail_session_capture(replay_data::cadence::SKIPPED_REASON);
            return [0; N];
        }
        // On a scheduled refresh, the acquisition worker stages raw bytes via
        // `copy_session_refresh`; the decode worker fills the typed rows before
        // the ready handoff.
        if self.raw_mode {
            // Refresh payload staged: leave the reset outcomes for the decode
            // worker, which installs identical rows, outcomes and arenas.
            return [0; N];
        }

        let _reads = read_accounting::table(TableId::WorldTension);
        let begin = qpc();
        let first = self.pass(memory, base, game_state, growing);
        self.manager.outcome = first.tables[0];
        self.sources.outcome = first.tables[1];
        self.aggregates.outcome = first.tables[2];
        self.reason_bytes.len = first.bytes_len;
        self.reason_bytes.required = first.required_bytes;
        [elapsed(begin), 0, 0]
    }

    /// One authenticated pass over all three tables into the primary buffers.
    fn pass(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        growing: bool,
    ) -> Pass {
        let reader = match cworld_threat::Reader::new(memory, base, game_state) {
            Ok(reader) => reader,
            Err(reason) => return Pass::unavailable(reason),
        };
        if growing {
            // World-tension aggregates use the full current country census.
            // These are the original owned header extents.
            let grown = (|| {
                engine_native::walk::Buffer::Growing(&mut self.manager.rows).ensure(1)?;
                engine_native::walk::Buffer::Growing(&mut self.aggregates.rows)
                    .ensure(reader.counts()[1])?;
                Ok::<(), &'static str>(())
            })();
            if let Err(reason) = grown {
                return Pass::unavailable(reason);
            }
        }
        let (manager, sources, aggregates, reasons) = (
            &mut self.manager.rows,
            &mut self.sources.rows,
            &mut self.aggregates.rows,
            &mut self.reason_bytes.first,
        );
        let manager = bounded(reader.manager(manager), manager.len());
        let result = if growing {
            reader.sources_growing(memory, base, sources, reasons)
        } else {
            reader.sources(memory, base, sources, reasons)
        };
        let sources = bounded(result.outcome, sources.len());
        let aggregates = bounded(reader.aggregates(memory, aggregates), aggregates.len());
        Pass {
            tables: [manager, sources, aggregates],
            bytes_len: result.bytes_len,
            required_bytes: result.required_bytes,
        }
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.manager.outcome,
            self.sources.outcome,
            self.aggregates.outcome,
        ]
    }

    /// Spool-thread conversion; raw operands and incomplete status survive.
    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        [
            table::<tables::WorldTension, _>(outcomes[0], &self.manager.rows, |r| {
                rows::WorldTensionRow {
                    total_raw: r.total_raw,
                    source_count: r.source_count,
                    country_aggregate_count: r.country_aggregate_count,
                }
            }),
            sources(outcomes[1], &self.sources, &self.reason_bytes),
            table::<tables::ThreatCountryAggregates, _>(outcomes[2], &self.aggregates.rows, |r| {
                rows::ThreatCountryAggregateRow {
                    country_slot: r.country_slot,
                    total_raw: r.total_raw,
                }
            }),
        ]
    }
}

fn sources(
    outcome: Outcome,
    rows_source: &CaptureRows<cthreat_source::Row>,
    arena: &Arena<u8>,
) -> TableView {
    if let Outcome::Unavailable { reason } = outcome {
        return unavailable(reason);
    }
    let Some(source) = rows_source.rows.get(..outcome.len()) else {
        return unavailable("walker row count exceeds allocated buffer");
    };
    let Some(bytes) = arena.first.get(..arena.len) else {
        return unavailable("threat source reasons lie outside captured arena");
    };
    if source
        .iter()
        .any(|row| text(bytes, row.reason_start, row.reason_len).is_none())
    {
        return unavailable("threat source reasons lie outside captured arena");
    }
    table::<tables::ThreatSources, _>(outcome, source, |r| rows::ThreatSourceRow {
        ordinal: r.ordinal,
        country: r.country,
        second_country: r.second_country,
        current_raw: r.current_raw,
        final_raw: r.final_raw,
        daily_raw: r.daily_raw,
        date: r.date,
        reason: owned_vec(copy_vec(text(bytes, r.reason_start, r.reason_len).unwrap())),
    })
}

fn text(arena: &[u8], start: usize, len: usize) -> Option<&[u8]> {
    arena.get(start..start.checked_add(len)?)
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

// ===========================================================================
// Scheduled session raw-copy split (WorldTension, ThreatSources,
// ThreatCountryAggregates).
//
// The acquisition worker copies raw bytes only: manager pointer, manager header
// (vtable, total, both array headers), per-source pointers with head and
// reason strings and the aggregate cache in one span. No row is decoded during acquisition.
// The single FIFO decode worker replays the `cworld_threat::Reader` walk against the
// recorded spans (`crate::raw::Recorded`), so rows, ordinals, arenas,
// truncation and failure reasons are identical by construction (see
// `crate::raw` for the mirror rules).
//
// The mirror traverses structurally and records every discoverable span,
// never aborting except on overflow: validation, precedence and error scope
// are local parses, so the replay derives every outcome from the same
// recorded spans and later extras are harmless.
// ===========================================================================

/// Fault-attribution bits, one per session table in family order.
const M_SOURCES: u32 = 0x02;
const M_AGGREGATES: u32 = 0x04;
const M_ALL: u32 = 0x07;

/// One refresh of raw session payload. Worker acquisition grows from original
/// requests and owned decoding grows the rows before publication.
#[derive(Debug, Default)]
pub struct SessionJob {
    pub log: RawLog<3>,
    pub game_state: usize,
    active: bool,
    // Fixed mode exists for callback-capacity and fault fixtures only.
    worker_growth: bool,
    /// This hour against the held read: a carried hour copies nothing.
    plan: Plan,
    pub observed: Observed,
}

impl SessionJob {
    pub fn new() -> Self {
        Self {
            log: RawLog::new(0, 0),
            game_state: 0,
            active: false,
            worker_growth: true,
            plan: Plan::READ,
            observed: Observed::default(),
        }
    }

    pub fn is_provisioned(&self) -> bool {
        self.worker_growth || self.log.spans.capacity() != 0
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.active = false;
        self.plan = Plan::READ;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    /// Fixed fixtures retain demand for their next independent observation.
    /// Production workers grow before each original span is acquired.
    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SessionCopyReport {
    /// Fixed fixtures can report staging overflow. Production acquisitions
    /// retain their original source observation without a live retry.
    pub raw_ok: bool,
    pub copy_ticks: [u64; 3],
    pub raw_bytes: [u64; 3],
}

/// Acquisition-worker copy parameters, bundled to keep dispatch small.
#[derive(Debug, Clone, Copy)]
pub struct SessionCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
}

fn pool_u64(log: &RawLog<3>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads. Bulk spans
/// serve enclosed sub-reads (e.g., 8-byte length words inside a 32-byte
/// string header) via `Recorded` containment, so superset headers cover the
/// walk's scattered reads.
fn sspan(
    log: &mut RawLog<3>,
    memory: &impl engine_native::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take_bulk(memory, addr?, len, mask)
}

/// Heap payload and terminator takes for one parsed reason literal. Inline
/// strings ride in the already-copied header span.
fn copy_reason_literal(
    log: &mut RawLog<3>,
    memory: &impl engine_native::Memory,
    len: usize,
    source: usize,
    inline: bool,
) {
    if inline {
        return;
    }
    if len != 0 {
        let _ = sspan(log, memory, Some(source), len, M_SOURCES);
    }
    let _ = sspan(log, memory, source.checked_add(len), 1, M_SOURCES);
}

/// Mirror of one threat source's read set: head plus reason string header
/// with heap payload. Takes are unconditional; the replay validates vtables
/// and string headers itself.
fn copy_source(log: &mut RawLog<3>, memory: &impl engine_native::Memory, pointer: usize) {
    use engine_native::msvc_string;
    use engine_stability::cthreat_source;
    let head = match sspan(log, memory, Some(pointer), 0x34, M_SOURCES) {
        Some(start) => start,
        None => return,
    };
    let reason_addr = match pointer.checked_add(cthreat_source::REASON) {
        Some(addr) => addr,
        None => return,
    };
    let header_start = match sspan(log, memory, Some(reason_addr), 32, M_SOURCES) {
        Some(start) => start,
        None => return,
    };
    let mut header = [0u8; 32];
    header.copy_from_slice(&log.bytes[header_start..header_start + 32]);
    let len = u64::from_le_bytes(header[16..24].try_into().unwrap());
    let capacity = u64::from_le_bytes(header[24..32].try_into().unwrap());
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return;
    }
    let Ok(len) = usize::try_from(len) else {
        return;
    };
    let inline = capacity == msvc_string::INLINE_CAPACITY;
    if inline {
        // Terminator lives in the already-copied header; the replay checks
        // it locally without extra spans.
        let _ = head;
        return;
    }
    let source = match usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
        .ok()
        .filter(|&s| s != 0)
    {
        Some(s) => s,
        None => return,
    };
    if source.checked_add(len).is_none() {
        return;
    }
    copy_reason_literal(log, memory, len, source, false);
}

/// One structural pass: manager pointer, manager header, sources with
/// reasons and aggregates.
/// Stops at the first typed abort point; later extras are harmless.
fn copy_pass(
    memory: &impl engine_native::Memory,
    base: usize,
    game_state: usize,
    log: &mut RawLog<3>,
) {
    use engine_native::{cgame_state, cpdx_array::Array};
    use engine_stability::cworld_threat;
    let _reads = read_accounting::table(TableId::WorldTension);
    let _ = base;
    log.has_bulk = true;
    let ptr_start = match sspan(
        log,
        memory,
        game_state.checked_add(cgame_state::WORLD_THREAT),
        8,
        M_ALL,
    ) {
        Some(start) => start,
        None => return,
    };
    let pointer = match usize::try_from(pool_u64(log, ptr_start))
        .ok()
        .filter(|&p| p != 0)
    {
        Some(p) => p,
        None => return,
    };
    let header_start = match sspan(log, memory, Some(pointer), 0x40, M_ALL) {
        Some(start) => start,
        None => return,
    };
    let mut head = [0u8; 0x40];
    head.copy_from_slice(&log.bytes[header_start..header_start + 0x40]);
    let header = |offset: usize| -> [u8; 16] { head[offset..offset + 16].try_into().unwrap() };
    let sources = match Array::from_header(header(cworld_threat::SOURCES), 8) {
        Some(a) => a,
        None => return,
    };
    let aggregates = match Array::from_header(header(cworld_threat::COUNTRY_AGGREGATES), 8) {
        Some(a) => a,
        None => return,
    };
    for ordinal in 0..sources.count {
        let addr = sources
            .data
            .checked_add((ordinal as usize).saturating_mul(8));
        let src = match sspan(log, memory, addr, 8, M_SOURCES)
            .and_then(|s| usize::try_from(pool_u64(log, s)).ok().filter(|&p| p != 0))
        {
            Some(p) => p,
            None => continue,
        };
        copy_source(log, memory, src);
        if log.overflowed {
            return;
        }
    }
    // One span for the contiguous cache: the walk fails the whole table at
    // any unreadable slot, as it does when this span fails.
    let len = (aggregates.count as usize).saturating_mul(cworld_threat::AGGREGATE_STRIDE);
    if len != 0 {
        let _ = sspan(log, memory, Some(aggregates.data), len, M_AGGREGATES);
    }
}

/// Acquire one refresh into its owned payload.
pub fn copy_session_refresh_growing(
    memory: &impl engine_native::Memory,
    ctx: SessionCopyContext,
    first: &mut SessionJob,
) -> SessionCopyReport {
    first.worker_growth = true;
    copy_session_refresh(memory, ctx, first)
}

pub fn copy_session_refresh(
    memory: &impl engine_native::Memory,
    ctx: SessionCopyContext,
    first: &mut SessionJob,
) -> SessionCopyReport {
    first.clear();
    first.active =
        replay_data::cadence::should_walk(TableId::WorldTension, ctx.hour, ctx.force_full);
    if !first.active {
        return SessionCopyReport::default();
    }
    let started = qpc();
    let Observed { changes, held } = first.observed;
    first.plan = held.try_lock().map_or(Plan::READ, |held| {
        held.plan(Domain::Tension, changes, ctx.force_full)
    });
    first.game_state = ctx.game_state;
    if first.plan.carry {
        return SessionCopyReport {
            raw_ok: true,
            copy_ticks: [elapsed(started), 0, 0],
            raw_bytes: [0; 3],
        };
    }
    if first.worker_growth {
        first.log.enable_worker_growth();
    }
    copy_pass(memory, ctx.base, ctx.game_state, &mut first.log);
    first.plan = first.plan.read(Domain::Tension, changes);
    let mut ticks = [0u64; 3];
    ticks[0] = elapsed(started);
    let raw_ok = !first.log.overflowed;
    SessionCopyReport {
        raw_ok,
        copy_ticks: ticks,
        raw_bytes: first.log.raw_bytes,
    }
}

impl Captured {
    /// Mark an accepted acquisition failure without retrying live memory.
    pub fn fail_session_capture(&mut self, reason: &'static str) {
        let reason = if self.active {
            reason
        } else {
            replay_data::cadence::SKIPPED_REASON
        };
        self.manager.outcome = Outcome::Unavailable { reason };
        self.sources.outcome = Outcome::Unavailable { reason };
        self.aggregates.outcome = Outcome::Unavailable { reason };
        self.reason_bytes.reset();
        self.raw_mode = false;
    }

    /// Decode-worker interpretation of a scheduled raw payload into the pooled
    /// typed rows. Outcomes and arena bytes match the typed walk exactly
    /// because the identical walk runs against the recorded spans. Clears `raw_mode`
    /// so the next capture starts clean.
    pub fn decode_session_payload(
        &mut self,
        base: usize,
        job: &SessionJob,
        decode_ticks: &mut [u64; 3],
    ) {
        self.active = job.active;
        if !job.active {
            self.fail_session_capture(replay_data::cadence::SKIPPED_REASON);
            return;
        }
        let started = qpc();
        if job.plan.carry {
            self.install_held(job.observed.held, job.plan);
        } else {
            let recorded = crate::raw::Recorded::build(&job.log);
            let walked = self.pass(&recorded, base, job.game_state, job.worker_growth);
            self.manager.outcome = walked.tables[0];
            self.sources.outcome = walked.tables[1];
            self.aggregates.outcome = walked.tables[2];
            self.reason_bytes.len = walked.bytes_len;
            self.reason_bytes.required = walked.required_bytes;
            self.hold(job.observed, job.plan);
        }
        decode_ticks[0] += elapsed(started);
        self.raw_mode = false;
    }

    /// A carried hour publishes the held read, when it is still the one the
    /// worker planned against (a failed decode in between forgot it).
    fn install_held(&mut self, held: &Mutex<Held<Tension>>, plan: Plan) {
        let Some(held) = held.lock().ok().filter(|held| held.holds(plan)) else {
            self.fail_session_capture("held world tension read no longer held");
            return;
        };
        let tension = held.carried(Domain::Tension);
        let grown = (|| {
            use engine_native::walk::Buffer::Growing;
            Growing(&mut self.manager.rows).ensure(tension.manager.len())?;
            Growing(&mut self.sources.rows).ensure(tension.sources.len())?;
            Growing(&mut self.aggregates.rows).ensure(tension.aggregates.len())?;
            Growing(&mut self.reason_bytes.first).ensure(tension.reasons.len())?;
            Ok::<(), &'static str>(())
        })();
        if let Err(reason) = grown {
            self.fail_session_capture(reason);
            return;
        }
        let rows = |len| Outcome::Rows { len };
        self.manager.rows[..tension.manager.len()].copy_from_slice(&tension.manager);
        self.sources.rows[..tension.sources.len()].copy_from_slice(&tension.sources);
        self.aggregates.rows[..tension.aggregates.len()].copy_from_slice(&tension.aggregates);
        self.reason_bytes.first[..tension.reasons.len()].copy_from_slice(&tension.reasons);
        self.manager.outcome = rows(tension.manager.len());
        self.sources.outcome = rows(tension.sources.len());
        self.aggregates.outcome = rows(tension.aggregates.len());
        self.reason_bytes.len = tension.reasons.len();
        self.reason_bytes.required = tension.reasons.len();
    }

    /// After a decoded read: compare it on the domain's comparison hour and
    /// hold it when the worker saw the token stand still across its copy.
    fn hold(&self, observed: Observed, plan: Plan) {
        let Ok(mut held) = observed.held.lock() else {
            return;
        };
        let read = match self.outcomes() {
            [
                Outcome::Rows { len: manager },
                Outcome::Rows { len: sources },
                Outcome::Rows { len: aggregates },
            ] => Some((
                &self.manager.rows[..manager],
                &self.sources.rows[..sources],
                &self.aggregates.rows[..aggregates],
                &self.reason_bytes.first[..self.reason_bytes.len],
            )),
            _ => None,
        };
        held.settle(
            Domain::Tension,
            observed.changes,
            plan,
            read.is_some(),
            |tension| {
                read.is_some_and(|read| {
                    (
                        &tension.manager[..],
                        &tension.sources[..],
                        &tension.aggregates[..],
                        &tension.reasons[..],
                    ) != read
                })
            },
            |tension| {
                read.is_some_and(|(manager, sources, aggregates, reasons)| {
                    changes::assign(&mut tension.manager, manager)
                        && changes::assign(&mut tension.sources, sources)
                        && changes::assign(&mut tension.aggregates, aggregates)
                        && changes::assign(&mut tension.reasons, reasons)
                })
            },
        );
    }
}

#[cfg(test)]
mod tests;
