//! World-tension acquisition: the CWorldThreat manager word, its ordered
//! contribution vector and its per-country aggregate cache.
//!
//! The hourly game speed is not a table here: `CGameState+0x4bc` is already read
//! by the authenticated clock read in the same guarded batch as the hour, so it
//! rides on the existing Clock row instead of paying a second read and a second
//! availability outcome. Pause is not recorded; see [archived domain index](#)'s D57 row.
//!
//! Only growth and replay conversion allocate.

use crate::{
    raw::RawLog,
    read_accounting,
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, cthreat_source, cworld_threat, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};

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
            // One manager row; a 1942 save carries 127 sources and about 90
            // aggregate slots, so both stay below the 75% growth threshold.
            rows: [2, 1024, 512],
            reason_bytes: 128 * 1024,
        }
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
    pub(crate) raw_mode: bool,
}

impl Captured {
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

    pub(crate) fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.active = replay::cadence::should_walk(TableId::WorldTension, hour, force_full);
    }

    /// Acquire the manager and both vectors from one authenticated snapshot.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.prepare_hour(hour, force_full);
        self.reset();
        if !self.active {
            self.fail_session_capture(replay::cadence::SKIPPED_REASON);
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
        let first = self.pass(memory, base, game_state);
        self.manager.outcome = first.tables[0];
        self.sources.outcome = first.tables[1];
        self.aggregates.outcome = first.tables[2];
        self.reason_bytes.len = first.bytes_len;
        self.reason_bytes.required = first.required_bytes;
        [elapsed(begin), 0, 0]
    }

    /// One authenticated pass over all three tables into the primary buffers.
    fn pass(&mut self, memory: &impl Memory, base: usize, game_state: usize) -> Pass {
        let reader = match cworld_threat::Reader::new(memory, base, game_state) {
            Ok(reader) => reader,
            Err(reason) => return Pass::unavailable(reason),
        };
        let (manager, sources, aggregates, reasons) = (
            &mut self.manager.rows,
            &mut self.sources.rows,
            &mut self.aggregates.rows,
            &mut self.reason_bytes.first,
        );
        let manager = bounded(reader.manager(manager), manager.len());
        let result = reader.sources(memory, base, sources, reasons);
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
        reason: text(bytes, r.reason_start, r.reason_len).unwrap().to_vec(),
    })
}

fn text(arena: &[u8], start: usize, len: usize) -> Option<&[u8]> {
    arena.get(start..start.checked_add(len)?)
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

fn bounded(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "walker row count exceeds allocated buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "walker marked truncation without missing rows",
        }
    } else {
        outcome
    }
}

// ===========================================================================
// Scheduled session raw-copy split (WorldTension, ThreatSources,
// ThreatCountryAggregates).
//
// The acquisition worker copies raw bytes only: manager pointer, manager header
// (vtable, total, both array headers), per-source pointers with head and
// reason strings and per-aggregate words. No row is decoded during acquisition.
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

/// One refresh of raw session payload. Flat pools recycle capacities and the
/// steady state allocates on neither thread; growth happens on the decode
/// side only (see `crate::decode::recycle_session_job`).
#[derive(Debug, Default)]
pub(crate) struct SessionJob {
    pub log: RawLog<3>,
    pub game_state: usize,
    active: bool,
}

