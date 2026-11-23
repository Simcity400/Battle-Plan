
use crate::{
    clock::qpc,
    raw::RawLog,
    read_accounting,
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
    table_conversion::table,
};
use engine_allocation::{copy_vec, owned_vec};
use engine_native_memory::{Memory, cpdx_lexer::{self, TOKEN_NAME_STRIDE}, walk::Outcome};
use replay_data::{rows, rows::TableId, rows::TableRows, rows::tables};
use replay_frame::{frame::TableInput, frame::TableView};

use std::sync::Arc;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::LexicalTokens];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    pub rows: usize,
    pub text: usize,
}

collector_primitives::capacity_max!(Capacities { rows, text });

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        _state: usize,
    ) -> Result<(), &'static str> {
        let (rows, bytes) = cpdx_lexer::demand(memory, base)?;
        grow_capacity(&mut self.rows, rows);
        grow_capacity(&mut self.text, bytes);
        Ok(())
    }
}

pub struct Captured {
    rows: CaptureRows<cpdx_lexer::Row>,
    text: Arena<u8>,
    pub raw_mode: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LexicalConversionStats {
    pub hits: u64,
    pub misses: u64,
}

pub struct LexicalConversion<'a> {
    cached: Option<&'a Arc<TableRows>>,
    stats: &'a mut LexicalConversionStats,
}

impl<'a> LexicalConversion<'a> {
    pub fn new(cached: Option<&'a Arc<TableRows>>, stats: &'a mut LexicalConversionStats) -> Self {
        Self { cached, stats }
    }
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            text: Arena::new(capacity.text),
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

    pub fn release_static(&mut self) {
        self.rows.rows = Vec::new();
        self.text.first = Vec::new();
        self.reset();
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        if self.raw_mode {
            return;
        }
        grow_capacity(&mut capacity.rows, self.rows.rows.len());
        self.rows.grow(&mut capacity.rows);
        capacity.text = capacity
            .text
            .max(self.text.first.len())
            .max(self.text.required);
        grow_vec(&mut self.text.first, capacity.text);
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.capture_fixture(memory, base, game_state, hour, force_full)
        }
        #[cfg(not(any(test, feature = "test-support")))]
        {
            let _ = (memory, base, game_state);
            self.reset();
            if !self.raw_mode {
                self.rows.outcome = Outcome::Unavailable {
                    reason: if replay_data::cadence::should_walk(
                        TableId::LexicalTokens,
                        hour,
                        force_full,
                    ) {
                        "lexical acquisition not supplied"
                    } else {
                        replay_data::cadence::SKIPPED_REASON
                    },
                };
            }
            [0; N]
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn capture_fixture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        _game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.reset();
        if self.raw_mode {
            return [0];
        }
        if !replay_data::cadence::should_walk(TableId::LexicalTokens, hour, force_full) {
            self.rows.outcome = Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
            };
            return [0];
        }
        let _reads = read_accounting::table(TableId::LexicalTokens);
        let start = qpc();
        let first = cpdx_lexer::walk(memory, base, &mut self.rows.rows, &mut self.text.first);
        self.rows.outcome = first.outcome;
        self.text.len = first.text_len;
        self.text.required = first.text_required;
        [elapsed(start)]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        [match self.source() {
            Ok((outcome, rows, bytes)) => converted(outcome, rows, bytes),
            Err(failure) => failure,
        }]
    }

    pub fn table_inputs(&self, reuse: Option<&mut LexicalConversion<'_>>) -> [TableInput; N] {
        let Some(reuse) = reuse else {
            return self.tables().map(TableInput::Sampled);
        };
        let source = self.source();
        if let Ok((Outcome::Rows { .. }, rows, bytes)) = &source
            && self.text.required == self.text.len
            && let Some(cached) = reuse.cached
            && let Some(previous) = cached.rows::<tables::LexicalTokens>()
            && rows.len() == previous.len()
            && rows
                .iter()
                .zip(previous)
                .enumerate()
                .all(|(index, (row, prior))| {
                    row.token as usize == index
                        && row.token == prior.token
                        && &bytes[row.text_start..row.text_start + row.text_len]
                            == prior.literal.as_slice()
                })
        {
            reuse.stats.hits = reuse.stats.hits.saturating_add(1);
            return [TableInput::SharedComplete(Arc::clone(cached))];
        }
        if !matches!(
            self.outcomes()[0],
            Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON
            }
        ) {
            reuse.stats.misses = reuse.stats.misses.saturating_add(1);
        }
        [TableInput::Sampled(match source {
            Ok((outcome, rows, bytes)) => converted(outcome, rows, bytes),
            Err(failure) => failure,
        })]
    }

    fn source(&self) -> Result<(Outcome, &[cpdx_lexer::Row], &[u8]), TableView> {
        let outcome = self.outcomes()[0];
        if let Outcome::Unavailable { reason } = outcome {
            return Err(crate::table_conversion::unavailable(reason));
        }
        let rows = self.rows.rows.get(..outcome.len()).ok_or_else(|| {
            crate::table_conversion::unavailable("walker row count exceeds allocated buffer")
        })?;
        let bytes = self.text.first.get(..self.text.len).ok_or_else(|| {
            crate::table_conversion::unavailable("lexical text lies outside captured arena")
        })?;
        if rows.iter().any(|row| {
            row.text_start
                .checked_add(row.text_len)
                .and_then(|end| bytes.get(row.text_start..end))
                .is_none()
        }) {
            return Err(crate::table_conversion::unavailable(
                "lexical text lies outside captured arena",
            ));
        }
        Ok((outcome, rows, bytes))
    }
}

fn converted(outcome: Outcome, source: &[cpdx_lexer::Row], bytes: &[u8]) -> TableView {
    table::<tables::LexicalTokens, _>(outcome, source, |row| rows::LexicalTokenRow {
        token: row.token,
        literal: owned_vec(copy_vec(
            &bytes[row.text_start..row.text_start + row.text_len],
        )),
    })
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}


const TABLE_CHUNK: usize = 65536;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Observation {
    #[default]
    Missing,
    Failed,
    Copied(u32),
}

