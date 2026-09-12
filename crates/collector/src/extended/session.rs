//! World-tension acquisition: the CWorldThreat manager word, its ordered
//! contribution vector and its per-country aggregate cache.
//!
//! The hourly game speed is not a table here: `CGameState+0x4bc` is already read
//! by the authenticated clock read in the same guarded batch as the hour, so it
//! rides on the existing Clock row instead of paying a second read and a second
//! availability outcome. Pause is not recorded; see PORTING.md's D57 row.
//!
//! Only growth and replay conversion allocate.

use crate::{
    raw::RawLog,
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
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("world threat manager, sources and aggregates", 0, &[0, 1, 2])];

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
    /// Raw-copy split gate for the hourly session family (copy/decode split).
    /// Set by the game thread every hour with a valid raw payload; `capture`
    /// then skips the typed walks and the decode worker fills these rows
    /// before the ready handoff. Preserved across `reset` (set per hour,
    /// cleared by the worker after decode).
    pub(crate) raw_mode: bool,
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            manager: CaptureRows::new(capacity.rows[0], successful[0]),
            sources: CaptureRows::new(capacity.rows[1], successful[1]),
            aggregates: CaptureRows::new(capacity.rows[2], successful[2]),
            reason_bytes: Arena::new(capacity.reason_bytes, successful[1]),
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

    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        assert_eq!(successful.len(), N);
        self.manager.grow(&mut capacity.rows[0], successful[0]);
        self.sources.grow(&mut capacity.rows[1], successful[1]);
        self.aggregates.grow(&mut capacity.rows[2], successful[2]);
        self.reason_bytes
            .grow(&mut capacity.reason_bytes, successful[1]);
    }

    /// The manager is resolved afresh for each pass, and its header is rechecked
    /// after the walk: a total or array header that moved mid-walk makes every
    /// row of this pass unavailable rather than publishing a mixed census.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        self.reset();
        // Raw-copy split (hourly session family): every hour the game thread
        // stages raw bytes via `copy_session_refresh` and the decode worker
        // fills the typed rows before the ready handoff, so both typed passes
        // skip here. Matched flags and costs come from the raw compare,
        // applied by the caller after this returns.
        if self.raw_mode {
            // Refresh payload staged: leave the reset outcomes for the decode
            // worker, which installs identical rows, outcomes and arenas.
            return ([false; N], [0; N]);
        }
        let mut matched = [false; N];
        let mut ticks = [0; N];

        let begin = qpc();
        let first = self.pass(memory, base, game_state, false);
        self.manager.outcome = first.tables[0];
        self.sources.outcome = first.tables[1];
        self.aggregates.outcome = first.tables[2];
        self.reason_bytes.len = first.bytes_len;
        self.reason_bytes.required = first.required_bytes;
        ticks[0] = elapsed(begin);

        if needed.iter().any(|needed| *needed) {
            let begin = qpc();
            let second = self.pass(memory, base, game_state, true);
            // A changed reason arena is a source mismatch even when the fixed
            // fields of every row happen to be equal.
            let reasons_matched = self
                .reason_bytes
                .compare(second.bytes_len, second.required_bytes);
            self.manager.checked = needed[0];
            self.sources.checked = needed[1];
            self.aggregates.checked = needed[2];
            matched[0] = needed[0] && self.manager.compare(second.tables[0]);
            let sources_matched = self.sources.compare(second.tables[1]);
            if !reasons_matched {
                self.sources.mismatches += 1;
            }
            matched[1] = needed[1] && sources_matched && reasons_matched;
            matched[2] = needed[2] && self.aggregates.compare(second.tables[2]);
            ticks[0] = ticks[0].saturating_add(elapsed(begin));
        }
        (matched, ticks)
    }

    /// One authenticated pass over all three tables into the chosen buffers.
    fn pass(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        second: bool,
    ) -> Pass {
        let reader = match cworld_threat::Reader::new(memory, base, game_state) {
            Ok(reader) => reader,
            Err(reason) => return Pass::unavailable(reason),
        };
        let (manager, sources, aggregates, reasons) = if second {
            (
                &mut self.manager.second,
                &mut self.sources.second,
                &mut self.aggregates.second,
                &mut self.reason_bytes.second,
            )
        } else {
            (
                &mut self.manager.rows,
                &mut self.sources.rows,
                &mut self.aggregates.rows,
                &mut self.reason_bytes.first,
            )
        };
        let manager = bounded(reader.manager(manager), manager.len());
        let result = reader.sources(memory, base, sources, reasons);
        let sources = bounded(result.outcome, sources.len());
        let aggregates = bounded(reader.aggregates(memory, aggregates), aggregates.len());
        if !reader.unchanged(memory) {
            return Pass::unavailable("world threat header changed during the walk");
        }
        Pass {
            tables: [manager, sources, aggregates],
            bytes_len: result.bytes_len,
            required_bytes: result.required_bytes,
        }
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            effective(&self.manager),
            effective(&self.sources),
            effective(&self.aggregates),
        ]
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            checked(&self.manager),
            checked(&self.sources),
            checked(&self.aggregates),
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
            table::<tables::ThreatCountryAggregates, _>(
                outcomes[2],
                &self.aggregates.rows,
                |r| rows::ThreatCountryAggregateRow {
                    country_slot: r.country_slot,
                    total_raw: r.total_raw,
                },
            ),
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

