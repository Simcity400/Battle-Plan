//! The global dictionary is acquired once per callback with passive witnesses.
//! Only spool-thread growth and conversion allocate.

use crate::{
    raw::RawLog,
    sampling::qpc,
    spooling::table,
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, cpdx_lexer, cpdx_lexer::TOKEN_NAME_STRIDE, walk::Outcome};
use replay::{
    frame::{TABLE_COUNT, TableView},
    rows::{self, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::LexicalTokens];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[("global lexical dictionary", 0, &[0])];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: usize,
    pub text: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 32768,
            text: 1_048_576,
        }
    }
}

pub struct Captured {
    rows: CaptureRows<cpdx_lexer::Row>,
    text: Arena<u8>,
    /// Raw-copy split gate for the Tier-4 slot-3 lexical table. Set by the
    /// game thread on refresh hours with a valid raw payload; `capture` then
    /// skips the typed walk and the decode worker fills these rows before
    /// the ready handoff. Preserved across `reset` (set per refresh, cleared
    /// by the worker after decode).
    pub(crate) raw_mode: bool,
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows, successful[0]),
            text: Arena::new(capacity.text, successful[0]),
            raw_mode: false,
        }
    }

    pub fn reset(&mut self) {
        self.rows.reset();
        self.text.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.text.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        self.rows.grow(&mut capacity.rows, successful[0]);
        self.text.grow(&mut capacity.text, successful[0]);
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        _game_state: usize,
        needed: &[bool],
        hour: i32,
        force_full: bool,
    ) -> ([bool; N], [u64; N]) {
        self.reset();
        // Tier-4 fallback: witness cannot prove staleness-free static caching.
        // Raw-copy split: on refresh hours the game thread stages raw bytes
        // via `copy_lexical_refresh` and the decode worker fills the typed
        // rows before the ready handoff, so the typed walk is skipped here.
        // Matched flags and costs come from the raw compare, applied by the
        // caller after this returns.
        if self.raw_mode {
            // Refresh payload staged: leave the reset outcomes for the decode
            // worker, which installs identical rows, outcomes and arenas.
            return ([false], [0]);
        }
        if !replay::cadence::should_walk(TableId::LexicalTokens, hour, force_full) {
            self.rows.outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            return ([false], [0]);
        }
        let start = qpc();
        let first = cpdx_lexer::walk(memory, base, &mut self.rows.rows, &mut self.text.first);
        self.rows.outcome = first.outcome;
        self.text.len = first.text_len;
        self.text.required = first.text_required;
        let mut matched = false;
        if needed[0] {
            self.rows.checked = true;
            let second =
                cpdx_lexer::walk(memory, base, &mut self.rows.second, &mut self.text.second);
            matched = self.rows.compare(second.outcome);
            if !self.text.compare(second.text_len, second.text_required) {
                self.rows.mismatches = self.rows.mismatches.saturating_add(1);
                matched = false;
            }
        }
        ([matched], [qpc().saturating_sub(start).max(0) as u64])
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "lexical dictionary changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }

    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::LexicalTokens, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |row| rows::LexicalTokenRow {
                token: row.token,
                literal: self.text.first[row.text_start..row.text_start + row.text_len].to_vec(),
            },
        )]
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

// ===========================================================================
// Tier-4 slot-3 raw-copy split (LexicalTokens).
//
// The game thread reads the writer witness as the copy gate, bulk-copies the
// contiguous token-header table in 64 KiB chunks, copies heap payloads and
// terminators by header length, and closes with the witness-after pair. The
// single FIFO decode worker replays the existing `cpdx_lexer::walk` against
// the recorded spans (`crate::raw::Recorded` with bulk containment), so rows,
// skips, truncation and failure reasons are identical by construction (see
// `crate::raw` for the mirror rules).
//
// An invalid witness stops the copy with the staged witness spans: the
// replay derives the identical reason without table spans. A failed bulk
// chunk falls back to per-token header takes for that chunk's range, so a
// mid-table fault fails at the identical token as the typed walk.
// ===========================================================================

/// Bulk chunk size for the contiguous header table; a documented raw-span
/// bound (the header stride itself stays the engine's constant).
const TABLE_CHUNK: usize = 65536;

/// One refresh of raw lexical payload. Flat pools recycle capacities and the
/// steady state allocates on neither thread; growth happens on the decode
/// side only (see `crate::decode::recycle_lexical_job`).
#[derive(Debug)]
pub(crate) struct LexicalJob {
    pub log: RawLog<1>,
}