impl Observation {
    fn start(self) -> Option<usize> {
        match self {
            Self::Copied(start) => Some(start as usize),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct WitnessPacket {
    flag: Observation,
    header: Observation,
    again: Observation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HeaderChunk {
    first: usize,
    count: usize,
    bulk: Observation,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct TokenPacket {
    header: Observation,
    terminator: Observation,
    payload: Observation,
}

#[derive(Debug, Default)]
pub struct LexicalJob {
    base: usize,
    bytes: Vec<u8>,
    chunks: Vec<HeaderChunk>,
    tokens: Vec<TokenPacket>,
    before: WitnessPacket,
    after: WitnessPacket,
    spans: usize,
    raw_bytes: u64,
    fault: bool,
    overflowed: bool,
    has_bulk: bool,
    charge: crate::raw_memory::Charge,
    #[cfg(any(test, feature = "test-support"))]
    reconstructions: std::cell::Cell<usize>,
}

impl LexicalJob {
    pub fn new() -> Self {
        let mut job = Self::default();
        job.update_charge();
        job
    }

    fn update_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.charge.set(Capacity {
            payload_vectors: vector_bytes(&self.bytes),
            auxiliary_vectors: vector_bytes(&self.tokens) + vector_bytes(&self.chunks),
            ..Capacity::default()
        });
    }

    pub fn has_storage(&self) -> bool {
        self.bytes.capacity() != 0
    }

    pub fn clear(&mut self) {
        self.base = 0;
        self.bytes.clear();
        self.chunks.clear();
        self.tokens.clear();
        self.before = WitnessPacket::default();
        self.after = WitnessPacket::default();
        self.spans = 0;
        self.raw_bytes = 0;
        self.fault = false;
        self.overflowed = false;
        self.has_bulk = false;
    }

    fn failed(&mut self) -> Observation {
        self.spans += 1;
        self.fault = true;
        Observation::Failed
    }

    #[inline]
    fn take(&mut self, memory: &impl Memory, address: usize, len: usize) -> Observation {
        if self.overflowed {
            return Observation::Missing;
        }
        if len == 0 {
            return Observation::Copied(self.bytes.len() as u32);
        }
        if len > u32::MAX as usize {
            self.overflowed = true;
            return Observation::Missing;
        }
        if address == 0
            || address.checked_add(len).is_none()
            || !crate::raw::is_canonical_range(address, len)
        {
            return self.failed();
        }
        let start = self.bytes.len();
        if start
            .checked_add(len)
            .is_none_or(|end| end > u32::MAX as usize)
        {
            self.overflowed = true;
            return Observation::Missing;
        }
        if len > self.bytes.capacity() - start {
            if engine_allocation::reserve(&mut self.bytes, len).is_err() {
                self.overflowed = true;
                return Observation::Missing;
            }
            self.update_charge();
        }
        match len {
            1 => self.bytes.push(0),
            2 => self.bytes.extend_from_slice(&[0; 2]),
            4 => self.bytes.extend_from_slice(&[0; 4]),
            8 => self.bytes.extend_from_slice(&[0; 8]),
            16 => self.bytes.extend_from_slice(&[0; 16]),
            32 => self.bytes.extend_from_slice(&[0; 32]),
            _ => self.bytes.resize(start + len, 0),
        }
        if !memory.copy(address, &mut self.bytes[start..]) {
            self.bytes.truncate(start);
            return self.failed();
        }
        self.spans += 1;
        self.raw_bytes += len as u64;
        Observation::Copied(start as u32)
    }

    fn header(&self, observation: Observation) -> Option<[u8; 32]> {
        let start = observation.start()?;
        self.bytes.get(start..start + 32)?.try_into().ok()
    }

    fn take_payload(&mut self, memory: &impl Memory, address: usize, len: usize) -> Observation {
        if self.overflowed
            || len > u32::MAX as usize
            || address == 0
            || address.checked_add(len).is_none()
            || !crate::raw::is_canonical_range(address, len)
            || len <= self.bytes.capacity() - self.bytes.len()
        {
            return self.take(memory, address, len);
        }
        let start = self.bytes.len();
        if start
            .checked_add(len)
            .is_none_or(|end| end > u32::MAX as usize)
        {
            self.overflowed = true;
            return Observation::Missing;
        }
        let mut scratch = [0u8; 4096];
        for first in (0..len).step_by(scratch.len()) {
            let count = (len - first).min(scratch.len());
            if !memory.copy(address + first, &mut scratch[..count]) {
                self.bytes.truncate(start);
                return self.failed();
            }
            if engine_allocation::reserve(&mut self.bytes, count).is_err() {
                self.overflowed = true;
                return Observation::Missing;
            }
            self.bytes.extend_from_slice(&scratch[..count]);
        }
        self.spans += 1;
        self.raw_bytes += len as u64;
        self.update_charge();
        Observation::Copied(start as u32)
    }

    fn witness(&self, packet: WitnessPacket) -> Option<cpdx_lexer::PreparedWitness> {
        let flag = packet.flag.start()?;
        let again = packet.again.start()?;
        if self.bytes.get(flag..flag + 4)? != [0; 4] || self.bytes.get(again..again + 4)? != [0; 4]
        {
            return None;
        }
        let header = packet.header.start()?;
        cpdx_lexer::PreparedWitness::from_header(
            self.bytes.get(header..header + 64)?.try_into().ok()?,
        )
        .ok()
    }

    fn matching_witnesses(&self) -> Result<cpdx_lexer::PreparedWitness, &'static str> {
        let before = self
            .witness(self.before)
            .ok_or("lexer opening witness unavailable")?;
        let after = self
            .witness(self.after)
            .ok_or("lexer closing witness unavailable")?;
        if before != after {
            return Err("lexer changed during copy");
        }
        Ok(before)
    }

    fn observations(&self, mut visit: impl FnMut(usize, usize, Observation, bool)) {
        let witness =
            |packet: WitnessPacket, visit: &mut dyn FnMut(usize, usize, Observation, bool)| {
                if let Ok(block) = cpdx_lexer::witness_address(self.base) {
                    let writer = block + cpdx_lexer::witness_offset(cpdx_lexer::WRITER_FLAG_RVA);
                    visit(writer, 4, packet.flag, false);
                    visit(block, 64, packet.header, false);
                    visit(writer, 4, packet.again, false);
                }
            };
        witness(self.before, &mut visit);
        if let Some(before) = self.witness(self.before) {
            for chunk in &self.chunks {
                let address = before.table() + chunk.first * TOKEN_NAME_STRIDE;
                visit(address, chunk.count * TOKEN_NAME_STRIDE, chunk.bulk, true);
                let end = (chunk.first + chunk.count).min(self.tokens.len());
                for index in chunk.first..end {
                    let token = self.tokens[index];
                    let address = before.table() + index * TOKEN_NAME_STRIDE;
                    if !matches!(chunk.bulk, Observation::Copied(_)) {
                        visit(address, 32, token.header, false);
                    }
                    let Some(header) = self.header(token.header) else {
                        continue;
                    };
                    let Ok(text) = cpdx_lexer::TokenText::from_header(address, header) else {
                        continue;
                    };
                    visit(text.source() + text.len(), 1, token.terminator, false);
                    visit(text.source(), text.len(), token.payload, false);
                }
            }
        }
        witness(self.after, &mut visit);
    }

    fn reconstruct(&self) -> RawLog<1> {
        #[cfg(any(test, feature = "test-support"))]
        self.reconstructions.set(self.reconstructions.get() + 1);
        let mut log = RawLog::new(self.spans, self.bytes.len());
        if log.bytes.capacity() < self.bytes.len() || log.spans.capacity() < self.spans {
            return log;
        }
        log.bytes.extend_from_slice(&self.bytes);
        self.observations(|addr, len, observation, bulk| {
            let (ok, start) = match observation {
                Observation::Missing => return,
                Observation::Failed => (false, 0),
                Observation::Copied(start) => (true, start),
            };
            log.spans.push(crate::raw::Span {
                addr,
                len: len as u32,
                ok,
                mask: 1,
                start,
                bulk: bulk && ok,
            });
        });
        debug_assert_eq!(log.spans.len(), self.spans);
        log.fault = [self.fault];
        log.raw_bytes = [self.raw_bytes];
        log.has_bulk = self.has_bulk;
        log.overflowed = self.overflowed;
        log
    }

    pub fn note_growth(&self) {
        if !self.overflowed {
            return;
        }
        let mut first_failed = None;
        let mut last = None;
        self.observations(|address, len, observation, _| {
            if observation == Observation::Failed && first_failed.is_none() {
                first_failed = Some((address, len));
            }
            if observation != Observation::Missing {
                last = Some((address, len));
            }
        });
        crate::raw::publish_growth(
            "Lexical",
            crate::raw::Growth::Raw(crate::raw::RawGrowth {
                spans: self.spans,
                span_capacity: self.spans,
                bytes: self.bytes.len(),
                byte_capacity: self.bytes.capacity(),
                faults: crate::raw::Faults::Packet([self.fault]),
                growing: false,
                first_failed_address: first_failed.map(|value| value.0),
                first_failed_length: first_failed.map(|value| value.1),
                first_failed_mask: first_failed.map(|_| 1),
                last_address: last.map(|value| value.0),
                last_length: last.map(|value| value.1),
                last_mask: last.map(|_| 1),
                span_capacity_scope: Some("no span budget; no Span vector in packet"),
                packet_inline_bytes_excluded_from_vector_accounting: Some(
                    std::mem::size_of::<Self>(),
                ),
                refusal: None,
            }),
        );
    }

    fn complete_output_sizes(&self) -> Option<(usize, usize)> {
        if self.fault || self.overflowed {
            return None;
        }
        let before = self.matching_witnesses().ok()?;
        let total = before.maximum() as usize + 1;
        if self.tokens.len() != total {
            return None;
        }
        let table_end = before
            .table()
            .checked_add(total.checked_mul(TOKEN_NAME_STRIDE)?)?;
        let witness = cpdx_lexer::witness_address(self.base).ok()?;
        let writer = witness + cpdx_lexer::witness_offset(cpdx_lexer::WRITER_FLAG_RVA);
        let overlaps = |start: usize, end: usize, other: usize, other_end: usize| {
            start < other_end && other < end
        };
        if overlaps(before.table(), table_end, witness, witness.checked_add(64)?)
            || overlaps(before.table(), table_end, writer, writer.checked_add(4)?)
        {
            return None;
        }
        let mut required = 0usize;
        for (index, packet) in self.tokens.iter().enumerate() {
            let header = self.header(packet.header)?;
            let address = before.table() + index * TOKEN_NAME_STRIDE;
            let text = cpdx_lexer::TokenText::from_header(address, header).ok()?;
            let terminator = packet.terminator.start()?;
            if *self.bytes.get(terminator)? != 0 {
                return None;
            }
            let inline = u64::from_le_bytes(header[24..].try_into().ok()?)
                == engine_native_memory::msvc_string::INLINE_CAPACITY;
            if !inline {
                let end = text.source().checked_add(text.len())?.checked_add(1)?;
                if overlaps(text.source(), end, before.table(), table_end)
                    || overlaps(text.source(), end, witness, witness.checked_add(64)?)
                    || overlaps(text.source(), end, writer, writer.checked_add(4)?)
                {
                    return None;
                }
                if !text.is_empty() {
                    let start = packet.payload.start()?;
                    self.bytes.get(start..start.checked_add(text.len())?)?;
                }
            }
            required = required.checked_add(text.len())?;
        }
        Some((total, required))
    }

    fn decode_complete(
        &self,
        total: usize,
        rows: &mut [cpdx_lexer::Row],
        output: &mut [u8],
    ) -> Option<cpdx_lexer::Walk> {
        let before = self.witness(self.before)?;
        let mut offset = 0;
        for (index, packet) in self.tokens.iter().enumerate() {
            let header = self.header(packet.header)?;
            let address = before.table() + index * TOKEN_NAME_STRIDE;
            let text = cpdx_lexer::TokenText::from_header(address, header).ok()?;
            let inline = u64::from_le_bytes(header[24..].try_into().ok()?)
                == engine_native_memory::msvc_string::INLINE_CAPACITY;
            if !text.is_empty() {
                let start = if inline {
                    packet.header.start()?
                } else {
                    packet.payload.start()?
                };
                output[offset..offset + text.len()]
                    .copy_from_slice(&self.bytes[start..start + text.len()]);
            }
            rows[index] = cpdx_lexer::Row {
                token: index as u32,
                text_start: offset,
                text_len: text.len(),
            };
            offset += text.len();
        }
        Some(cpdx_lexer::Walk {
            outcome: Outcome::Rows { len: total },
            text_len: offset,
            text_required: offset,
        })
    }

    fn fast_decode(
        &self,
        rows: &mut [cpdx_lexer::Row],
        output: &mut [u8],
    ) -> Option<cpdx_lexer::Walk> {
        let (total, required) = self.complete_output_sizes()?;
        if rows.len() < total || output.len() < required {
            return None;
        }
        self.decode_complete(total, rows, output)
    }

    fn decode(
        &self,
        base: usize,
        rows: &mut [cpdx_lexer::Row],
        text: &mut [u8],
    ) -> cpdx_lexer::Walk {
        if base == self.base
            && let Some(walked) = self.fast_decode(rows, text)
        {
            return walked;
        }
        let log = self.reconstruct();
        let recorded = match crate::raw::Recorded::build(&log) {
            Ok(recorded) => recorded,
            Err(reason) => {
                return cpdx_lexer::Walk {
                    outcome: Outcome::Unavailable { reason },
                    text_len: 0,
                    text_required: 0,
                };
            }
        };
        let walked = cpdx_lexer::walk(&recorded, base, rows, text);
        if !matches!(walked.outcome, Outcome::Unavailable { .. })
            && let Err(reason) = self.matching_witnesses()
        {
            return cpdx_lexer::Walk {
                outcome: Outcome::Unavailable { reason },
                text_len: 0,
                text_required: 0,
            };
        }
        walked
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LexicalCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; 1],
    pub raw_bytes: [u64; 1],
}

#[derive(Debug, Clone, Copy)]
pub struct LexicalCopyContext {
    pub base: usize,
}

fn copy_witness(
    job: &mut LexicalJob,
    memory: &impl Memory,
    base: usize,
) -> (WitnessPacket, Option<cpdx_lexer::PreparedWitness>) {
    let mut packet = WitnessPacket::default();
    let result = (|| {
        let header_address = cpdx_lexer::witness_address(base).ok()?;
        let writer = header_address + cpdx_lexer::witness_offset(cpdx_lexer::WRITER_FLAG_RVA);
        packet.flag = job.take(memory, writer, 4);
        let flag = packet.flag.start()?;
        if job.bytes[flag..flag + 4] != [0; 4] {
            return None;
        }
        packet.header = job.take(memory, header_address, 64);
        packet.header.start()?;
        packet.again = job.take(memory, writer, 4);
        job.witness(packet)
    })();
    (packet, result)
}

fn copy_token_text(
    job: &mut LexicalJob,
    memory: &impl Memory,
    packet: &mut TokenPacket,
    address: usize,
) -> bool {
    let Some(header) = job.header(packet.header) else {
        return false;
    };
    let Ok(text) = cpdx_lexer::TokenText::from_header(address, header) else {
        return false;
    };
    packet.terminator = job.take(memory, text.source() + text.len(), 1);
    if packet.terminator.start().is_none() {
        return false;
    }
    let inline = u64::from_le_bytes(header[24..].try_into().unwrap())
        == engine_native_memory::msvc_string::INLINE_CAPACITY;
    if !text.is_empty() && !inline {
        packet.payload = job.take_payload(memory, text.source(), text.len());
        if packet.payload.start().is_none() {
            return false;
        }
    }
    true
}

fn copy_pass(memory: &impl Memory, base: usize, job: &mut LexicalJob) {
    let _reads = read_accounting::table(TableId::LexicalTokens);
    job.clear();
    job.base = base;
    let (before, witness) = copy_witness(job, memory, base);
    job.before = before;
    let Some(witness) = witness else { return };
    job.has_bulk = true;
    let total = witness.maximum() as usize + 1;
    let chunks = total.div_ceil(TABLE_CHUNK / TOKEN_NAME_STRIDE);
    if engine_allocation::reserve(&mut job.tokens, total).is_err()
        || engine_allocation::reserve(&mut job.chunks, chunks).is_err()
    {
        job.overflowed = true;
        return;
    }
    job.update_charge();
    let mut first = 0;
    while first < total {
        let count = (total - first).min(TABLE_CHUNK / TOKEN_NAME_STRIDE);
        let Some(address) = witness.table().checked_add(first * TOKEN_NAME_STRIDE) else {
            return;
        };
        let bulk = job.take(memory, address, count * TOKEN_NAME_STRIDE);
        job.chunks.push(HeaderChunk { first, count, bulk });
        if bulk == Observation::Missing {
            return;
        }
        for index in 0..count {
            let header_address = address + index * TOKEN_NAME_STRIDE;
            let header = match bulk {
                Observation::Copied(start) => {
                    Observation::Copied(start + (index * TOKEN_NAME_STRIDE) as u32)
                }
                Observation::Failed => job.take(memory, header_address, 32),
                Observation::Missing => unreachable!(),
            };
            let mut packet = TokenPacket {
                header,
                ..Default::default()
            };
            let complete = copy_token_text(job, memory, &mut packet, header_address);
            job.tokens.push(packet);
            if !complete || job.overflowed {
                return;
            }
        }
        first += count;
    }
    job.after = copy_witness(job, memory, base).0;
}

pub fn copy_lexical_refresh(
    memory: &impl Memory,
    ctx: LexicalCopyContext,
    job: &mut LexicalJob,
) -> LexicalCopyReport {
    let started = qpc();
    copy_pass(memory, ctx.base, job);
    job.update_charge();
    LexicalCopyReport {
        raw_ok: !job.overflowed,
        copy_ticks: [elapsed(started)],
        raw_bytes: [job.raw_bytes],
    }
}

impl Captured {
    pub fn fail_lexical_capture(&mut self, reason: &'static str) {
        self.rows.outcome = Outcome::Unavailable { reason };
        self.raw_mode = false;
    }

    pub fn decode_lexical_payload(
        &mut self,
        base: usize,
        job: &LexicalJob,
        decode_ticks: &mut [u64; 1],
    ) {
        let started = qpc();
        let complete = (base == job.base)
            .then(|| job.complete_output_sizes())
            .flatten();
        let (rows, text) = complete.unwrap_or((job.tokens.len(), job.bytes.len()));
        let rows_capacity = rows.max(self.rows.rows.len());
        let text_capacity = text.max(self.text.first.len());
        if engine_allocation::resize_default(&mut self.rows.rows, rows_capacity).is_err()
            || engine_allocation::resize_default(&mut self.text.first, text_capacity).is_err()
        {
            self.fail_lexical_capture("collector allocation failed");
            decode_ticks[0] += elapsed(started);
            return;
        }
        let walked = if complete.is_some() {
            job.decode_complete(rows, &mut self.rows.rows, &mut self.text.first)
                .expect("validated complete lexical packet")
        } else {
            job.decode(base, &mut self.rows.rows, &mut self.text.first)
        };
        self.rows.outcome = walked.outcome;
        self.text.len = walked.text_len;
        self.text.required = walked.text_required;
        decode_ticks[0] += elapsed(started);
        self.raw_mode = false;
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod lexical_test_kit {
    use super::{
        Capacities, Captured, LexicalCopyContext, LexicalCopyReport, LexicalJob,
        copy_lexical_refresh,
    };
    use engine_native_memory::{cpdx_lexer, msvc_string};
    use engine_native_memory::cpdx_lexer::{
        DYNAMIC_CAPACITY_RVA,
        DYNAMIC_COUNT_RVA,
        DYNAMIC_TOKENS_RVA,
        INITIALIZED_RVA,
        MAX_TOKEN_ID_RVA,
        STATIC_MAX_TOKEN_ID_RVA,
        TOKEN_ALLOCATION_RVA,
        TOKEN_CAPACITY_RVA,
        TOKEN_NAMES_RVA,
        WRITER_FLAG_RVA,
        witness_offset,
    };

    pub const BASE: usize = 0x140000000;
    const TABLE_BASE: usize = 0x20000000;
    const HEAP_BASE: usize = 0x30000000;

    #[derive(Clone)]
    pub struct KitMemory {
        pub writer_before: u32,
        pub header_before: [u8; 64],
        pub table: Vec<u8>,
        pub heap: Vec<u8>,
    }

    impl KitMemory {
        pub fn header(maximum: u32, table: usize, length: u32) -> [u8; 64] {
            let mut header = [0u8; 64];
            header[0] = 1;
            header[witness_offset(WRITER_FLAG_RVA)..witness_offset(WRITER_FLAG_RVA) + 4]
                .copy_from_slice(&0_u32.to_le_bytes());
            header[witness_offset(MAX_TOKEN_ID_RVA)..witness_offset(MAX_TOKEN_ID_RVA) + 4]
                .copy_from_slice(&maximum.to_le_bytes());
            header[witness_offset(STATIC_MAX_TOKEN_ID_RVA)
                ..witness_offset(STATIC_MAX_TOKEN_ID_RVA) + 4]
                .copy_from_slice(&maximum.to_le_bytes());
            header[witness_offset(TOKEN_NAMES_RVA)..witness_offset(TOKEN_NAMES_RVA) + 8]
                .copy_from_slice(&(table as u64).to_le_bytes());
            header[witness_offset(TOKEN_ALLOCATION_RVA)
                ..witness_offset(TOKEN_ALLOCATION_RVA) + 4]
                .copy_from_slice(&length.to_le_bytes());
            header[witness_offset(TOKEN_CAPACITY_RVA)..witness_offset(TOKEN_CAPACITY_RVA) + 4]
                .copy_from_slice(&length.to_le_bytes());
            header[witness_offset(DYNAMIC_CAPACITY_RVA)
                ..witness_offset(DYNAMIC_CAPACITY_RVA) + 4]
                .copy_from_slice(&0_u32.to_le_bytes());
            header[witness_offset(DYNAMIC_COUNT_RVA)..witness_offset(DYNAMIC_COUNT_RVA) + 4]
                .copy_from_slice(&0_u32.to_le_bytes());
            header[witness_offset(DYNAMIC_TOKENS_RVA)..witness_offset(DYNAMIC_TOKENS_RVA) + 8]
                .copy_from_slice(&0_u64.to_le_bytes());
            header
        }

        pub fn inline(table: &mut [u8], token: usize, value: &[u8]) {
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

        pub fn fixture() -> Self {
            let mut table = vec![0u8; 3 * 32];
            let mut heap = Vec::new();
            Self::inline(&mut table, 0, b"ab");
            Self::heap(&mut table, &mut heap, 1, b"heap-token-lexical!");
            Self::inline(&mut table, 2, b"");
            let header = Self::header(2, TABLE_BASE, 3);
            Self {
                writer_before: 0,
                header_before: header,
                table,
                heap,
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

    impl engine_native_memory::Memory for KitMemory {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if destination.is_empty() {
                return true;
            }
            if address == BASE + WRITER_FLAG_RVA.0 && destination.len() == 4 {
                destination.copy_from_slice(&self.writer_before.to_le_bytes());
                return true;
            }
            if address == BASE + INITIALIZED_RVA.0 && destination.len() == 64 {
                destination.copy_from_slice(&self.header_before);
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

    pub fn direct(memory: &KitMemory, capacities: &Capacities) -> Captured {
        let mut captured = Captured::with_capacities(capacities);
        let walked = cpdx_lexer::walk(
            memory,
            BASE,
            &mut captured.rows.rows,
            &mut captured.text.first,
        );
        captured.rows.outcome = walked.outcome;
        captured.text.len = walked.text_len;
        captured.text.required = walked.text_required;
        captured
    }

    pub fn split(
        memory: &KitMemory,
        capacities: &Capacities,
    ) -> (Captured, LexicalCopyReport, LexicalJob) {
        let mut captured = Captured::with_capacities(capacities);
        let ctx = LexicalCopyContext { base: BASE };
        let mut first = LexicalJob::new();
        let report = copy_lexical_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        let mut ticks = [0u64; 1];
        captured.decode_lexical_payload(BASE, &first, &mut ticks);
        (captured, report, first)
    }

    pub fn allocation_failure_decode_exercise(
        rows: usize,
        text: usize,
    ) -> impl FnOnce() -> bool + Send + 'static {
        let memory = KitMemory::fixture();
        let mut packet = LexicalJob::new();
        let report = copy_lexical_refresh(&memory, LexicalCopyContext { base: BASE }, &mut packet);
        let expected = packet.complete_output_sizes();
        let mut captured = Captured::with_capacities(&Capacities { rows, text });
        captured.raw_mode = true;
        move || {
            let _decode = crate::allocation::enter(crate::allocation::Phase::Decode);
            let mut ticks = [0];
            captured.decode_lexical_payload(BASE, &packet, &mut ticks);
            let outcome = captured.rows.outcome;
            report.raw_ok
                && expected.is_some()
                && !captured.raw_mode
                && if crate::allocation::failed() {
                    outcome
                        == engine_native_memory::walk::Outcome::Unavailable {
                            reason: "collector allocation failed",
                        }
                } else {
                    expected.is_some_and(|(rows, text)| {
                        outcome == engine_native_memory::walk::Outcome::Rows { len: rows }
                            && captured.text.len == text
                    })
                }
        }
    }

    pub fn assert_matches_direct(split: &Captured, direct: &Captured) {
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

    struct TracedMemory {
        inner: lexical_test_kit::KitMemory,
        calls: std::cell::RefCell<Vec<(usize, usize)>>,
        faults: Vec<usize>,
        replacements: Vec<(usize, Vec<u8>)>,
    }

    impl Memory for TracedMemory {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let ordinal = self.calls.borrow().len();
            self.calls.borrow_mut().push((address, output.len()));
            if self.faults.contains(&ordinal) {
                let partial = output.len().div_ceil(2);
                output[..partial].fill(0xee);
                return false;
            }
            if let Some((_, bytes)) = self.replacements.iter().find(|(at, _)| *at == ordinal) {
                assert_eq!(bytes.len(), output.len());
                output.copy_from_slice(bytes);
                return true;
            }
            self.inner.copy(address, output)
        }
    }

    fn recycled(bytes: usize) -> LexicalJob {
        let mut job = LexicalJob::new();
        engine_allocation::reserve_exact(&mut job.bytes, bytes).unwrap();
        job.update_charge();
        job
    }

    fn assert_packet_trace(
        memory: lexical_test_kit::KitMemory,
        faults: &[usize],
        replacements: &[(usize, Vec<u8>)],
        bytes: usize,
    ) -> LexicalJob {
        let traced = |inner| TracedMemory {
            inner,
            calls: Default::default(),
            faults: faults.to_vec(),
            replacements: replacements.to_vec(),
        };
        let reference_memory = traced(memory.clone());
        let packet_memory = traced(memory);
        let mut reference = RawLog::<1>::new(0, 0);
        let terminal =
            scalar_reference::copy_pass(&reference_memory, lexical_test_kit::BASE, &mut reference);
        let mut packet = recycled(bytes);
        copy_pass(&packet_memory, lexical_test_kit::BASE, &mut packet);
        assert_eq!(
            *packet_memory.calls.borrow(),
            *reference_memory.calls.borrow(),
            "source order"
        );
        let reconstructed = packet.reconstruct();
        assert_eq!(
            reconstructed.spans, reference.spans,
            "exact observation descriptors"
        );
        assert_eq!(
            reconstructed.bytes, reference.bytes,
            "successful owned bytes"
        );
        assert_eq!(reconstructed.raw_bytes, reference.raw_bytes);
        assert_eq!(reconstructed.fault, reference.fault);
        assert_eq!(reconstructed.has_bulk, reference.has_bulk);
        assert_eq!(packet.overflowed, reference.overflowed);
        for (rows, bytes) in [(0, 0), (1, 0), (1, 2), (2, 4), (3, 21), (4, 128)] {
            let mut expected_rows = vec![cpdx_lexer::Row::default(); rows];
            let mut actual_rows = expected_rows.clone();
            let mut expected_text = vec![0xcc; bytes];
            let mut actual_text = expected_text.clone();
            let recorded = crate::raw::Recorded::build(&reference).unwrap();
            let mut expected = cpdx_lexer::walk(
                &recorded,
                lexical_test_kit::BASE,
                &mut expected_rows,
                &mut expected_text,
            );
            if !matches!(expected.outcome, Outcome::Unavailable { .. })
                && let Err(reason) = terminal
            {
                expected = cpdx_lexer::Walk {
                    outcome: Outcome::Unavailable { reason },
                    text_len: 0,
                    text_required: 0,
                };
            }
            let actual = packet.decode(lexical_test_kit::BASE, &mut actual_rows, &mut actual_text);
            assert_eq!(actual, expected, "rows={rows} text={bytes}");
            assert_eq!(actual_rows, expected_rows, "written rows");
            assert_eq!(
                actual_text, expected_text,
                "written arena including discarded prefix"
            );
        }
        packet
    }

    fn heap_source(
        memory: &mut lexical_test_kit::KitMemory,
        token: usize,
        source: usize,
        len: usize,
    ) {
        let header = &mut memory.table[token * 32..(token + 1) * 32];
        header.fill(0);
        header[..8].copy_from_slice(&(source as u64).to_le_bytes());
        header[16..24].copy_from_slice(&(len as u64).to_le_bytes());
        header[24..].copy_from_slice(&(len.max(16) as u64).to_le_bytes());
    }

    #[test]
    fn packet_matches_scalar_at_every_fault_and_retained_byte_capacity() {
        use lexical_test_kit::KitMemory;
        assert_packet_trace(KitMemory::fixture(), &[], &[], 0);
        for fault in 0..11 {
            assert_packet_trace(KitMemory::fixture(), &[fault], &[], 0);
        }
        for fault in 4..14 {
            assert_packet_trace(KitMemory::fixture(), &[3, fault], &[], 0);
        }
        for bytes in 0..264 {
            assert_packet_trace(KitMemory::fixture(), &[], &[], bytes);
            assert_packet_trace(KitMemory::fixture(), &[3], &[], bytes);
        }
    }

    #[test]
    fn packet_rejects_busy_or_changed_closing_witness_without_live_retry() {
        use lexical_test_kit::{BASE, KitMemory};
        for ordinal in [0, 2, 8, 10] {
            let job = assert_packet_trace(
                KitMemory::fixture(),
                &[],
                &[(ordinal, 1_u32.to_le_bytes().to_vec())],
                0,
            );
            assert!(job.complete_output_sizes().is_none());
            let walked = job.decode(BASE, &mut [cpdx_lexer::Row::default(); 3], &mut [0; 32]);
            assert!(matches!(walked.outcome, Outcome::Unavailable { .. }));
        }
        let mut changed = KitMemory::fixture().header_before;
        let offset = witness_offset(TOKEN_NAMES_RVA);
        changed[offset..offset + 8].copy_from_slice(&0x20010000_u64.to_le_bytes());
        let job = assert_packet_trace(KitMemory::fixture(), &[], &[(9, changed.to_vec())], 0);
        assert!(job.complete_output_sizes().is_none());
        let walked = job.decode(BASE, &mut [cpdx_lexer::Row::default(); 3], &mut [0; 32]);
        assert_eq!(
            walked.outcome,
            Outcome::Unavailable {
                reason: "lexer changed during copy"
            }
        );
    }

    #[test]
    fn packet_normal_complete_decode_never_reconstructs_a_recorded_index() {
        let mut job = LexicalJob::new();
        copy_pass(
            &lexical_test_kit::KitMemory::fixture(),
            lexical_test_kit::BASE,
            &mut job,
        );
        let mut rows = [cpdx_lexer::Row::default(); 3];
        let mut text = [0; 32];
        assert_eq!(
            job.decode(lexical_test_kit::BASE, &mut rows, &mut text)
                .outcome,
            Outcome::Rows { len: 3 }
        );
        assert_eq!(job.reconstructions.get(), 0);
        assert_eq!(&text[..21], b"abheap-token-lexical!");
        assert!(matches!(
            job.decode(0, &mut rows, &mut text).outcome,
            Outcome::Unavailable { .. }
        ));
        assert_eq!(job.reconstructions.get(), 1);
    }

    #[test]
    fn packet_faults_missing_witness_and_readable_nonzero_terminators_replay_exactly() {
        use lexical_test_kit::KitMemory;
        let mut nonzero = KitMemory::fixture();
        nonzero.table[2] = 1;
        let job = assert_packet_trace(nonzero, &[], &[], 0);
        assert!(
            job.after.again.start().is_some(),
            "nonzero byte does not stop capture"
        );
        assert!(
            job.fast_decode(&mut [cpdx_lexer::Row::default(); 3], &mut [0; 128])
                .is_none()
        );
        let mut malformed = KitMemory::fixture();
        malformed.table[48..56].copy_from_slice(&100u64.to_le_bytes());
        let job = assert_packet_trace(malformed, &[], &[], 0);
        assert_eq!(job.after, WitnessPacket::default());
        let walked = job.decode(
            lexical_test_kit::BASE,
            &mut [cpdx_lexer::Row::default(); 1],
            &mut [0; 2],
        );
        assert_eq!(
            walked.outcome,
            Outcome::Unavailable {
                reason: "lexer closing witness unavailable"
            }
        );
        assert_eq!((walked.text_len, walked.text_required), (0, 0));
        let walked = job.decode(
            lexical_test_kit::BASE,
            &mut [cpdx_lexer::Row::default(); 3],
            &mut [0; 32],
        );
        assert_eq!(
            walked.outcome,
            Outcome::Unavailable {
                reason: "lexer token string header invalid"
            }
        );
        let job = assert_packet_trace(KitMemory::fixture(), &[3], &[], 0);
        assert!(
            job.fault,
            "successful scalar fallback never clears failed bulk"
        );
        assert!(job.after.again.start().is_some());
        let log = job.reconstruct();
        assert!(!log.spans[3].ok && !log.spans[3].bulk);
    }

    #[test]
    fn packet_payload_header_and_skipped_witness_aliases_use_owned_reconstruction() {
        use lexical_test_kit::{BASE, KitMemory};
        let mut header_alias = KitMemory::fixture();
        heap_source(&mut header_alias, 0, 0x20000020, 32);
        let mut replacement = vec![0; 32];
        KitMemory::inline(&mut replacement, 0, b"changed");
        let job = assert_packet_trace(header_alias, &[], &[(5, replacement)], 1024);
        assert!(
            job.fast_decode(&mut [cpdx_lexer::Row::default(); 3], &mut [0; 128])
                .is_none()
        );
        for (source, len) in [(BASE + WRITER_FLAG_RVA.0, 4), (BASE + INITIALIZED_RVA.0, 64)] {
            let mut memory = KitMemory::fixture();
            heap_source(&mut memory, 0, source, len);
            let job = assert_packet_trace(memory, &[], &[(4, vec![0])], 1024);
            assert!(
                job.fast_decode(&mut [cpdx_lexer::Row::default(); 3], &mut [0; 128])
                    .is_none()
            );
        }
    }

    #[test]
    fn packet_shared_heap_keys_keep_full_consumption_observation_order() {
        use lexical_test_kit::{BASE, KitMemory};
        let mut shared = KitMemory::fixture();
        shared.heap = vec![b'x', 0];
        heap_source(&mut shared, 0, 0x30000000, 1);
        heap_source(&mut shared, 1, 0x30000000, 0);
        heap_source(&mut shared, 2, 0x30000000, 1);
        let job = assert_packet_trace(shared, &[], &[(6, vec![0]), (8, vec![b'y'])], 0);
        let mut rows = [cpdx_lexer::Row::default(); 3];
        let mut text = [0; 16];
        let before = job.reconstructions.get();
        let walked = job.decode(BASE, &mut rows, &mut text);
        assert_eq!(walked.outcome, Outcome::Rows { len: 3 });
        assert_eq!(&text[..2], b"xy");
        assert_eq!(job.reconstructions.get(), before);
        let log = job.reconstruct();
        let first = crate::raw::Recorded::build(&log).unwrap();
        let second = crate::raw::Recorded::build(&log).unwrap();
        let mut one = [0];
        let mut two = [0];
        for expected in [b'x', 0, b'y', b'y'] {
            assert!(first.copy(0x30000000, &mut one));
            assert!(second.copy(0x30000000, &mut two));
            assert_eq!(one, [expected]);
            assert_eq!(two, one);
        }
    }

    #[test]
    fn packet_final_single_header_chunk_and_reused_job_drop_old_shape() {
        use lexical_test_kit::{BASE, KitMemory};
        let mut memory = KitMemory::fixture();
        memory.table = vec![0; 2049 * 32];
        for token in 0..2049 {
            KitMemory::inline(&mut memory.table, token, b"a");
        }
        memory.header_before = KitMemory::header(2048, 0x20000000, 2049);
        let mut job = assert_packet_trace(memory, &[], &[], 100_000);
        assert_eq!(job.chunks.len(), 2);
        assert_eq!(job.chunks[1].count, 1);
        let log = job.reconstruct();
        assert!(
            log.spans
                .iter()
                .any(|span| span.ok && span.bulk && span.len == 32)
        );
        let mut rows = vec![cpdx_lexer::Row::default(); 2049];
        assert!(job.fast_decode(&mut rows, &mut vec![0; 2049]).is_some());
        let capacities = (
            job.tokens.capacity(),
            job.chunks.capacity(),
            job.bytes.capacity(),
        );
        copy_pass(&KitMemory::fixture(), BASE, &mut job);
        assert_eq!((job.tokens.len(), job.chunks.len()), (3, 1));
        assert_eq!(
            (
                job.tokens.capacity(),
                job.chunks.capacity(),
                job.bytes.capacity()
            ),
            capacities
        );
        let before = job.reconstructions.get();
        assert!(job.fast_decode(&mut rows, &mut [0; 32]).is_some());
        assert_eq!(job.reconstructions.get(), before);
    }

    #[test]
    fn exceptional_large_owned_payload_keeps_exact_observations_with_small_outputs() {
        use lexical_test_kit::{BASE, KitMemory};
        let mut memory = KitMemory::fixture();
        memory.heap = vec![b'x'; 8192];
        memory.heap.push(0);
        heap_source(&mut memory, 0, 0x30000000, 8192);
        heap_source(&mut memory, 1, BASE + INITIALIZED_RVA.0, 64);
        let memory = TracedMemory {
            inner: memory,
            calls: Default::default(),
            faults: vec![],
            replacements: vec![(6, vec![0])],
        };
        let mut job = recycled(16384);
        copy_pass(&memory, BASE, &mut job);
        assert!(!job.fault && !job.overflowed);
        assert!(job.complete_output_sizes().is_none());
        let calls = memory.calls.borrow().len();
        let mut captured = Captured::with_capacities(&Capacities { rows: 0, text: 0 });
        captured.decode_lexical_payload(BASE, &job, &mut [0]);
        assert_eq!(memory.calls.borrow().len(), calls);
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 3 }]);
        assert_eq!(captured.text.len, 8192 + 64);
        assert_eq!(&captured.text.first[..8192], &[b'x'; 8192]);
        assert_eq!(
            &captured.text.first[8192..8192 + 64],
            &memory.inner.header_before
        );
    }

    #[test]
    fn unacquired_huge_payload_grows_only_through_its_readable_prefix() {
        use lexical_test_kit::{BASE, KitMemory};
        const HEAP: usize = 0x30000000;
        const DECLARED: usize = 1 << 30;
        struct Huge {
            inner: KitMemory,
            readable: usize,
            probes: std::cell::Cell<usize>,
        }
        impl Memory for Huge {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == HEAP + DECLARED && output.len() == 1 {
                    output[0] = 0;
                    return true;
                }
                if (HEAP..HEAP + DECLARED).contains(&address) {
                    self.probes.set(self.probes.get() + 1);
                    if address - HEAP + output.len() > self.readable {
                        return false;
                    }
                    output.fill(b'x');
                    return true;
                }
                self.inner.copy(address, output)
            }
        }
        for readable in [0, 4096] {
            let mut inner = KitMemory::fixture();
            heap_source(&mut inner, 1, HEAP, DECLARED);
            let memory = Huge {
                inner,
                readable,
                probes: std::cell::Cell::new(0),
            };
            let mut job = LexicalJob::new();
            let mut capacities = Capacities { rows: 3, text: 32 };
            let mut captured = Captured::with_capacities(&capacities);
            for attempt in 0..3 {
                memory.probes.set(0);
                copy_pass(&memory, BASE, &mut job);
                assert!(job.fault && !job.overflowed);
                assert_eq!(memory.probes.get(), if readable == 0 { 1 } else { 2 });
                assert!(job.bytes.capacity() < 4 * 4096);
                assert!(matches!(
                    job.decode(BASE, &mut [cpdx_lexer::Row::default(); 3], &mut [0; 32])
                        .outcome,
                    Outcome::Unavailable { .. }
                ));

                let may_grow = readable != 0 && attempt == 0;
                memory.probes.set(0);
                captured.capture(&memory, BASE, 0, 0, true);
                assert_eq!(
                    matches!(captured.outcomes()[0], Outcome::Truncated { .. }),
                    may_grow
                );
                captured.grow(&mut capacities);
                let allocated = capacities.text;
                captured.grow(&mut capacities);
                assert_eq!(capacities.text, allocated);
                assert_eq!(allocated, if readable == 0 { 32 } else { 4096 });
                assert_eq!(
                    memory.probes.get(),
                    if readable == 0 || attempt == 0 { 1 } else { 2 }
                );
            }
        }
    }

    #[test]
    fn packet_invalid_addresses_fail_without_native_reads() {
        let memory = TracedMemory {
            inner: lexical_test_kit::KitMemory::fixture(),
            calls: Default::default(),
            faults: vec![],
            replacements: vec![],
        };
        for source in [0, usize::MAX, 0x0000_8000_0000_0000] {
            let mut job = LexicalJob::new();
            let mut raw = RawLog::<1>::new(0, 0);
            let result = job.take(&memory, source, 4);
            let expected = raw.take(&memory, source, 4, 1);
            assert_eq!(result.start(), expected);
            assert_eq!(job.spans, raw.spans.len());
            assert_eq!([job.fault], raw.fault);
            assert_eq!(job.overflowed, raw.overflowed);
        }
        assert!(memory.calls.borrow().is_empty());
        assert_eq!(
            Observation::Copied(u32::MAX - 1).start(),
            Some(u32::MAX as usize - 1)
        );
    }

    #[test]
    fn packet_vector_lifetime_charge_survives_clear_and_releases_on_drop() {
        use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
        static LOCAL: Tracker = Tracker::new();
        let mut job = LexicalJob::new();
        job.charge = Charge::empty_on(&LOCAL);
        job.update_charge();
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
        copy_pass(
            &lexical_test_kit::KitMemory::fixture(),
            lexical_test_kit::BASE,
            &mut job,
        );
        let grown = Capacity {
            payload_vectors: vector_bytes(&job.bytes),
            auxiliary_vectors: vector_bytes(&job.tokens) + vector_bytes(&job.chunks),
            ..Default::default()
        };
        assert_eq!(
            LOCAL.snapshot().current,
            grown,
            "the pass charges its growth"
        );
        assert!(grown.auxiliary_vectors > 0);
        let charged = LOCAL.snapshot();
        job.clear();
        assert_eq!(LOCAL.snapshot(), charged);
        let memory = lexical_test_kit::KitMemory::fixture();
        let required = job.bytes.capacity() + 1;
        job.take(&memory, 0x20000000, required);
        assert_eq!(
            LOCAL.snapshot().current.payload_vectors,
            vector_bytes(&job.bytes)
        );
        assert!(LOCAL.snapshot().current.payload_vectors > grown.payload_vectors);
        let charged_after_growth = LOCAL.snapshot();
        job.clear();
        let mut moved = Some(job);
        assert_eq!(LOCAL.snapshot(), charged_after_growth);
        let moved_job = moved.take().unwrap();
        drop(moved);
        assert_eq!(LOCAL.snapshot(), charged_after_growth);
        drop(moved_job);
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
        assert_eq!(
            LOCAL.snapshot().peak_total_lower_bound,
            charged_after_growth.peak_total_lower_bound
        );
    }

    mod scalar_reference {
        use super::*;
        #[derive(PartialEq, Eq)]
        struct Witness {
            maximum: u32,
            table: usize,
            static_maximum: u32,
            dynamic_pointer: u64,
            dynamic_capacity: u32,
            dynamic_count: u32,
            allocation: u32,
            length: u32,
        }

        fn copy_witness(log: &mut RawLog<1>, memory: &impl Memory, base: usize) -> Option<Witness> {
            use engine_native_memory::cpdx_lexer::*;
            let writer = base.checked_add(WRITER_FLAG_RVA.0)?;
            let mut flag = [0u8; 4];
            let start = log.take(memory, writer, 4, 0x01)?;
            flag.copy_from_slice(&log.bytes[start..start + 4]);
            if u32::from_le_bytes(flag) != 0 {
                return None;
            }
            let header_addr = base.checked_add(INITIALIZED_RVA.0)?;
            let header_start = log.take(memory, header_addr, 64, 0x01)?;
            let mut header = [0u8; 64];
            header.copy_from_slice(&log.bytes[header_start..header_start + 64]);
            let writer_again = log.take(memory, writer, 4, 0x01)?;
            if u32::from_le_bytes(
                log.bytes[writer_again..writer_again + 4]
                    .try_into()
                    .unwrap(),
            ) != 0
            {
                return None;
            }
            if header[0] != 1 {
                return None;
            }
            let signed = |offset: usize| -> Option<u32> {
                u32::try_from(i32::from_le_bytes(
                    header[offset..offset + 4].try_into().unwrap(),
                ))
                .ok()
            };
            let pointer = |offset: usize| -> usize {
                u64::from_le_bytes(header[offset..offset + 8].try_into().unwrap()) as usize
            };
            if signed(witness_offset(WRITER_FLAG_RVA))? != 0 {
                return None;
            }
            let maximum = signed(witness_offset(MAX_TOKEN_ID_RVA))?;
            let static_maximum = signed(witness_offset(STATIC_MAX_TOKEN_ID_RVA))?;
            let dynamic_pointer = u64::from_le_bytes(
                header[witness_offset(DYNAMIC_TOKENS_RVA)
                    ..witness_offset(DYNAMIC_TOKENS_RVA) + 8]
                    .try_into()
                    .unwrap(),
            );
            let dynamic_capacity = signed(witness_offset(DYNAMIC_CAPACITY_RVA))?;
            let dynamic_count = signed(witness_offset(DYNAMIC_COUNT_RVA))?;
            let table = pointer(witness_offset(TOKEN_NAMES_RVA));
            let allocation = signed(witness_offset(TOKEN_ALLOCATION_RVA))?;
            let length = signed(witness_offset(TOKEN_CAPACITY_RVA))?;
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
                .map(|_| Witness {
                    maximum,
                    table,
                    static_maximum,
                    dynamic_pointer,
                    dynamic_capacity,
                    dynamic_count,
                    allocation,
                    length,
                })
        }

        fn copy_token_text(
            log: &mut RawLog<1>,
            memory: &impl Memory,
            header: &[u8; 32],
            address: usize,
        ) -> bool {
            use engine_native_memory::msvc_string;
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
            if len != 0
                && capacity != msvc_string::INLINE_CAPACITY
                && log.take(memory, source, len, 0x01).is_none()
            {
                return false;
            }
            true
        }

        pub fn copy_pass(
            memory: &impl Memory,
            base: usize,
            log: &mut RawLog<1>,
        ) -> Result<(), &'static str> {
            let witness =
                copy_witness(log, memory, base).ok_or("lexer opening witness unavailable")?;
            let closed = (|| {
                log.has_bulk = true;
                let total = (witness.maximum as usize + 1)
                    .checked_mul(TOKEN_NAME_STRIDE)
                    .unwrap_or(0);
                let mut offset = 0usize;
                while offset < total {
                    let len = (total - offset).min(TABLE_CHUNK);
                    let address = witness.table.checked_add(offset)?;
                    match log.take_bulk(memory, address, len, 0x01) {
                        Some(start) => {
                            let tokens = len / TOKEN_NAME_STRIDE;
                            for index in 0..tokens {
                                let header_addr = address.checked_add(index * TOKEN_NAME_STRIDE)?;
                                let mut header = [0u8; 32];
                                header.copy_from_slice(
                                    &log.bytes[start + index * TOKEN_NAME_STRIDE
                                        ..start + (index + 1) * TOKEN_NAME_STRIDE],
                                );
                                if !copy_token_text(log, memory, &header, header_addr) {
                                    return None;
                                }
                                if log.overflowed {
                                    return None;
                                }
                            }
                        }
                        None if log.overflowed => return None,
                        None => {
                            let tokens = len / TOKEN_NAME_STRIDE;
                            for index in 0..tokens {
                                let header_addr = witness
                                    .table
                                    .checked_add(offset + index * TOKEN_NAME_STRIDE)?;
                                let header_start = log.take(memory, header_addr, 32, 0x01)?;
                                let mut header = [0u8; 32];
                                header.copy_from_slice(&log.bytes[header_start..header_start + 32]);
                                if !copy_token_text(log, memory, &header, header_addr) {
                                    return None;
                                }
                                if log.overflowed {
                                    return None;
                                }
                            }
                        }
                    }
                    offset += len;
                }
                copy_witness(log, memory, base).map(|after| after == witness)
            })();
            match closed {
                Some(true) => Ok(()),
                Some(false) => Err("lexer changed during copy"),
                None => Err("lexer closing witness unavailable"),
            }
        }
    }

    use engine_native_memory::{cpdx_lexer::*, memory::SliceMemory};
    use std::cell::Cell;
    struct MemoryFixture {
        header: [u8; 64],
        names: [u8; 64],
        copies: Cell<usize>,
    }

    impl Memory for MemoryFixture {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == WRITER_FLAG_RVA.0 {
                destination.fill(0);
                return true;
            }
            if address == INITIALIZED_RVA.0 {
                destination.copy_from_slice(&self.header);
                return true;
            }
            let count = self.copies.get();
            if address == 0x1000 && destination.len() == 32 {
                self.copies.set(count + 1);
            }
            SliceMemory {
                base: 0x1000,
                bytes: &self.names,
            }
            .copy(address, destination)
        }
    }

    fn fixture() -> MemoryFixture {
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
        }
    }

    #[test]
    fn lexical_pack_preserves_exact_first_bytes() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 2, text: 32 });
        let memory = fixture();
        captured.capture(&memory, 0, 0, 3, true);
        let [TableView::Complete(rows)] = captured.tables() else {
            panic!("complete dictionary expected")
        };
        let row = &rows.rows::<tables::LexicalTokens>().unwrap()[0];
        assert_eq!(row.literal, [255]);
        assert_eq!(row.token, 0);
        assert_eq!(memory.copies.get(), 1);
    }