fn effective<R>(captured: &CaptureRows<R>) -> Outcome {
    if captured.checked && captured.mismatches != 0 {
        Outcome::Unavailable {
            reason: "world threat table changed during double read",
        }
    } else {
        captured.outcome
    }
}

fn checked<R>(captured: &CaptureRows<R>) -> usize {
    if captured.checked {
        captured.mismatches
    } else {
        0
    }
}

// ===========================================================================
// Hourly session raw-copy split (WorldTension, ThreatSources,
// ThreatCountryAggregates).
//
// The game thread copies raw bytes only: manager pointer, manager header
// (vtable, total, both array headers), per-source pointers with head and
// reason strings, per-aggregate words, plus the manager header again for the
// unchanged witness. No row is decoded on the game thread. The single FIFO
// decode worker replays the existing `cworld_threat::Reader` walk against the
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

/// One hour of raw session payload. Flat pools recycle capacities and the
/// steady state allocates on neither thread; growth happens on the decode
/// side only (see `crate::decode::recycle_session_job`).
#[derive(Debug)]
pub(crate) struct SessionJob {
    pub log: RawLog<3>,
    pub game_state: usize,
}

impl SessionJob {
    pub fn new() -> Self {
        Self {
            // Late-campaign worst case is about 127 sources with reasons
            // plus ~440 aggregates; both pools start above that so steady
            // state never overflows.
            log: RawLog::new(1 << 14, 4 << 20),
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
pub(crate) struct SessionCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    /// Whole-log raw equality gated by live-fault conservatism, in
    /// [world, sources, aggregates] order.
    pub matched: [bool; 3],
    pub copy_ticks: [u64; 3],
    pub raw_bytes: [u64; 3],
}

/// Game-thread copy parameters, bundled to keep the tick entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SessionCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; 3],
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
fn copy_source(
    log: &mut RawLog<3>,
    memory: &impl engine::Memory,
    pointer: usize,
) {
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
/// reasons, aggregates, manager header again for the unchanged witness.
/// Stops at the first typed abort point; later extras are harmless.
fn copy_pass(memory: &impl engine::Memory, base: usize, game_state: usize, log: &mut RawLog<3>) {
    use engine::{cgame_state, cpdx_array::Array, cworld_threat};
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
    let pointer = match usize::try_from(pool_u64(log, ptr_start)).ok().filter(|&p| p != 0) {
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
        let addr = sources.data.checked_add((ordinal as usize).saturating_mul(8));
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
        let addr = aggregates.data.checked_add((slot as usize).saturating_mul(8));
        let _ = sspan(log, memory, addr, 8, M_AGGREGATES);
        if log.overflowed {
            return;
        }
    }
    // Unchanged witness: manager header again, served in record order.
    let _ = sspan(log, memory, Some(pointer), 0x40, M_ALL);
}

impl Captured {
    /// Game-thread raw copy for an hourly refresh. Copies `first` always and
    /// `second` while any table still needs double-read checks, compares the
    /// per-table raw partitions, and records copy-only costs plus the
    /// existing checked/mismatches bookkeeping. Returns whether the payload
    /// is complete (`raw_ok`); on staging overflow the caller falls back to
    /// the typed walk and enqueues nothing.
    pub(crate) fn copy_session_refresh(
        &mut self,
        memory: &impl engine::Memory,
        ctx: SessionCopyContext,
        first: &mut SessionJob,
        second: Option<&mut SessionJob>,
    ) -> SessionCopyReport {
        let started = qpc();
        first.clear();
        first.game_state = ctx.game_state;
        copy_pass(memory, ctx.base, ctx.game_state, &mut first.log);
        let mut ticks = [0u64; 3];
        ticks[0] = elapsed(started);
        let mut matched = [true; 3];
        if let Some(second_job) = second {
            let second_started = qpc();
            second_job.clear();
            second_job.game_state = ctx.game_state;
            copy_pass(memory, ctx.base, ctx.game_state, &mut second_job.log);
            ticks[0] += elapsed(second_started);
            // Three-way partition equality: manager spans (M_ALL) gate all
            // three slots, sources/aggregates private spans only their own.
            let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
            for (slot, matched) in matched.iter_mut().enumerate() {
                let equal =
                    raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
                *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
            }
        }
        // Binary per refresh per table: tearing detected (0/1), not a row
        // count. Spool gates test `==0`/`!=0` only.
        self.manager.checked = ctx.needed[0];
        self.manager.mismatches = usize::from(ctx.needed[0] && !matched[0]);
        self.sources.checked = ctx.needed[1];
        self.sources.mismatches = usize::from(ctx.needed[1] && !matched[1]);
        self.aggregates.checked = ctx.needed[2];
        self.aggregates.mismatches = usize::from(ctx.needed[2] && !matched[2]);
        let raw_ok = !first.log.overflowed;
        SessionCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: first.log.raw_bytes,
        }
    }

    /// Decode-worker interpretation of a raw hourly payload into the pooled
    /// typed rows. Outcomes and arena bytes match the typed walk exactly
    /// because the identical walk runs against the recorded spans;
    /// checked/mismatches set by the raw compare survive. Clears `raw_mode`
    /// so the next capture starts clean.
    pub(crate) fn decode_session_payload(
        &mut self,
        base: usize,
        job: &SessionJob,
        decode_ticks: &mut [u64; 3],
    ) {
        let started = qpc();
        let recorded = crate::raw::Recorded::build(&job.log);
        let walked = self.pass(&recorded, base, job.game_state, false);
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
    use crate::sampling::CONSISTENCY_TARGET;
    use engine::{cgame_state, memory::SliceMemory, msvc_string};
    use replay::rows::TableSchema;
    use std::cell::Cell;

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
        put(bytes, at, &(cthreat_source::VTABLE_RVA as u64).to_le_bytes());
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
        array(&mut bytes, MANAGER + cworld_threat::SOURCES, SOURCE_ARRAY, 2);
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
        let captured = Captured::with_capacities(&capacities, &[0; N]);
        (capacities, captured)
    }

    fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
        let TableView::Complete(rows) = view else {
            panic!("expected complete rows, got {view:?}")
        };
        rows.rows::<S>().unwrap()
    }

    #[test]
    fn manager_sources_and_aggregates_keep_raw_operands_and_literal_reasons() {
        let (_, mut captured) = small();
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let (matched, _) = captured.capture(&memory, 0, BASE, &[true; N]);
        assert_eq!(matched, [true; N]);
        assert_eq!(captured.checked_mismatches(), [0; N]);
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
        let mut captured_small = Captured::with_capacities(&capacities, &[0; N]);
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let (matched, _) = captured_small.capture(&memory, 0, BASE, &[true; N]);
        // A truncated table never earns a consistency check, even when both
        // passes truncate identically.
        assert_eq!(matched, [true, false, true]);
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
        captured_small.grow(&mut capacities, &[0; N]);
        assert!(capacities.reason_bytes >= 5);

        let mut broken = fixture();
        put(&mut broken, MANAGER, &0_u64.to_le_bytes());
        let (matched, _) = captured.capture(
            &SliceMemory {
                base: BASE,
                bytes: &broken,
            },
            0,
            BASE,
            &[true; N],
        );
        assert_eq!(matched, [false; N]);
        for outcome in captured.outcomes() {
            assert_eq!(
                outcome,
                Outcome::Unavailable {
                    reason: "world threat vtable mismatch"
                }
            );
        }
    }

    struct ChangingReason<'a> {
        bytes: &'a [u8],
        copies: Cell<usize>,
    }
    impl Memory for ChangingReason<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if !(SliceMemory {
                base: BASE,
                bytes: self.bytes,
            })
            .copy(address, output)
            {
                return false;
            }
            if address == FIRST + cthreat_source::REASON && output.len() == 5 {
                let copies = self.copies.get();
                self.copies.set(copies + 1);
                if copies != 0 {
                    output[0] ^= 1;
                }
            }
            true
        }
    }

    #[test]
    fn a_changed_reason_arena_fails_the_source_window_and_finished_windows_release() {
        let (mut capacities, mut captured) = small();
        let bytes = fixture();
        let memory = ChangingReason {
            bytes: &bytes,
            copies: Cell::new(0),
        };
        let (matched, _) = captured.capture(&memory, 0, BASE, &[true; N]);
        assert_eq!(matched, [true, false, true]);
        assert_eq!(captured.checked_mismatches(), [0, 1, 0]);
        assert!(matches!(
            captured.tables()[1],
            TableView::Unavailable { .. }
        ));
        assert!(!captured.sources.second.is_empty());
        captured.grow(&mut capacities, &[CONSISTENCY_TARGET; N]);
        // A finished window releases every comparison buffer off the game
        // thread; the first buffers keep their own growth headroom.
        assert!(captured.manager.second.is_empty());
        assert!(captured.sources.second.is_empty());
        assert!(captured.aggregates.second.is_empty());
        assert!(captured.reason_bytes.second.is_empty());
    }

    fn split_capacities() -> Capacities {
        Capacities {
            rows: [2, 8, 8],
            reason_bytes: 64,
        }
    }

    fn direct(memory: &SliceMemory<'_>, capacities: &Capacities) -> Captured {
        let mut captured = Captured::with_capacities(capacities, &[0; N]);
        let (matched, _) = captured.capture(memory, 0, BASE, &[true; N]);
        assert_eq!(matched, [true; N]);
        captured
    }

    fn split(memory: &SliceMemory<'_>, capacities: &Capacities) -> (Captured, SessionCopyReport) {
        let mut captured = Captured::with_capacities(capacities, &[0; N]);
        let ctx = SessionCopyContext {
            base: 0,
            game_state: BASE,
            needed: [true; N],
        };
        let mut first = SessionJob::new();
        let mut second = SessionJob::new();
        let report = captured.copy_session_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        captured.raw_mode = report.raw_ok;
        let mut ticks = [0u64; 3];
        captured.decode_session_payload(0, &first, &mut ticks);
        (captured, report)
    }

    fn assert_matches_direct(split: &Captured, direct: &Captured) {
        assert_eq!(split.manager.outcome, direct.manager.outcome, "manager outcome");
        assert_eq!(split.sources.outcome, direct.sources.outcome, "sources outcome");
        assert_eq!(
            split.aggregates.outcome, direct.aggregates.outcome,
            "aggregates outcome"
        );
        let len = split.manager.outcome.len();
        assert_eq!(&split.manager.rows[..len], &direct.manager.rows[..len], "manager rows");
        let len = split.sources.outcome.len();
        assert_eq!(&split.sources.rows[..len], &direct.sources.rows[..len], "source rows");
        assert_eq!(
            &split.reason_bytes.first[..split.reason_bytes.len],
            &direct.reason_bytes.first[..direct.reason_bytes.len],
            "reason arena"
        );
        assert_eq!(split.reason_bytes.required, direct.reason_bytes.required, "required");
        let len = split.aggregates.outcome.len();
        assert_eq!(
            &split.aggregates.rows[..len],
            &direct.aggregates.rows[..len],
            "aggregate rows"
        );
    }

    #[test]
    fn session_split_copy_matches_direct_walk() {
        let capacities = split_capacities();
        let bytes = fixture();
        let memory = SliceMemory { base: BASE, bytes: &bytes };
        let direct = direct(&memory, &capacities);
        let (split, report) = split(&memory, &capacities);
        assert_eq!(report.matched, [true; 3]);
        assert_matches_direct(&split, &direct);
        assert_eq!(split.manager.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.sources.outcome, Outcome::Rows { len: 2 });
    }

    #[test]
    fn session_split_copy_matches_direct_walk_under_truncation() {
        let capacities = Capacities { rows: [1, 1, 1], reason_bytes: 2 };
        let bytes = fixture();
        let memory = SliceMemory { base: BASE, bytes: &bytes };
        let mut direct = Captured::with_capacities(&capacities, &[0; N]);
        let (matched, _) = direct.capture(&memory, 0, BASE, &[true; N]);
        assert_eq!(matched, [true, false, false]);
        let (split, report) = {
            let mut captured = Captured::with_capacities(&capacities, &[0; N]);
            let ctx = SessionCopyContext { base: 0, game_state: BASE, needed: [true; N] };
            let mut first = SessionJob::new();
            let mut second = SessionJob::new();
            let report = captured.copy_session_refresh(&memory, ctx, &mut first, Some(&mut second));
            assert!(report.raw_ok);
            captured.raw_mode = true;
            let mut ticks = [0u64; 3];
            captured.decode_session_payload(0, &first, &mut ticks);
            (captured, report)
        };
        assert_eq!(report.matched, [true; 3]);
        assert_matches_direct(&split, &direct);
        assert!(matches!(split.sources.outcome, Outcome::Truncated { .. }));
    }

    #[test]
    fn session_double_copy_compare_blocks_checks_on_tearing() {
        let capacities = split_capacities();
        let bytes = fixture();
        let memory = SliceMemory { base: BASE, bytes: &bytes };
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = SessionCopyContext { base: 0, game_state: BASE, needed: [true; N] };
        let mut first = SessionJob::new();
        let mut second = SessionJob::new();
        let report = captured.copy_session_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 3]);
        // A torn source pointer between the passes changes the log.
        let mut torn = fixture();
        put(&mut torn, SOURCE_ARRAY, &0_u64.to_le_bytes());
        let mut after = SessionJob::new();
        copy_pass(&memory, 0, BASE, &mut after.log);
        let mut after_torn = SessionJob::new();
        copy_pass(
            &SliceMemory { base: BASE, bytes: &torn },
            0,
            BASE,
            &mut after_torn.log,
        );
        assert!(!after.log.logs_equal(&after_torn.log));
    }

    #[test]
    fn session_partition_equal_isolates_sources_private_bytes() {
        // Tear exactly the sources table's private bytes (M_SOURCES 0x02):
        // only slot 1 flips; manager (M_ALL) and aggregates (0x04) stay
        // true. The manager header gates all three, but sources-private
        // payloads gate only sources.
        let capacities = split_capacities();
        let bytes = fixture();
        let memory = SliceMemory { base: BASE, bytes: &bytes };
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = SessionCopyContext { base: 0, game_state: BASE, needed: [true; N] };
        let mut first = SessionJob::new();
        let mut second = SessionJob::new();
        let report = captured.copy_session_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 3]);
        let sources_idx = second
            .log
            .spans
            .iter()
            .position(|span| span.ok && span.mask == 0x02)
            .expect("sources-private span");
        let start = second.log.spans[sources_idx].start as usize;
        second.log.bytes[start] ^= 0xff;
        assert!(first.log.partition_equal(&second.log, 0));
        assert!(!first.log.partition_equal(&second.log, 1));
        assert!(first.log.partition_equal(&second.log, 2));
    }

    #[test]
    fn session_staging_overflow_falls_back_to_typed_walk() {
        let capacities = split_capacities();
        let bytes = fixture();
        let memory = SliceMemory { base: BASE, bytes: &bytes };
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = SessionCopyContext { base: 0, game_state: BASE, needed: [true; N] };
        let mut tiny = SessionJob { log: crate::raw::RawLog::new(8, 8), game_state: 0 };
        let report = captured.copy_session_refresh(&memory, ctx, &mut tiny, None);
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
        let memory = SliceMemory { base: BASE, bytes: &bytes };
        for (rows, reason) in [
            ([0, 0, 0], 0),
            ([1, 1, 1], 2),
            ([1, 2, 2], 8),
            ([2, 3, 3], 8),
            ([8, 8, 8], 64),
        ] {
            let capacities = Capacities { rows, reason_bytes: reason };
            let mut direct = Captured::with_capacities(&capacities, &[0; N]);
            let (matched, _) = direct.capture(&memory, 0, BASE, &[true; N]);
            let _ = matched;
            let mut split = Captured::with_capacities(&capacities, &[0; N]);
            let ctx = SessionCopyContext { base: 0, game_state: BASE, needed: [true; N] };
            let mut first = SessionJob::new();
            let mut second = SessionJob::new();
            let report = split.copy_session_refresh(&memory, ctx, &mut first, Some(&mut second));
            assert!(report.raw_ok, "rows={rows:?}");
            assert_eq!(report.matched, [true; 3], "rows={rows:?}");
            split.raw_mode = true;
            let mut ticks = [0u64; 3];
            split.decode_session_payload(0, &first, &mut ticks);
            assert_matches_direct(&split, &direct);
        }
        // Exact-fit arm pins Rows with all rows present.
        let capacities = Capacities { rows: [1, 2, 2], reason_bytes: 8 };
        let mut direct = Captured::with_capacities(&capacities, &[0; N]);
        let _ = direct.capture(&memory, 0, BASE, &[true; N]);
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
        let memory = Recorder { inner: SliceMemory { base: BASE, bytes: &bytes }, sizes: std::cell::RefCell::new(BTreeSet::new()) };
        let mut job = SessionJob::new();
        copy_pass(&memory, 0, BASE, &mut job.log);
        assert!(job.log.raw_bytes.iter().any(|&b| b > 0));
        let sizes = memory.sizes.borrow().clone();
        for required in [8, 32, 52, 64] {
            assert!(sizes.contains(&required), "missing size {required} in {sizes:?}");
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
        let memory = SliceMemory { base: BASE, bytes: &bytes };
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
        expected.copy_from_slice(
            &bytes[header.addr - BASE + 3..header.addr - BASE + 11],
        );
        assert_eq!(word, expected);
    }
}