impl LexicalJob {
    pub fn new() -> Self {
        Self {
            // Late-campaign worst case is about 71k headers (2.3 MB) plus of
            // heap payloads; both pools start above that so steady state
            // never overflows.
            log: RawLog::new(1 << 17, 12 << 20),
        }
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    /// Double every pool off the game thread until copies fit. Converges like
    /// the typed capacity growth; overflowed refreshes fall back to the typed
    /// walk in the meantime, so sizing is a pure performance knob.
    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct LexicalCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    pub matched: [bool; 1],
    pub copy_ticks: [u64; 1],
    pub raw_bytes: [u64; 1],
}

/// Game-thread copy parameters, bundled to keep the refresh entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LexicalCopyContext {
    pub base: usize,
    pub needed: [bool; TABLE_COUNT],
}

/// Parsed writer witness: the table geometry the bulk copy sizes itself
/// from. Parsing is local; only the takes touch memory.
struct Witness {
    maximum: u32,
    table: usize,
}

/// Mirror of `Witness::read`: writer flag, 64-byte header, writer flag, with
/// identical local parses. Returns `None` exactly when the typed witness
/// fails, having recorded the same spans the replay re-derives the reason
/// from.
fn copy_witness(
    log: &mut RawLog<1>,
    memory: &impl Memory,
    base: usize,
) -> Option<Witness> {
    use engine::cpdx_lexer::*;
    let writer = base.checked_add(WRITER_FLAG_RVA)?;
    let mut flag = [0u8; 4];
    let start = log.take(memory, writer, 4, 0x01)?;
    flag.copy_from_slice(&log.bytes[start..start + 4]);
    if u32::from_le_bytes(flag) != 0 {
        return None;
    }
    let header_addr = base.checked_add(INITIALIZED_RVA)?;
    let header_start = log.take(memory, header_addr, 64, 0x01)?;
    let mut header = [0u8; 64];
    header.copy_from_slice(&log.bytes[header_start..header_start + 64]);
    let writer_again = log.take(memory, writer, 4, 0x01)?;
    if u32::from_le_bytes(log.bytes[writer_again..writer_again + 4].try_into().unwrap()) != 0 {
        return None;
    }
    if header[0] != 1 {
        return None;
    }
    let signed = |offset: usize| -> Option<u32> {
        u32::try_from(i32::from_le_bytes(header[offset..offset + 4].try_into().unwrap())).ok()
    };
    let pointer = |offset: usize| -> usize {
        u64::from_le_bytes(header[offset..offset + 8].try_into().unwrap()) as usize
    };
    if signed(WRITER_FLAG_RVA - INITIALIZED_RVA)? != 0 {
        return None;
    }
    let maximum = signed(MAX_TOKEN_ID_RVA - INITIALIZED_RVA)?;
    let static_maximum = signed(STATIC_MAX_TOKEN_ID_RVA - INITIALIZED_RVA)?;
    let dynamic_pointer = u64::from_le_bytes(
        header[DYNAMIC_TOKENS_RVA - INITIALIZED_RVA..DYNAMIC_TOKENS_RVA - INITIALIZED_RVA + 8]
            .try_into()
            .unwrap(),
    );
    let dynamic_capacity = signed(DYNAMIC_CAPACITY_RVA - INITIALIZED_RVA)?;
    let dynamic_count = signed(DYNAMIC_COUNT_RVA - INITIALIZED_RVA)?;
    let table = pointer(TOKEN_NAMES_RVA - INITIALIZED_RVA);
    let allocation = signed(TOKEN_ALLOCATION_RVA - INITIALIZED_RVA)?;
    let length = signed(TOKEN_CAPACITY_RVA - INITIALIZED_RVA)?;
    if static_maximum.checked_add(dynamic_count)? != maximum
        || dynamic_count > dynamic_capacity
        || (dynamic_count != 0 && dynamic_pointer == 0)
        || table == 0
        || maximum >= length
        || length > allocation
    {
        return None;
    }
    table
        .checked_add((length as usize).checked_mul(TOKEN_NAME_STRIDE)?)
        .map(|_| Witness { maximum, table })
}