    #[test]
    fn lexical_pool_grows_after_primary_truncation() {
        let mut capacity = Capacities { rows: 1, text: 0 };
        let mut captured = Captured::with_capacities(&capacity);
        captured.capture(&fixture(), 0, 0, 3, true);
        assert_eq!(
            captured.outcomes()[0],
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        captured.grow(&mut capacity);
        assert!(capacity.text >= 1);
        captured.capture(&fixture(), 0, 0, 3, true);
        assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 1 });
    }

    #[test]
    fn lexical_reads_the_dictionary_on_full_hours_only() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 2, text: 32 });
        let memory = fixture();
        for hour in -24..24 {
            let before = memory.copies.get();
            captured.capture(&memory, 0, 0, hour, false);
            assert_eq!(memory.copies.get(), before);
            assert_eq!(
                captured.outcomes()[0],
                Outcome::Unavailable {
                    reason: replay_data::cadence::SKIPPED_REASON,
                }
            );
        }
        let before = memory.copies.get();
        captured.capture(&memory, 0, 0, 5, true);
        assert_eq!(memory.copies.get(), before + 1);
        assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 1 });
        let [TableView::Complete(rows)] = captured.tables() else {
            panic!("complete dictionary expected on a full hour");
        };
        assert_eq!(
            rows.rows::<tables::LexicalTokens>().unwrap()[0].literal,
            [255]
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
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.rows.outcome, Outcome::Rows { len: 3 });
        assert_eq!(split.rows.rows[0].token, 0);
        assert_eq!(&split.text.first[0..2], b"ab");
        assert_eq!(split.rows.rows[1].text_len, 19);
        assert_eq!(split.rows.rows[2].text_len, 0);
    }

    #[test]
    fn complete_lexical_payload_preserves_all_rows_across_destination_capacities() {
        use super::lexical_test_kit as kit;
        let memory = kit::KitMemory::fixture();
        let direct = kit::direct(&memory, &split_capacities());
        for capacities in [
            Capacities { rows: 0, text: 0 },
            Capacities { rows: 1, text: 2 },
            Capacities { rows: 2, text: 4 },
            Capacities { rows: 3, text: 32 },
            Capacities { rows: 4, text: 64 },
            Capacities {
                rows: 256,
                text: 8192,
            },
        ] {
            let (split, report, _) = kit::split(&memory, &capacities);
            assert!(
                report.raw_ok,
                "rows={} text={}",
                capacities.rows, capacities.text
            );
            kit::assert_matches_direct(&split, &direct);
        }
        let exact = Capacities { rows: 3, text: 32 };
        let (split, _, _) = kit::split(&memory, &exact);
        assert_eq!(split.rows.outcome, Outcome::Rows { len: 3 });
        let mut torn = kit::KitMemory::fixture();
        torn.table[16 + 32] ^= 0xff;
        let capacities = split_capacities();
        let direct = kit::direct(&torn, &capacities);
        let (split, report, _) = kit::split(&torn, &capacities);
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
    }

    #[test]
    fn lexical_bulk_chunk_serves_enclosed_header_words() {
        use super::lexical_test_kit as kit;
        let memory = kit::KitMemory::fixture();
        let mut job = LexicalJob::new();
        copy_pass(&memory, kit::BASE, &mut job);
        assert!(job.has_bulk);
        let log = job.reconstruct();
        let recorded = crate::raw::Recorded::build(&log).unwrap();
        let chunk = log
            .spans
            .iter()
            .find(|span| span.ok && span.bulk)
            .expect("bulk chunk");
        let mut word = [0u8; 8];
        assert!(recorded.copy(chunk.addr + 3, &mut word));
        let mut expected = [0u8; 8];
        assert!(memory.copy(chunk.addr + 3, &mut expected));
        assert_eq!(word, expected);
        let mut header = [0u8; 32];
        assert!(recorded.copy(chunk.addr, &mut header));
        let mut straddle = [0u8; 32];
        assert!(!recorded.copy(chunk.addr + chunk.len as usize - 16, &mut straddle));
    }

    #[test]
    fn complete_payload_grows_small_outputs_without_new_native_reads() {
        use super::lexical_test_kit as kit;
        let mut memory = TracedMemory {
            inner: kit::KitMemory::fixture(),
            calls: Default::default(),
            faults: Vec::new(),
            replacements: Vec::new(),
        };
        let direct = kit::direct(&memory.inner, &split_capacities());
        let mut job = LexicalJob::new();
        copy_pass(&memory, kit::BASE, &mut job);
        let native_reads = memory.calls.borrow().len();
        memory.inner.table.fill(0);
        memory.inner.heap.clear();
        let capacities = Capacities { rows: 2, text: 4 };
        let mut split = Captured::with_capacities(&capacities);
        let before_bytes = split.allocated_bytes();
        split.decode_lexical_payload(kit::BASE, &job, &mut [0]);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.rows.outcome, Outcome::Rows { len: 3 });
        assert!(split.allocated_bytes() > before_bytes);
        assert_eq!(memory.calls.borrow().len(), native_reads);
        assert_eq!(job.reconstructions.get(), 0);
    }

    #[test]
    fn split_copy_matches_direct_walk_on_first_writer_fault() {
        use super::lexical_test_kit as kit;
        let mut memory = kit::KitMemory::fixture();
        memory.writer_before = 1;
        let capacities = split_capacities();
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
        assert!(matches!(direct.rows.outcome, Outcome::Unavailable { .. }));
    }

    #[test]
    fn live_lexical_demand_and_same_payload_growth_keep_mod_dictionary() {
        use super::lexical_test_kit as kit;
        let mut job = LexicalJob::default();
        let mut captured = Captured::with_capacities(&Capacities { rows: 0, text: 0 });
        for count in [180_000, 220_000] {
            let mut memory = kit::KitMemory::fixture();
            memory.table = vec![0; count * TOKEN_NAME_STRIDE];
            for token in 0..count {
                kit::KitMemory::inline(&mut memory.table, token, b"mod_dictionary!");
            }
            memory.header_before =
                kit::KitMemory::header(count as u32 - 1, 0x20000000, count as u32);
            let mut capacity = Capacities::default();
            capacity.presize(&memory, kit::BASE, 0).unwrap();
            assert!(capacity.rows >= count && capacity.text >= count * 15);
            assert!(
                copy_lexical_refresh(&memory, LexicalCopyContext { base: kit::BASE }, &mut job)
                    .raw_ok
            );
            captured.decode_lexical_payload(kit::BASE, &job, &mut [0]);
            assert_eq!(captured.rows.outcome, Outcome::Rows { len: count });
            assert_eq!(captured.text.len, count * 15);
            assert_eq!(captured.rows.rows[count - 1].token, count as u32 - 1);
            assert!(job.has_storage());
            assert!(!job.overflowed);
        }
    }

    #[test]
    fn lexical_cold_storage_holds_no_content_guesses() {
        let job = LexicalJob::new();
        assert!(!job.has_storage());
        assert_eq!(
            (
                job.bytes.capacity(),
                job.tokens.capacity(),
                job.chunks.capacity()
            ),
            (0, 0, 0)
        );
        let capacity = Capacities::default();
        assert_eq!((capacity.rows, capacity.text), (0, 0));
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn lexical_reuse_fixture(value: u8) -> Captured {
    let mut captured = Captured::with_capacities(&Capacities { rows: 4, text: 16 });
    captured.rows.rows[..3].copy_from_slice(&[
        cpdx_lexer::Row {
            token: 0,
            text_start: 0,
            text_len: 0,
        },
        cpdx_lexer::Row {
            token: 1,
            text_start: 0,
            text_len: 3,
        },
        cpdx_lexer::Row {
            token: 2,
            text_start: 3,
            text_len: 2,
        },
    ]);
    captured.rows.outcome = Outcome::Rows { len: 3 };
    captured.text.first[..5].copy_from_slice(&[value, 0xff, 0, b'a', b'b']);
    captured.text.len = 5;
    captured.text.required = 5;
    captured
}

#[cfg(test)]
mod conversion_tests {
    use super::*;

    fn input(
        captured: &Captured,
        cached: Option<&Arc<TableRows>>,
        stats: &mut LexicalConversionStats,
    ) -> TableInput {
        let [result] = captured.table_inputs(Some(&mut LexicalConversion::new(cached, stats)));
        result
    }

    fn cached(captured: &Captured) -> Arc<TableRows> {
        let [TableView::Complete(rows)] = captured.tables() else {
            panic!()
        };
        Arc::new(rows)
    }

    fn canonical(captured: &Captured, input: &TableInput) {
        let [expected] = captured.tables();
        match input {
            TableInput::SharedComplete(rows) => {
                assert_eq!(TableView::Complete((**rows).clone()), expected)
            }
            TableInput::Sampled(actual) => assert_eq!(actual, &expected),
            TableInput::ComparedEqual(_)
            | TableInput::ComparedChanges { .. }
            | TableInput::SliceUpdate { .. }
            | TableInput::UniformOptionalI32 { .. } => {
                panic!("lexical reuse must retain full encoder validation")
            }
        }
    }

    #[test]
    fn lexical_reuse_compares_every_token_byte_and_returns_the_actual_cache_allocation() {
        let mut current = lexical_reuse_fixture(9);
        let previous = cached(&current);
        let mut stats = LexicalConversionStats::default();
        let TableInput::SharedComplete(rows) = input(&current, Some(&previous), &mut stats) else {
            panic!()
        };
        assert!(Arc::ptr_eq(&rows, &previous));
        current.rows.rows[3].token = u32::MAX;
        current.text.first[15] = 42;
        let replacement = Arc::new((*previous).clone());
        let TableInput::SharedComplete(rows) = input(&current, Some(&replacement), &mut stats)
        else {
            panic!()
        };
        assert!(Arc::ptr_eq(&rows, &replacement));
        assert!(!Arc::ptr_eq(&rows, &previous));
        assert_eq!((stats.hits, stats.misses), (2, 0));
        for byte in 0..5 {
            let mut changed = lexical_reuse_fixture(9);
            changed.text.first[byte] ^= 1;
            let result = input(&changed, Some(&previous), &mut stats);
            canonical(&changed, &result);
            assert!(matches!(result, TableInput::Sampled(_)));
        }
        for token in 0..3 {
            let mut changed = lexical_reuse_fixture(9);
            changed.rows.rows[token].token += 20;
            let result = input(&changed, Some(&previous), &mut stats);
            canonical(&changed, &result);
            assert!(matches!(result, TableInput::Sampled(_)));
        }
        assert_eq!((stats.hits, stats.misses), (2, 8));
    }

    #[test]
    fn lexical_reuse_current_outcomes_counts_and_spans_cannot_be_hidden() {
        let previous = cached(&lexical_reuse_fixture(9));
        let changes: &[fn(&mut Captured)] = &[
            |c| c.rows.outcome = Outcome::Rows { len: 2 },
            |c| c.rows.outcome = Outcome::Rows { len: usize::MAX },
            |c| {
                c.rows.outcome = Outcome::Truncated {
                    len: 3,
                    unread_rows: 1,
                }
            },
            |c| {
                c.rows.outcome = Outcome::Truncated {
                    len: 3,
                    unread_rows: 0,
                }
            },
            |c| {
                c.rows.outcome = Outcome::Unavailable {
                    reason: "current lexical witness failed",
                }
            },
            |c| c.rows.rows[1].token = 0,
            |c| c.rows.rows.swap(1, 2),
            |c| c.rows.rows[0].text_len = 1,
            |c| c.rows.rows[1].text_start = 1,
            |c| c.rows.rows[1].text_len = 2,
            |c| c.rows.rows[2].text_start = usize::MAX,
            |c| c.rows.rows[2].text_len = usize::MAX,
            |c| c.rows.rows[2].text_start = 5,
            |c| c.text.len = usize::MAX,
            |c| c.text.len = 4,
            |c| c.text.required = 6,
        ];
        for change in changes {
            let mut current = lexical_reuse_fixture(9);
            change(&mut current);
            let mut stats = LexicalConversionStats::default();
            let result = input(&current, Some(&previous), &mut stats);
            canonical(&current, &result);
            assert!(matches!(result, TableInput::Sampled(_)));
            assert_eq!((stats.hits, stats.misses), (0, 1));
        }
        let mut current = lexical_reuse_fixture(9);
        current.rows.outcome = Outcome::Unavailable {
            reason: "native failure before bad arena",
        };
        current.text.len = usize::MAX;
        let [view] = current.tables();
        assert!(
            matches!(view, TableView::Unavailable { reason } if reason == "native failure before bad arena")
        );
        current.rows.outcome = Outcome::Rows { len: 3 };
        let [view] = current.tables();
        assert!(
            matches!(view, TableView::Unavailable { reason } if reason == "lexical text lies outside captured arena")
        );
    }

    #[test]
    fn lexical_reuse_missing_wrong_cache_empty_dictionary_and_skips_preserve_semantics() {
        let mut current = lexical_reuse_fixture(9);
        let mut stats = LexicalConversionStats::default();
        let wrong = Arc::new(TableRows::Characters(Vec::new()));
        for previous in [None, Some(&wrong)] {
            let result = input(&current, previous, &mut stats);
            canonical(&current, &result);
            assert!(matches!(
                result,
                TableInput::Sampled(TableView::Complete(_))
            ));
        }
        current.rows.outcome = Outcome::Rows { len: 0 };
        current.text.len = 0;
        current.text.required = 0;
        let previous = cached(&current);
        let TableInput::SharedComplete(rows) = input(&current, Some(&previous), &mut stats) else {
            panic!()
        };
        assert!(Arc::ptr_eq(&rows, &previous));
        current.rows.outcome = Outcome::Unavailable {
            reason: replay_data::cadence::SKIPPED_REASON,
        };
        let before = stats;
        let result = input(&current, Some(&previous), &mut stats);
        canonical(&current, &result);
        assert_eq!(stats, before);
    }
}