impl SessionJob {
    pub fn new() -> Self {
        Self {
            // Late-campaign worst case is about 127 sources with reasons
            // plus ~440 aggregates; both pools start above that so steady
            // state never overflows.
            log: RawLog::new(1 << 14, 4 << 20),
            game_state: 0,
            active: false,
        }
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.active = false;
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
pub(crate) struct SessionCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    pub copy_ticks: [u64; 3],
    pub raw_bytes: [u64; 3],
}

/// Acquisition-worker copy parameters, bundled to keep dispatch small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SessionCopyContext {
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
    memory: &impl engine::Memory,
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
    memory: &impl engine::Memory,
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
fn copy_source(log: &mut RawLog<3>, memory: &impl engine::Memory, pointer: usize) {
    use engine::{cthreat_source, msvc_string};
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
fn copy_pass(memory: &impl engine::Memory, base: usize, game_state: usize, log: &mut RawLog<3>) {
    use engine::{cgame_state, cpdx_array::Array, cworld_threat};
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
    for slot in 0..aggregates.count {
        let addr = aggregates
            .data
            .checked_add((slot as usize).saturating_mul(8));
        let _ = sspan(log, memory, addr, 8, M_AGGREGATES);
        if log.overflowed {
            return;
        }
    }
}

/// Acquire one refresh into its owned payload.
pub(crate) fn copy_session_refresh(
    memory: &impl engine::Memory,
    ctx: SessionCopyContext,
    first: &mut SessionJob,
) -> SessionCopyReport {
    first.clear();
    first.active = replay::cadence::should_walk(TableId::WorldTension, ctx.hour, ctx.force_full);
    if !first.active {
        return SessionCopyReport::default();
    }
    let started = qpc();
    first.game_state = ctx.game_state;
    copy_pass(memory, ctx.base, ctx.game_state, &mut first.log);
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
    pub(crate) fn fail_session_capture(&mut self, reason: &'static str) {
        let reason = if self.active {
            reason
        } else {
            replay::cadence::SKIPPED_REASON
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
    pub(crate) fn decode_session_payload(
        &mut self,
        base: usize,
        job: &SessionJob,
        decode_ticks: &mut [u64; 3],
    ) {
        self.active = job.active;
        if !job.active {
            self.fail_session_capture(replay::cadence::SKIPPED_REASON);
            return;
        }
        let started = qpc();
        let recorded = crate::raw::Recorded::build(&job.log);
        let walked = self.pass(&recorded, base, job.game_state);
        self.manager.outcome = walked.tables[0];
        self.sources.outcome = walked.tables[1];
        self.aggregates.outcome = walked.tables[2];
        self.reason_bytes.len = walked.bytes_len;
        self.reason_bytes.required = walked.required_bytes;
        decode_ticks[0] += elapsed(started);
        self.raw_mode = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{cgame_state, memory::SliceMemory, msvc_string};
    use replay::rows::TableSchema;

    const BASE: usize = 0x1000;
    const MANAGER: usize = 0x2000;
    const SOURCE_ARRAY: usize = 0x2100;
    const AGGREGATES: usize = 0x2200;
    const FIRST: usize = 0x2400;
    const SECOND: usize = 0x2500;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
    }

    fn array(bytes: &mut [u8], header: usize, data: usize, count: u32) {
        put(bytes, header, &(data as u64).to_le_bytes());
        put(bytes, header + 8, &count.to_le_bytes());
        put(bytes, header + 12, &count.to_le_bytes());
    }

    fn source(bytes: &mut [u8], at: usize, current: i64, reason: &[u8]) {
        put(
            bytes,
            at,
            &(cthreat_source::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(bytes, at + cthreat_source::COUNTRY, &u32::MAX.to_le_bytes());
        put(
            bytes,
            at + cthreat_source::SECOND_COUNTRY,
            &7_u32.to_le_bytes(),
        );
        put(bytes, at + cthreat_source::CURRENT, &current.to_le_bytes());
        put(bytes, at + cthreat_source::FINAL, &i64::MAX.to_le_bytes());
        put(bytes, at + cthreat_source::DAILY, &(-1_i64).to_le_bytes());
        put(bytes, at + cthreat_source::DATE, &i32::MIN.to_le_bytes());
        put(bytes, at + cthreat_source::REASON, reason);
        put(
            bytes,
            at + cthreat_source::REASON + msvc_string::LENGTH,
            &(reason.len() as u64 - 1).to_le_bytes(),
        );
        put(
            bytes,
            at + cthreat_source::REASON + msvc_string::CAPACITY,
            &15_u64.to_le_bytes(),
        );
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x2600];
        put(
            &mut bytes,
            BASE + cgame_state::WORLD_THREAT,
            &(MANAGER as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            MANAGER,
            &(cworld_threat::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            MANAGER + cworld_threat::TOTAL,
            &(-125_682_i64).to_le_bytes(),
        );
        array(
            &mut bytes,
            MANAGER + cworld_threat::SOURCES,
            SOURCE_ARRAY,
            2,
        );
        array(
            &mut bytes,
            MANAGER + cworld_threat::COUNTRY_AGGREGATES,
            AGGREGATES,
            2,
        );
        put(&mut bytes, SOURCE_ARRAY, &(FIRST as u64).to_le_bytes());
        put(&mut bytes, SOURCE_ARRAY + 8, &(SECOND as u64).to_le_bytes());
        put(&mut bytes, AGGREGATES, &i64::MIN.to_le_bytes());
        put(&mut bytes, AGGREGATES + 8, &0_i64.to_le_bytes());
        source(&mut bytes, FIRST, i64::MIN, b"Peace\0");
        source(&mut bytes, SECOND, 0, b"\0");
        bytes
    }

    fn small() -> (Capacities, Captured) {
        let capacities = Capacities {
            rows: [1, 2, 2],
            reason_bytes: 8,
        };
        let captured = Captured::with_capacities(&capacities);
        (capacities, captured)
    }

    fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
        let TableView::Complete(rows) = view else {
            panic!("expected complete rows, got {view:?}")
        };
        rows.rows::<S>().unwrap()
    }

    #[test]
    fn daily_session_skips_native_reads_and_failures_outside_slot_18() {
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, address: usize, _: &mut [u8]) -> bool {
                panic!("inactive session read at {address:#x}");
            }
        }
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let (capacities, mut captured) = small();
        let mut job = SessionJob::new();
        for force_full in [false, true] {
            for hour in 0..24 {
                let ctx = SessionCopyContext {
                    base: 0,
                    game_state: BASE,
                    hour,
                    force_full,
                };
                captured.prepare_hour(hour, force_full);
                if force_full || hour == 18 {
                    let report = copy_session_refresh(&memory, ctx, &mut job);
                    assert!(report.raw_ok);
                    captured.decode_session_payload(0, &job, &mut [0; N]);
                    assert_matches_direct(&captured, &direct(&memory, &capacities));
                } else {
                    let report = copy_session_refresh(&NoReads, ctx, &mut job);
                    assert!(!report.raw_ok && job.log.spans.is_empty());
                    assert_eq!((report.copy_ticks, report.raw_bytes), ([0; N], [0; N]));
                    assert!(!job.grow_for_next());
                    assert_eq!(captured.capture(&NoReads, 0, BASE, hour, false), [0; N]);
                    captured.fail_session_capture("worker failed");
                    let mut ticks = [0; N];
                    captured.decode_session_payload(0, &job, &mut ticks);
                    assert_eq!(ticks, [0; N]);
                    assert_eq!(
                        captured.outcomes(),
                        [Outcome::Unavailable {
                            reason: replay::cadence::SKIPPED_REASON
                        }; N]
                    );
                    assert_eq!(
                        (captured.reason_bytes.len, captured.reason_bytes.required),
                        (0, 0)
                    );
                }
            }
        }
    }

    #[test]
    fn manager_sources_and_aggregates_keep_raw_operands_and_literal_reasons() {
        let (_, mut captured) = small();
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let _ticks = captured.capture(&memory, 0, BASE, 0, true);
        let tables = captured.tables();
        assert_eq!(
            complete::<tables::WorldTension>(&tables[0]),
            &[rows::WorldTensionRow {
                total_raw: -125_682,
                source_count: 2,
                country_aggregate_count: 2,
            }]
        );
        let sources = complete::<tables::ThreatSources>(&tables[1]);
        assert_eq!(sources.len(), 2);
        assert_eq!(
            (
                sources[0].ordinal,
                sources[0].country,
                sources[0].second_country,
                sources[0].current_raw,
                sources[0].final_raw,
                sources[0].daily_raw,
                sources[0].date,
            ),
            (0, u32::MAX, 7, i64::MIN, i64::MAX, -1, i32::MIN)
        );
        assert_eq!(sources[0].reason, b"Peace");
        assert_eq!(sources[1].reason, b"");
        assert_eq!(
            complete::<tables::ThreatCountryAggregates>(&tables[2]),
            &[
                rows::ThreatCountryAggregateRow {
                    country_slot: 0,
                    total_raw: i64::MIN,
                },
                rows::ThreatCountryAggregateRow {
                    country_slot: 1,
                    total_raw: 0,
                },
            ]
        );
    }

    #[test]
    fn capacity_truncation_and_a_broken_root_stay_distinct_and_grow_off_thread() {
        let (mut capacities, mut captured) = small();
        capacities.reason_bytes = 2;
        let mut captured_small = Captured::with_capacities(&capacities);
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let _ticks = captured_small.capture(&memory, 0, BASE, 0, true);
        // The five-byte reason does not fit; its row is omitted and counted.
        assert_eq!(
            captured_small.outcomes()[1],
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert!(matches!(
            captured_small.tables()[1],
            TableView::Truncated { missing: 1, .. }
        ));
        captured_small.grow(&mut capacities);
        assert!(capacities.reason_bytes >= 5);

        let mut broken = fixture();
        put(&mut broken, MANAGER, &0_u64.to_le_bytes());
        let _ticks = captured.capture(
            &SliceMemory {
                base: BASE,
                bytes: &broken,
            },
            0,
            BASE,
            0,
            true,
        );
        for outcome in captured.outcomes() {
            assert_eq!(
                outcome,
                Outcome::Unavailable {
                    reason: "world threat vtable mismatch"
                }
            );
        }
    }

    fn split_capacities() -> Capacities {
        Capacities {
            rows: [2, 8, 8],
            reason_bytes: 64,
        }
    }

    fn direct(memory: &SliceMemory<'_>, capacities: &Capacities) -> Captured {
        let mut captured = Captured::with_capacities(capacities);
        let _ticks = captured.capture(memory, 0, BASE, 0, true);
        captured
    }

    fn split(memory: &impl Memory, capacities: &Capacities) -> (Captured, SessionCopyReport) {
        let mut captured = Captured::with_capacities(capacities);
        let ctx = SessionCopyContext {
            hour: 0,
            force_full: true,
            base: 0,
            game_state: BASE,
        };
        let mut first = SessionJob::new();
        let report = copy_session_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        captured.raw_mode = report.raw_ok;
        let mut ticks = [0u64; 3];
        captured.decode_session_payload(0, &first, &mut ticks);
        (captured, report)
    }

    fn assert_matches_direct(split: &Captured, direct: &Captured) {
        assert_eq!(
            split.manager.outcome, direct.manager.outcome,
            "manager outcome"
        );
        assert_eq!(
            split.sources.outcome, direct.sources.outcome,
            "sources outcome"
        );
        assert_eq!(
            split.aggregates.outcome, direct.aggregates.outcome,
            "aggregates outcome"
        );
        let len = split.manager.outcome.len();
        assert_eq!(
            &split.manager.rows[..len],
            &direct.manager.rows[..len],
            "manager rows"
        );
        let len = split.sources.outcome.len();
        assert_eq!(
            &split.sources.rows[..len],
            &direct.sources.rows[..len],
            "source rows"
        );
        assert_eq!(
            &split.reason_bytes.first[..split.reason_bytes.len],
            &direct.reason_bytes.first[..direct.reason_bytes.len],
            "reason arena"
        );
        assert_eq!(
            split.reason_bytes.required, direct.reason_bytes.required,
            "required"
        );
        let len = split.aggregates.outcome.len();
        assert_eq!(
            &split.aggregates.rows[..len],
            &direct.aggregates.rows[..len],
            "aggregate rows"
        );
    }

    #[test]
    fn session_split_copy_matches_direct_walk() {
        use engine::memory::{CountedMemory, ReadCounts};
        let capacities = split_capacities();
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let direct = direct(&memory, &capacities);
        let memory = CountedMemory::new(memory);
        let capture = read_accounting::Capture::new();
        let (split, _report) = split(&memory, &capacities);
        let counts = capture.finish();
        assert_matches_direct(&split, &direct);
        assert_eq!(split.manager.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.sources.outcome, Outcome::Rows { len: 2 });
        assert_eq!(split.aggregates.outcome, Outcome::Rows { len: 2 });
        let anchor = TableId::WorldTension;
        let mut expected = read_accounting::ZERO;
        // Manager word/header, two source pointers/52-byte heads/32-byte
        // inline reasons, two aggregates: ten requests, 272 bytes at one anchor.
        expected[anchor.index()] = ReadCounts {
            copy_calls: 10,
            spans: 10,
            bytes: 272,
            ..ReadCounts::ZERO
        };
        assert_eq!(counts, expected);
        let mut job = SessionJob::default();
        let mut refreshes = 0;
        for hour in 0..24 {
            if replay::cadence::is_refresh(anchor, hour) {
                refreshes += 1;
                continue;
            }
            let capture = read_accounting::Capture::new();
            let skipped = copy_session_refresh(
                &memory,
                SessionCopyContext {
                    base: 0,
                    game_state: BASE,
                    hour,
                    force_full: false,
                },
                &mut job,
            );
            assert_eq!(capture.finish(), read_accounting::ZERO);
            assert!(!skipped.raw_ok && !job.grow_for_next());
            assert_eq!((skipped.copy_ticks, skipped.raw_bytes), ([0; N], [0; N]));
        }
        assert_eq!(refreshes, 1);
        let measured = counts[anchor.index()];
        let per_refresh = [
            measured.copy_calls,
            measured.scattered_calls,
            measured.record_calls,
            measured.plan_calls,
            measured.spans,
            measured.bytes,
        ];
        println!(
            "daily_fixture cohort=session_split_copy_matches_direct_walk anchor={anchor:?} counters=[copy,scattered,records,plans,spans,bytes] per_refresh={per_refresh:?} before_24h={:?} after_24h={per_refresh:?} basis=unchanged_refresh_x_24_legacy_hourly_vs_1_daily",
            per_refresh.map(|value| value * 24)
        );
    }

    #[test]
    fn session_split_copy_matches_direct_walk_under_truncation() {
        let capacities = Capacities {
            rows: [1, 1, 1],
            reason_bytes: 2,
        };
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let mut direct = Captured::with_capacities(&capacities);
        let _ticks = direct.capture(&memory, 0, BASE, 0, true);
        let (split, _report) = {
            let mut captured = Captured::with_capacities(&capacities);
            let ctx = SessionCopyContext {
                hour: 0,
                force_full: true,
                base: 0,
                game_state: BASE,
            };
            let mut first = SessionJob::new();
            let report = copy_session_refresh(&memory, ctx, &mut first);
            assert!(report.raw_ok);
            captured.raw_mode = true;
            let mut ticks = [0u64; 3];
            captured.decode_session_payload(0, &first, &mut ticks);
            (captured, report)
        };
        assert_matches_direct(&split, &direct);
        assert!(matches!(split.sources.outcome, Outcome::Truncated { .. }));
    }

    #[test]
    fn session_staging_overflow_falls_back_to_typed_walk() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let ctx = SessionCopyContext {
            hour: 0,
            force_full: true,
            base: 0,
            game_state: BASE,
        };
        let mut tiny = SessionJob {
            log: crate::raw::RawLog::new(8, 8),
            game_state: 0,
            ..Default::default()
        };
        let report = copy_session_refresh(&memory, ctx, &mut tiny);
        assert!(!report.raw_ok);
        assert!(tiny.log.overflowed);
        assert!(tiny.grow_for_next());
    }

    #[test]
    fn session_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/one/exact-fit/overflow-by-one/massive run typed AND
        // copy→decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        // Fixture holds 1 manager, 2 sources, 2 aggregates; exact-fit is
        // rows [1,2,2] with 8 reason bytes. u32 unread saturation is not
        // reachable with 2 rows and is covered by the walk's own saturating
        // arithmetic.
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        for (rows, reason) in [
            ([0, 0, 0], 0),
            ([1, 1, 1], 2),
            ([1, 2, 2], 8),
            ([2, 3, 3], 8),
            ([8, 8, 8], 64),
        ] {
            let capacities = Capacities {
                rows,
                reason_bytes: reason,
            };
            let mut direct = Captured::with_capacities(&capacities);
            let _ticks = direct.capture(&memory, 0, BASE, 0, true);
            let mut split = Captured::with_capacities(&capacities);
            let ctx = SessionCopyContext {
                hour: 0,
                force_full: true,
                base: 0,
                game_state: BASE,
            };
            let mut first = SessionJob::new();
            let report = copy_session_refresh(&memory, ctx, &mut first);
            assert!(report.raw_ok, "rows={rows:?}");
            split.raw_mode = true;
            let mut ticks = [0u64; 3];
            split.decode_session_payload(0, &first, &mut ticks);
            assert_matches_direct(&split, &direct);
        }
        // Exact-fit arm pins Rows with all rows present.
        let capacities = Capacities {
            rows: [1, 2, 2],
            reason_bytes: 8,
        };
        let mut direct = Captured::with_capacities(&capacities);
        let _ = direct.capture(&memory, 0, BASE, 0, true);
        assert_eq!(direct.manager.outcome, Outcome::Rows { len: 1 });
        assert_eq!(direct.sources.outcome, Outcome::Rows { len: 2 });
        assert_eq!(direct.aggregates.outcome, Outcome::Rows { len: 2 });
    }

    #[test]
    fn session_copy_sizes_cover_documented_read_set() {
        // S3: every Memory::copy size the session copy pass issues, recorded
        // through the real pass. The pass bulk-copies (32 B string headers,
        // 52/64 B manager/aggregate structs, 8 B words) wider than the
        // walk's scattered reads (4 B keys, 8 B words, 1 B flags inside);
        // sub-ranges serve from the enclosing bulk span at replay (proved
        // by session_bulk_header_serves_enclosed_length_word). Alignment
        // follows the engine layout (structs 8-aligned, headers 8-aligned).
        use engine::Memory;
        use std::collections::BTreeSet;
        struct Recorder<'a> {
            inner: SliceMemory<'a>,
            sizes: std::cell::RefCell<BTreeSet<usize>>,
        }
        impl Memory for Recorder<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.sizes.borrow_mut().insert(output.len());
                self.inner.copy(address, output)
            }
        }
        let bytes = fixture();
        let memory = Recorder {
            inner: SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            sizes: std::cell::RefCell::new(BTreeSet::new()),
        };
        let mut job = SessionJob::new();
        copy_pass(&memory, 0, BASE, &mut job.log);
        assert!(job.log.raw_bytes.iter().any(|&b| b > 0));
        let sizes = memory.sizes.borrow().clone();
        for required in [8, 32, 52, 64] {
            assert!(
                sizes.contains(&required),
                "missing size {required} in {sizes:?}"
            );
        }
    }