/// Mirror of one token's string reads: header (served from the bulk chunk
/// at replay), terminator and heap payload takes. Returns `false` exactly
/// when the typed `text` fails for this token.
fn copy_token_text(
    log: &mut RawLog<1>,
    memory: &impl Memory,
    header: &[u8; 32],
    address: usize,
) -> bool {
    use engine::msvc_string;
    let len = u64::from_le_bytes(header[16..24].try_into().unwrap());
    let capacity = u64::from_le_bytes(header[24..32].try_into().unwrap());
    if capacity < msvc_string::INLINE_CAPACITY || len > capacity {
        return false;
    }
    let Ok(len) = usize::try_from(len) else {
        return false;
    };
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        address
    } else {
        match usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&source| source != 0)
        {
            Some(source) => source,
            None => return false,
        }
    };
    let Some(end) = source.checked_add(len) else {
        return false;
    };
    if log.take(memory, end, 1, 0x01).is_none() {
        return false;
    }
    if len != 0 && capacity != msvc_string::INLINE_CAPACITY && log.take(memory, source, len, 0x01).is_none() {
        return false;
    }
    // The replay validates the terminator byte itself from the recorded
    // spans; identical bytes give the identical verdict.
    true
}

/// One structural pass: witness gate, bulk header chunks with per-token heap
/// takes, witness-after pair. Stops at the first typed abort point.
fn copy_pass(memory: &impl Memory, base: usize, log: &mut RawLog<1>) {
    let Some(witness) = copy_witness(log, memory, base) else {
        return;
    };
    log.has_bulk = true;
    let total = (witness.maximum as usize + 1)
        .checked_mul(TOKEN_NAME_STRIDE)
        .unwrap_or(0);
    let mut offset = 0usize;
    while offset < total {
        let len = (total - offset).min(TABLE_CHUNK);
        let address = match witness.table.checked_add(offset) {
            Some(address) => address,
            None => return,
        };
        match log.take_bulk(memory, address, len, 0x01) {
            Some(start) => {
                // Heap payloads for this chunk, parsed locally from the
                // just-copied headers in token order. Each header is lifted
                // to the stack first so the payload takes can borrow the log.
                let tokens = len / TOKEN_NAME_STRIDE;
                for index in 0..tokens {
                    let header_addr = match address.checked_add(index * TOKEN_NAME_STRIDE) {
                        Some(addr) => addr,
                        None => return,
                    };
                    let mut header = [0u8; 32];
                    header.copy_from_slice(
                        &log.bytes[start + index * TOKEN_NAME_STRIDE
                            ..start + (index + 1) * TOKEN_NAME_STRIDE],
                    );
                    if !copy_token_text(log, memory, &header, header_addr) {
                        return;
                    }
                    if log.overflowed {
                        return;
                    }
                }
            }
            None if log.overflowed => return,
            // Failed chunk: per-token header takes for this range, stopping
            // at the identical token the typed walk fails on.
            None => {
                let tokens = len / TOKEN_NAME_STRIDE;
                for index in 0..tokens {
                    let header_addr = match witness
                        .table
                        .checked_add(offset + index * TOKEN_NAME_STRIDE)
                    {
                        Some(addr) => addr,
                        None => return,
                    };
                    let Some(header_start) = log.take(memory, header_addr, 32, 0x01) else {
                        return;
                    };
                    let mut header = [0u8; 32];
                    header.copy_from_slice(&log.bytes[header_start..header_start + 32]);
                    if !copy_token_text(log, memory, &header, header_addr) {
                        return;
                    }
                    if log.overflowed {
                        return;
                    }
                }
            }
        }
        offset += len;
    }
    // Witness-after pair, read after the rows exactly like the typed walk.
    copy_witness(log, memory, base);
}

impl Captured {
    /// Game-thread raw copy for a slot-3 refresh hour. Copies `first` always
    /// and `second` while the table still needs double-read checks, compares
    /// the raw logs, and records copy-only costs plus the existing
    /// checked/mismatches bookkeeping. Returns whether the payload is
    /// complete (`raw_ok`); on staging overflow the caller falls back to the
    /// typed walk and enqueues nothing.
    pub(crate) fn copy_lexical_refresh(
        &mut self,
        memory: &impl Memory,
        ctx: LexicalCopyContext,
        first: &mut LexicalJob,
        second: Option<&mut LexicalJob>,
    ) -> LexicalCopyReport {
        let needed = ctx.needed[TableId::LexicalTokens.index()];
        let started = qpc();
        copy_pass(memory, ctx.base, &mut first.log);
        let mut ticks = [0u64; 1];
        ticks[0] = elapsed(started);
        let mut matched = [true; 1];
        if let Some(second_job) = second {
            let second_started = qpc();
            copy_pass(memory, ctx.base, &mut second_job.log);
            ticks[0] += elapsed(second_started);
            let equal =
                first.log.logs_equal(&second_job.log) && !first.log.overflowed && !second_job.log.overflowed;
            matched[0] = equal && !first.log.fault[0] && !second_job.log.fault[0];
        }
        self.rows.checked = needed;
        self.rows.mismatches = usize::from(needed && !matched[0]);
        let raw_ok = !first.log.overflowed;
        LexicalCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: first.log.raw_bytes,
        }
    }

    /// Decode-worker interpretation of a raw refresh payload into the pooled
    /// typed rows. Outcomes and arena bytes match the typed walk exactly
    /// because the identical walk runs against the recorded spans;
    /// checked/mismatches set by the raw compare survive. Clears `raw_mode`
    /// so the next capture starts clean.
    pub(crate) fn decode_lexical_payload(
        &mut self,
        base: usize,
        job: &LexicalJob,
        decode_ticks: &mut [u64; 1],
    ) {
        let started = qpc();
        let recorded = crate::raw::Recorded::build(&job.log);
        let walked = cpdx_lexer::walk(&recorded, base, &mut self.rows.rows, &mut self.text.first);
        self.rows.outcome = walked.outcome;
        self.text.len = walked.text_len;
        self.text.required = walked.text_required;
        decode_ticks[0] += elapsed(started);
        self.raw_mode = false;
    }
}

/// Shared synthetic address space for the lexical copy/decode split
/// harness: a three-token dictionary with inline, heap and empty names.
#[cfg(test)]
pub(crate) mod lexical_test_kit {
    use super::{Capacities, Captured, LexicalCopyContext, LexicalCopyReport, LexicalJob, N};
    use engine::{cpdx_lexer, msvc_string};
    use engine::cpdx_lexer::{
        DYNAMIC_CAPACITY_RVA, DYNAMIC_COUNT_RVA, DYNAMIC_TOKENS_RVA, INITIALIZED_RVA,
        MAX_TOKEN_ID_RVA, STATIC_MAX_TOKEN_ID_RVA, TOKEN_ALLOCATION_RVA, TOKEN_CAPACITY_RVA,
        TOKEN_NAMES_RVA, WRITER_FLAG_RVA,
    };
    use replay::frame::TABLE_COUNT;
    use std::cell::Cell;

    pub(crate) const BASE: usize = 0x140000000;
    const TABLE_BASE: usize = 0x20000000;
    const HEAP_BASE: usize = 0x30000000;

    pub(crate) struct KitMemory {
        pub writer_before: u32,
        pub writer_after: u32,
        pub header_before: [u8; 64],
        pub header_after: [u8; 64],
        pub table: Vec<u8>,
        pub heap: Vec<u8>,
        writer_reads: Cell<usize>,
        header_reads: Cell<usize>,
    }

    impl KitMemory {
        fn header(maximum: u32, table: usize, length: u32) -> [u8; 64] {
            let mut header = [0u8; 64];
            header[0] = 1;
            header[WRITER_FLAG_RVA - INITIALIZED_RVA
                ..WRITER_FLAG_RVA - INITIALIZED_RVA + 4]
                .copy_from_slice(&0_u32.to_le_bytes());
            header[MAX_TOKEN_ID_RVA - INITIALIZED_RVA..MAX_TOKEN_ID_RVA - INITIALIZED_RVA + 4]
                .copy_from_slice(&maximum.to_le_bytes());
            header[STATIC_MAX_TOKEN_ID_RVA - INITIALIZED_RVA
                ..STATIC_MAX_TOKEN_ID_RVA - INITIALIZED_RVA + 4]
                .copy_from_slice(&maximum.to_le_bytes());
            header[TOKEN_NAMES_RVA - INITIALIZED_RVA..TOKEN_NAMES_RVA - INITIALIZED_RVA + 8]
                .copy_from_slice(&(table as u64).to_le_bytes());
            header[TOKEN_ALLOCATION_RVA - INITIALIZED_RVA..TOKEN_ALLOCATION_RVA - INITIALIZED_RVA + 4]
                .copy_from_slice(&length.to_le_bytes());
            header[TOKEN_CAPACITY_RVA - INITIALIZED_RVA..TOKEN_CAPACITY_RVA - INITIALIZED_RVA + 4]
                .copy_from_slice(&length.to_le_bytes());
            header[DYNAMIC_CAPACITY_RVA - INITIALIZED_RVA
                ..DYNAMIC_CAPACITY_RVA - INITIALIZED_RVA + 4]
                .copy_from_slice(&0_u32.to_le_bytes());
            header[DYNAMIC_COUNT_RVA - INITIALIZED_RVA..DYNAMIC_COUNT_RVA - INITIALIZED_RVA + 4]
                .copy_from_slice(&0_u32.to_le_bytes());
            header[DYNAMIC_TOKENS_RVA - INITIALIZED_RVA..DYNAMIC_TOKENS_RVA - INITIALIZED_RVA + 8]
                .copy_from_slice(&0_u64.to_le_bytes());
            header
        }