    #[test]
    fn session_worst_case_pools_start_above_late_campaign_caps() {
        // Late-campaign worst case: 1024 sources + 512 aggregates with
        // 128 KiB reasons. Pools start above that so steady state never
        // overflows; overflow still falls back to the typed walk above.
        let job = SessionJob::new();
        assert!(job.log.spans.capacity() >= 8192);
        assert!(job.log.bytes.capacity() >= (1 << 20));
        let capacities = Capacities::default();
        assert!(capacities.rows[1] >= 1024);
        assert!(capacities.reason_bytes >= 128 * 1024);
    }

    #[test]
    fn session_bulk_header_serves_enclosed_length_word() {
        // S3 containment: the 32 B reason-string header is a bulk span, so
        // the walk's enclosed 8 B length/terminator reads inside it are
        // served from the same recorded bytes (raw.rs bulk rule). Proves
        // the sspan containment claim with overlapping/unaligned sub-reads.
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let mut job = SessionJob::new();
        copy_pass(&memory, 0, BASE, &mut job.log);
        assert!(job.log.has_bulk);
        let recorded = crate::raw::Recorded::build(&job.log);
        // Find a 32 B bulk header span and read an enclosed 8 B word at an
        // unaligned offset inside it.
        let header = job
            .log
            .spans
            .iter()
            .find(|span| span.ok && span.bulk && span.len == 32)
            .expect("bulk string header");
        let mut word = [0u8; 8];
        assert!(recorded.copy(header.addr + 3, &mut word));
        let mut expected = [0u8; 8];
        expected.copy_from_slice(&bytes[header.addr - BASE + 3..header.addr - BASE + 11]);
        assert_eq!(word, expected);
    }
}