        fn inline(table: &mut [u8], token: usize, value: &[u8]) {
            assert!(value.len() <= msvc_string::INLINE_CAPACITY as usize);
            let at = token * 32;
            table[at..at + value.len()].copy_from_slice(value);
            table[at + 16..at + 24].copy_from_slice(&(value.len() as u64).to_le_bytes());
            table[at + 24..at + 32].copy_from_slice(&15_u64.to_le_bytes());
        }

        fn heap(table: &mut [u8], heap: &mut Vec<u8>, token: usize, value: &[u8]) {
            assert!(value.len() > msvc_string::INLINE_CAPACITY as usize);
            let at = token * 32;
            let payload = HEAP_BASE + heap.len();
            table[at..at + 8].copy_from_slice(&(payload as u64).to_le_bytes());
            table[at + 16..at + 24].copy_from_slice(&(value.len() as u64).to_le_bytes());
            table[at + 24..at + 32].copy_from_slice(&(value.len() as u64).to_le_bytes());
            heap.extend_from_slice(value);
            heap.push(0);
            let _ = payload;
        }

        pub(crate) fn fixture() -> Self {
            let mut table = vec![0u8; 3 * 32];
            let mut heap = Vec::new();
            Self::inline(&mut table, 0, b"ab");
            Self::heap(&mut table, &mut heap, 1, b"heap-token-lexical!");
            Self::inline(&mut table, 2, b"");
            let header = Self::header(2, TABLE_BASE, 3);
            Self {
                writer_before: 0,
                writer_after: 0,
                header_before: header,
                header_after: header,
                table,
                heap,
                writer_reads: Cell::new(0),
                header_reads: Cell::new(0),
            }
        }

        fn serve(&self, bytes: &[u8], destination: &mut [u8]) -> bool {
            if destination.len() > bytes.len() {
                return false;
            }
            destination.copy_from_slice(&bytes[..destination.len()]);
            true
        }
    }

    impl engine::Memory for KitMemory {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if destination.is_empty() {
                return true;
            }
            if address == BASE + WRITER_FLAG_RVA && destination.len() == 4 {
                let reads = self.writer_reads.get();
                self.writer_reads.set(reads + 1);
                let value = if reads < 2 {
                    self.writer_before
                } else {
                    self.writer_after
                };
                destination.copy_from_slice(&value.to_le_bytes());
                return true;
            }
            if address == BASE + INITIALIZED_RVA && destination.len() == 64 {
                let reads = self.header_reads.get();
                self.header_reads.set(reads + 1);
                let header = if reads == 0 {
                    &self.header_before
                } else {
                    &self.header_after
                };
                destination.copy_from_slice(header);
                return true;
            }
            if address >= TABLE_BASE
                && let Some(offset) = address.checked_sub(TABLE_BASE)
                && let Some(bytes) = self.table.get(offset..)
            {
                return self.serve(bytes, destination);
            }
            if address >= HEAP_BASE
                && let Some(offset) = address.checked_sub(HEAP_BASE)
                && let Some(bytes) = self.heap.get(offset..)
            {
                return self.serve(bytes, destination);
            }
            false
        }
    }

    /// Reference typed walk with identical pooled buffers and arenas.
    pub(crate) fn direct(memory: &KitMemory, capacities: &Capacities) -> Captured {
        let mut captured = Captured::with_capacities(capacities, &[0; N]);
        let walked = cpdx_lexer::walk(memory, BASE, &mut captured.rows.rows, &mut captured.text.first);
        captured.rows.outcome = walked.outcome;
        captured.text.len = walked.text_len;
        captured.text.required = walked.text_required;
        captured
    }

    /// Raw-copy split: two raw passes compared, then the first payload
    /// decoded into fresh pooled buffers of the same capacities.
    pub(crate) fn split(
        memory: &KitMemory,
        capacities: &Capacities,
    ) -> (Captured, LexicalCopyReport, LexicalJob) {
        let mut captured = Captured::with_capacities(capacities, &[0; N]);
        let ctx = LexicalCopyContext {
            base: BASE,
            needed: [true; TABLE_COUNT],
        };
        let mut first = LexicalJob::new();
        let mut second = LexicalJob::new();
        let report = captured.copy_lexical_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut ticks = [0u64; 1];
        captured.decode_lexical_payload(BASE, &first, &mut ticks);
        (captured, report, first)
    }

    pub(crate) fn assert_matches_direct(split: &Captured, direct: &Captured) {
        assert_eq!(split.rows.outcome, direct.rows.outcome, "outcome");
        let len = split.rows.outcome.len();
        assert_eq!(direct.rows.outcome.len(), len, "length");
        assert_eq!(&split.rows.rows[..len], &direct.rows.rows[..len], "rows");
        assert_eq!(
            &split.text.first[..split.text.len],
            &direct.text.first[..direct.text.len],
            "arena"
        );
        assert_eq!(split.text.required, direct.text.required, "required");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
    use engine::{cpdx_lexer::*, memory::SliceMemory};
    use std::cell::Cell;
    struct MemoryFixture {
        header: [u8; 64],
        names: [u8; 64],
        copies: Cell<usize>,
        change_on_second: bool,
    }

    impl Memory for MemoryFixture {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == WRITER_FLAG_RVA {
                destination.fill(0);
                return true;
            }
            if address == INITIALIZED_RVA {
                destination.copy_from_slice(&self.header);
                return true;
            }
            let count = self.copies.get();
            if address == 0x1000 && destination.len() == 32 {
                self.copies.set(count + 1);
            }
            let success = SliceMemory {
                base: 0x1000,
                bytes: &self.names,
            }
            .copy(address, destination);
            if success
                && address == 0x1000
                && destination.len() == 1
                && self.change_on_second
                && self.copies.get() > 1
            {
                destination[0] = b'z';
            }
            success
        }
    }

    fn fixture(change_on_second: bool) -> MemoryFixture {
        let mut header = [0; 64];
        header[0] = 1;
        header[48..56].copy_from_slice(&0x1000_u64.to_le_bytes());
        header[56..60].copy_from_slice(&1_i32.to_le_bytes());
        header[60..64].copy_from_slice(&1_i32.to_le_bytes());
        let mut names = [0; 64];
        names[0] = 255;
        names[16..24].copy_from_slice(&1_u64.to_le_bytes());
        names[24..32].copy_from_slice(&15_u64.to_le_bytes());
        MemoryFixture {
            header,
            names,
            copies: Cell::new(0),
            change_on_second,
        }
    }

    #[test]
    fn lexical_pack_preserves_exact_bytes_and_compares_the_arena() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 2, text: 32 }, &[0]);
        let (matched, _) = captured.capture(&fixture(false), 0, 0, &[true], 3, true);
        assert_eq!(matched, [true]);
        let views = captured.tables();
        let TableView::Complete(rows) = &views[0] else {
            panic!("complete dictionary expected")
        };
        let row = &rows.rows::<tables::LexicalTokens>().unwrap()[0];
        assert_eq!(row.literal, [255]);
        assert_eq!(row.token, 0);
        let (matched, _) = captured.capture(&fixture(true), 0, 0, &[true], 3, true);
        assert_eq!(matched, [false]);
        assert_eq!(captured.checked_mismatches(), [1]);
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn lexical_pool_grows_after_stable_truncation_and_releases_comparison_storage() {
        let mut capacity = Capacities { rows: 1, text: 0 };
        let mut captured = Captured::with_capacities(&capacity, &[0]);
        captured.capture(&fixture(false), 0, 0, &[false], 3, true);
        assert_eq!(
            captured.outcomes()[0],
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        captured.grow(&mut capacity, &[CONSISTENCY_TARGET]);
        assert!(capacity.text >= 1);
        assert!(captured.rows.second.is_empty() && captured.text.second.is_empty());
        captured.capture(&fixture(false), 0, 0, &[false], 3, true);
        assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 1 });
    }

    #[test]
    fn lexical_off_hours_skips_walk_and_marks_carry_forward() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 2, text: 32 }, &[0]);
        let (matched, ticks) = captured.capture(&fixture(false), 0, 0, &[true], 0, false);
        assert_eq!(matched, [false]);
        assert_eq!(ticks, [0]);
        assert_eq!(
            captured.outcomes()[0],
            Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            }
        );
    }

    fn split_capacities() -> Capacities {
        Capacities {
            rows: 256,
            text: 8192,
        }
    }

    #[test]
    fn split_copy_matches_direct_walk() {
        use super::lexical_test_kit as kit;
        let memory = kit::KitMemory::fixture();
        let capacities = split_capacities();
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true]);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.rows.outcome, Outcome::Rows { len: 3 });
        assert_eq!(split.rows.rows[0].token, 0);
        assert_eq!(&split.text.first[0..2], b"ab");
        assert_eq!(split.rows.rows[1].text_len, 19);
        assert_eq!(split.rows.rows[2].text_len, 0);
    }

    #[test]
    fn split_copy_matches_direct_walk_under_truncation() {
        use super::lexical_test_kit as kit;
        let memory = kit::KitMemory::fixture();
        let capacities = Capacities { rows: 2, text: 4 };
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true]);
        kit::assert_matches_direct(&split, &direct);
        assert!(matches!(
            split.rows.outcome,
            Outcome::Truncated { .. }
        ));
    }

    #[test]
    fn split_copy_matches_direct_walk_on_writer_and_mutation_faults() {
        use super::lexical_test_kit as kit;
        // Writer active at the gate: both copies agree the refresh is
        // unwitnessable and the replay derives the identical reason.
        let mut memory = kit::KitMemory::fixture();
        memory.writer_before = 1;
        memory.writer_after = 1;
        let capacities = split_capacities();
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true]);
        kit::assert_matches_direct(&split, &direct);
        assert!(matches!(direct.rows.outcome, Outcome::Unavailable { .. }));
        // Header mutated mid-copy: the witness-after pair disagrees and both
        // paths report the change without advancing the check. Each side
        // gets a fresh witness counter so both observe the same before/after
        // sequence. The allocation field stays valid, so the verdict is the
        // change itself rather than an inconsistent witness.
        let mutate = || {
            let mut memory = kit::KitMemory::fixture();
            memory.header_after[0x38..0x3c].copy_from_slice(&5_u32.to_le_bytes());
            memory
        };
        let direct_memory = mutate();
        let direct = kit::direct(&direct_memory, &capacities);
        let split_memory = mutate();
        let (split, report, _) = kit::split(&split_memory, &capacities);
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(
            direct.rows.outcome,
            Outcome::Unavailable {
                reason: "lexer changed during copy"
            }
        );
        assert_eq!(report.matched, [false]);
    }

    #[test]
    fn double_copy_compare_blocks_checks_on_tearing() {
        use super::lexical_test_kit as kit;
        use replay::frame::TABLE_COUNT;
        let memory = kit::KitMemory::fixture();
        let capacities = split_capacities();
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = LexicalCopyContext {
            base: kit::BASE,
            needed: [true; TABLE_COUNT],
        };
        let mut first = LexicalJob::new();
        let mut second = LexicalJob::new();
        let report = captured.copy_lexical_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true]);
        // A torn token header between the passes changes the log.
        let mut torn = kit::KitMemory::fixture();
        torn.table[16] ^= 0xff;
        let mut after = LexicalJob::new();
        copy_pass(&memory, kit::BASE, &mut after.log);
        let mut after_torn = LexicalJob::new();
        copy_pass(&torn, kit::BASE, &mut after_torn.log);
        assert!(!after.log.logs_equal(&after_torn.log));
    }

    #[test]
    fn staging_overflow_falls_back_to_typed_walk() {
        use super::lexical_test_kit as kit;
        use replay::frame::TABLE_COUNT;
        let memory = kit::KitMemory::fixture();
        let capacities = split_capacities();
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = LexicalCopyContext {
            base: kit::BASE,
            needed: [true; TABLE_COUNT],
        };
        let mut tiny = LexicalJob {
            log: crate::raw::RawLog::new(8, 8),
        };
        let report = captured.copy_lexical_refresh(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
        assert!(tiny.log.overflowed);
        assert!(tiny.grow_for_next());
    }
}
