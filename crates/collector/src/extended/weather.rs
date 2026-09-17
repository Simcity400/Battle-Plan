//! Native weather acquisition. Only growth and replay conversion allocate.
//!
//! Hourly weather raw-copy split (WeatherLimits, WeatherRegions,
//! WeatherProvinces): the game thread copies raw bytes only and the single
//! FIFO decode worker runs the native row parsers over owned exact-field
//! gathers. Root topology, row order, truncation and field-failure prefixes
//! survive without retaining unused dense-array padding or rendering bytes.

use crate::{raw::RawLog, sampling::qpc, spooling::table, table_capture::CaptureRows};
use engine::{
    Memory, cweather_manager,
    memory::{RecordLayout, RecordSources},
    sweather_per_province, sweather_per_region,
    walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};

pub const N: usize = 3;
pub const TABLES: [TableId; N] = [
    TableId::WeatherLimits,
    TableId::WeatherRegions,
    TableId::WeatherProvinces,
];

/// Manager authentication is charged once to limits for each acquisition pass.
/// The two dense arrays retain their own read/comparison times and outcomes.
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("weather manager and dense tables", 0, &[0, 1, 2])];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            // Keep the pinned map below CaptureRows' 75% growth threshold.
            rows: [2, 512, 20_000],
        }
    }
}

pub struct Captured {
    limits: CaptureRows<cweather_manager::Row>,
    regions: CaptureRows<sweather_per_region::Row>,
    provinces: CaptureRows<sweather_per_province::Row>,
    /// Raw-copy split gate for the hourly weather family (copy/decode split).
    /// Set by the game thread every hour with a valid raw payload; `capture`
    /// then skips the typed walks and the decode worker fills these rows
    /// before the ready handoff. Preserved across `reset` (set per hour,
    /// cleared by the worker after decode).
    pub(crate) raw_mode: bool,
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        Self {
            limits: CaptureRows::new(capacity.rows[0], successful[0]),
            regions: CaptureRows::new(capacity.rows[1], successful[1]),
            provinces: CaptureRows::new(capacity.rows[2], successful[2]),
            raw_mode: false,
        }
    }

    pub fn reset(&mut self) {
        self.limits.reset();
        self.regions.reset();
        self.provinces.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.limits.allocated_bytes()
            + self.regions.allocated_bytes()
            + self.provinces.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        self.limits.grow(&mut capacity.rows[0], successful[0]);
        self.regions.grow(&mut capacity.rows[1], successful[1]);
        self.provinces.grow(&mut capacity.rows[2], successful[2]);
    }

    /// The manager is resolved afresh for the second pass. Every primary walk
    /// still runs after its own consistency window completes. No allocation,
    /// encoding or I/O occurs here; returned times are raw QPC ticks.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        self.reset();
        // Raw-copy split (hourly weather family): every hour the game thread
        // stages raw bytes via `copy_weather_refresh` and the decode worker
        // fills the typed rows before the ready handoff, so both typed passes
        // skip here. Matched flags and costs come from the raw compare,
        // applied by the caller after this returns.
        if self.raw_mode {
            // Refresh payload staged: leave the reset outcomes for the decode
            // worker, which installs identical rows and outcomes.
            return ([false; N], [0; N]);
        }
        let mut matched = [false; N];
        let mut ticks = [0; N];
        let begin = qpc();
        let first = cweather_manager::Reader::new(memory, base, game_state);
        ticks[0] = elapsed(begin);
        let first = match first {
            Ok(reader) => reader,
            Err(reason) => {
                self.limits.outcome = Outcome::Unavailable { reason };
                self.regions.outcome = Outcome::Unavailable { reason };
                self.provinces.outcome = Outcome::Unavailable { reason };
                return (matched, ticks);
            }
        };
        macro_rules! primary {
            ($index:literal, $field:ident) => {{
                let begin = qpc();
                self.$field.outcome =
                    bounded(first.$field(&mut self.$field.rows), self.$field.rows.len());
                ticks[$index] = ticks[$index].saturating_add(elapsed(begin));
            }};
        }
        primary!(0, limits);
        primary!(1, regions);
        primary!(2, provinces);

        if needed[..N].iter().any(|needed| *needed) {
            let begin = qpc();
            let second = cweather_manager::Reader::new(memory, base, game_state);
            ticks[0] = ticks[0].saturating_add(elapsed(begin));
            macro_rules! check {
                ($index:literal, $field:ident) => {
                    if needed[$index] {
                        let begin = qpc();
                        self.$field.checked = true;
                        let outcome = match &second {
                            Ok(reader) => bounded(
                                reader.$field(&mut self.$field.second),
                                self.$field.second.len(),
                            ),
                            Err(reason) => Outcome::Unavailable { reason },
                        };
                        matched[$index] = self.$field.compare(outcome);
                        ticks[$index] = ticks[$index].saturating_add(elapsed(begin));
                    }
                };
            }
            check!(0, limits);
            check!(1, regions);
            check!(2, provinces);
        }
        (matched, ticks)
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            self.limits.mismatches,
            self.regions.mismatches,
            self.provinces.mismatches,
        ]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            effective(&self.limits),
            effective(&self.regions),
            effective(&self.provinces),
        ]
    }

    /// Spool-thread conversion; raw values and incomplete-table status survive.
    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        [
            table::<tables::WeatherLimits, _>(outcomes[0], &self.limits.rows, |r| {
                rows::WeatherLimitsRow {
                    max_water: r.max_water,
                    max_snow: r.max_snow,
                }
            }),
            table::<tables::WeatherRegions, _>(outcomes[1], &self.regions.rows, |r| {
                rows::WeatherRegionRow {
                    id: r.id,
                    temperature: r.temperature,
                    next_change_hour: r.next_change_hour,
                    light_rain: r.light_rain,
                    heavy_rain: r.heavy_rain,
                    snow: r.snow,
                    blizzard: r.blizzard,
                    sandstorm: r.sandstorm,
                    arctic_water: r.arctic_water,
                }
            }),
            table::<tables::WeatherProvinces, _>(outcomes[2], &self.provinces.rows, |r| {
                rows::WeatherProvinceRow {
                    id: r.id,
                    water: r.water,
                    snow: r.snow,
                    temperature: r.temperature,
                    temperature_change: r.temperature_change,
                    mud: r.mud,
                    custom_mud: r.custom_mud,
                    custom_modifier_count: r.custom_modifier_count,
                }
            }),
        ]
    }
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
            reason: "table changed during double read",
        }
    } else {
        captured.outcome
    }
}

// ===========================================================================
// Hourly weather raw-copy split (WeatherLimits, WeatherRegions,
// WeatherProvinces).
//
// The game thread copies raw bytes only: manager pointer, one bulk manager
// header (vtable, both ceilings, both array headers), then dense exact fields
// in bounded gathers. No row is decoded on the game thread. Only manager
// reads use Recorded containment; packed arrays never pretend to be complete
// native address ranges and cannot accidentally serve cross-field reads.
//
// The mirror traverses structurally and records every discoverable span,
// never aborting except on overflow: validation, precedence and error scope
// are local parses, so the replay derives every outcome from the same
// recorded spans and later extras are harmless.
// ===========================================================================

/// Fault-attribution bits in family order: [limits, regions, provinces].
/// The manager header carries all three bits; array payloads carry one bit.
const M_W_ALL: u32 = 0x07;

/// One hour of raw weather payload. Flat pools recycle capacities and the
/// steady state allocates on neither thread; growth happens on the decode
/// side only (see `crate::decode::recycle_weather_job`).
#[derive(Debug, Default)]
pub(crate) struct WeatherJob {
    pub log: RawLog<3>,
    pub game_state: usize,
    regions: DensePayload,
    provinces: DensePayload,
    capacity_charge: crate::raw_memory::Charge,
    staging_failure: Option<&'static str>,
}

impl WeatherJob {
    pub fn new() -> Self {
        let mut job = Self {
            // Only the shared root and manager remain address-indexed.
            log: RawLog::new(16, 1024),
            game_state: 0,
            regions: DensePayload::new(512 * sweather_per_region::RECORD_BYTES),
            provinces: DensePayload::new(20_000 * sweather_per_province::RECORD_BYTES),
            capacity_charge: Default::default(),
            staging_failure: None,
        };
        job.update_capacity_charge();
        job
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.regions.clear();
        self.provinces.clear();
        self.staging_failure = None;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next() || self.regions.needs_growth() || self.provinces.needs_growth()
    }

    /// Double every pool off the game thread until copies fit. Converges like
    /// the typed capacity growth; overflowed hours fall back to the typed
    /// walk in the meantime, so sizing is a pure performance knob.
    pub fn grow(&mut self) {
        self.log.grow();
        self.regions.grow();
        self.provinces.grow();
        self.update_capacity_charge();
    }

    fn update_capacity_charge(&mut self) {
        self.capacity_charge.set(crate::raw_memory::Capacity {
            payload_vectors: crate::raw_memory::vector_bytes(&self.regions.bytes)
                + crate::raw_memory::vector_bytes(&self.provinces.bytes),
            ..Default::default()
        });
    }

    fn partition_equal(&self, other: &Self, slot: usize) -> bool {
        self.log.partition_equal(&other.log, slot)
            && match slot {
                1 => self.regions.equal(&other.regions),
                2 => self.provinces.equal(&other.provinces),
                _ => true,
            }
    }
}

/// Exact declared fields only, not an address-space substitute. Skipped native
/// padding, rendering bytes and modifier contents no longer participate in raw
/// equality; every recorded field, identity and full custom-array header does.
/// The first failed field is retained as a prefix, never retried or zero-filled.
#[derive(Debug, Default)]
struct DensePayload {
    array: Option<engine::cpdx_array::Array>,
    bytes: Vec<u8>,
    required: usize,
    copied_fields: usize,
    copied_bytes: usize,
}

impl DensePayload {
    fn new(capacity: usize) -> Self {
        Self {
            bytes: vec![0; capacity],
            ..Default::default()
        }
    }
    fn clear(&mut self) {
        self.array = None;
        self.required = 0;
        self.copied_fields = 0;
        self.copied_bytes = 0;
    }
    fn needs_growth(&self) -> bool {
        self.required > self.bytes.len()
    }
    fn prepare(&mut self, array: Option<engine::cpdx_array::Array>, record_bytes: usize) {
        self.array = array;
        self.required = array.map_or(0, |array| {
            (array.count as usize).saturating_mul(record_bytes)
        });
    }
    fn grow(&mut self) {
        if self.needs_growth() {
            self.bytes
                .resize(self.required.max(self.bytes.len().saturating_mul(2)), 0);
        }
    }
    fn equal(&self, other: &Self) -> bool {
        self.array == other.array
            && self.copied_fields == other.copied_fields
            && self.copied_bytes == other.copied_bytes
            && self.bytes.get(..self.copied_bytes) == other.bytes.get(..other.copied_bytes)
    }
    fn capture(
        &mut self,
        memory: &impl Memory,
        array: Option<engine::cpdx_array::Array>,
        stride: usize,
        layout: &RecordLayout<'_>,
    ) -> bool {
        let record_bytes = layout.stride();
        self.prepare(array, record_bytes);
        let Some(array) = array else {
            return true;
        };
        if self.needs_growth() {
            return false;
        }
        let mut first = 0;
        while first < array.count as usize {
            let count = (array.count as usize - first).min(64);
            // Array validated the complete native extent before this loop.
            let sources = RecordSources::Strided {
                base: array.element(first as u32, stride).unwrap_or(0),
                stride,
                count,
            };
            let begin = first * record_bytes;
            let completed = memory.copy_records(
                sources,
                layout,
                &mut self.bytes[begin..begin + count * record_bytes],
            );
            debug_assert!(completed <= count * layout.field_count());
            self.copied_fields += completed;
            self.copied_bytes += layout.completed_bytes(completed);
            if completed != count * layout.field_count() {
                break;
            }
            first += count;
        }
        true
    }
    fn faulted(&self, fields: usize) -> bool {
        self.array
            .is_some_and(|array| self.copied_fields < array.count as usize * fields)
    }
    #[allow(clippy::too_many_arguments)]
    fn decode<R>(
        &self,
        base: usize,
        vtable_rva: usize,
        fields: usize,
        record_bytes: usize,
        output: &mut [R],
        invalid_array: &'static str,
        decode: impl Fn(u32, &[u8], usize, usize) -> Result<R, &'static str>,
    ) -> Outcome {
        let Some(array) = self.array else {
            return Outcome::Unavailable {
                reason: invalid_array,
            };
        };
        let Some(vtable) = base.checked_add(vtable_rva) else {
            return Outcome::Unavailable {
                reason: "module address overflow",
            };
        };
        let count = (array.count as usize).min(output.len());
        for (index, row) in output[..count].iter_mut().enumerate() {
            let Some(record) = self
                .bytes
                .get(index * record_bytes..(index + 1) * record_bytes)
            else {
                return Outcome::Unavailable {
                    reason: "weather packed record exceeds allocated buffer",
                };
            };
            let copied = self
                .copied_fields
                .saturating_sub(index * fields)
                .min(fields);
            match decode(index as u32, record, copied, vtable) {
                Ok(value) => *row = value,
                Err(reason) => return Outcome::Unavailable { reason },
            }
        }
        let unread_rows = array.count - count as u32;
        if unread_rows == 0 {
            Outcome::Rows { len: count }
        } else {
            Outcome::Truncated {
                len: count,
                unread_rows,
            }
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct WeatherCopyReport {
    /// False when preflight lacks capacity and encountered no source failure:
    /// the caller can safely run the typed walk. A source-fault/capacity pair
    /// instead retains an explicit terminal-unavailable payload without retry.
    pub raw_ok: bool,
    /// Per-table raw equality gated by live-fault conservatism, in family
    /// order [limits, regions, provinces].
    pub matched: [bool; 3],
    pub copy_ticks: [u64; 3],
    pub raw_bytes: [u64; 3],
}

/// Game-thread copy parameters, bundled to keep the tick entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WeatherCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; 3],
}

fn w_pool_u64(log: &RawLog<3>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn wspan(
    log: &mut RawLog<3>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Bulk variant for contiguous manager/array copies. Enclosed sub-reads
/// serve from the bulk span at replay.
fn wsbulk(
    log: &mut RawLog<3>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take_bulk(memory, addr?, len, mask)
}

/// Acquire each pass's manager topology once before any dense fields. The
/// caller preflights both tables in BOTH passes before starting dense reads.
fn prepare_weather_pass(memory: &impl Memory, game_state: usize, job: &mut WeatherJob) {
    use engine::{cgame_state, cpdx_array::Array, cweather_manager};
    let log = &mut job.log;
    log.has_bulk = true;
    let Some(ptr_start) = wspan(
        log,
        memory,
        game_state.checked_add(cgame_state::WEATHER_MANAGER),
        8,
        M_W_ALL,
    ) else {
        return;
    };
    let pointer = match usize::try_from(w_pool_u64(log, ptr_start))
        .ok()
        .filter(|&p| p != 0)
    {
        Some(p) => p,
        None => return,
    };
    // Bulk manager header encloses vtable, both ceilings and both array
    // headers (0x00..0x200 covers 0x10/0x40/0x1a8/0x1f8).
    let Some(header_start) = wsbulk(log, memory, Some(pointer), 0x200, M_W_ALL) else {
        return;
    };
    let mut head = [0u8; 0x200];
    head.copy_from_slice(&log.bytes[header_start..header_start + 0x200]);
    let header = |offset: usize| -> [u8; 16] { head[offset..offset + 16].try_into().unwrap() };
    let regions = Array::from_header(
        header(cweather_manager::REGIONS),
        sweather_per_region::STRIDE,
    );
    let provinces = Array::from_header(
        header(cweather_manager::PROVINCES),
        sweather_per_province::STRIDE,
    );
    job.regions
        .prepare(regions, sweather_per_region::RECORD_BYTES);
    job.provinces
        .prepare(provinces, sweather_per_province::RECORD_BYTES);
}

fn preflight_weather(job: &mut WeatherJob) {
    job.log.overflowed |= job.regions.needs_growth() || job.provinces.needs_growth();
}

fn copy_weather_dense(memory: &impl Memory, job: &mut WeatherJob) {
    const REGIONS_LAYOUT: RecordLayout<'static> = RecordLayout::new(
        &sweather_per_region::FIELDS,
        sweather_per_region::RECORD_BYTES,
    )
    .expect("valid record layout");
    const PROVINCES_LAYOUT: RecordLayout<'static> = RecordLayout::new(
        &sweather_per_province::FIELDS,
        sweather_per_province::RECORD_BYTES,
    )
    .expect("valid record layout");
    debug_assert!(!job.log.overflowed);
    let log = &mut job.log;
    // A bad array header is table-local. Capacity was checked for every
    // table/pass before these reads, so a later shortage cannot cause a typed
    // retry after an earlier read consumed a PAGE_GUARD or otherwise failed.
    log.overflowed |= !job.regions.capture(
        memory,
        job.regions.array,
        sweather_per_region::STRIDE,
        &REGIONS_LAYOUT,
    );
    log.overflowed |= !job.provinces.capture(
        memory,
        job.provinces.array,
        sweather_per_province::STRIDE,
        &PROVINCES_LAYOUT,
    );
    log.raw_bytes[1] = job.regions.copied_bytes as u64;
    log.raw_bytes[2] = job.provinces.copied_bytes as u64;
    log.fault[1] |= job.regions.faulted(sweather_per_region::FIELDS.len());
    log.fault[2] |= job.provinces.faulted(sweather_per_province::FIELDS.len());
}

#[cfg(test)]
fn copy_weather_pass(memory: &impl Memory, _base: usize, game_state: usize, job: &mut WeatherJob) {
    prepare_weather_pass(memory, game_state, job);
    preflight_weather(job);
    if !job.log.overflowed {
        copy_weather_dense(memory, job);
    }
}

/// Acquire a refresh into owned jobs and compare independent raw passes.
/// The caller applies the report after acquisition has completed.
pub(crate) fn copy_weather_refresh(
    memory: &impl Memory,
    ctx: WeatherCopyContext,
    first: &mut WeatherJob,
    mut second: Option<&mut WeatherJob>,
) -> WeatherCopyReport {
    let _ = ctx.base;
    let started = qpc();
    first.clear();
    first.game_state = ctx.game_state;
    prepare_weather_pass(memory, ctx.game_state, first);
    if let Some(second_job) = second.as_deref_mut() {
        second_job.clear();
        second_job.game_state = ctx.game_state;
        prepare_weather_pass(memory, ctx.game_state, second_job);
    }
    preflight_weather(first);
    if let Some(second_job) = second.as_deref_mut() {
        preflight_weather(second_job);
    }
    let overflow = first.log.overflowed || second.as_ref().is_some_and(|job| job.log.overflowed);
    let root_fault = first.log.fault.iter().any(|&fault| fault)
        || second
            .as_ref()
            .is_some_and(|job| job.log.fault.iter().any(|&fault| fault));
    // If even preflight encountered a real source failure, falling back
    // would reread that failed source. Keep a terminal unavailable payload
    // instead; its capacity still grows when recycled off the callback.
    if overflow && root_fault {
        first.staging_failure = Some("weather staging unavailable after source read failure");
    }
    if !overflow {
        copy_weather_dense(memory, first);
        if let Some(second_job) = second.as_deref_mut() {
            copy_weather_dense(memory, second_job);
        }
    }
    let mut ticks = [0u64; 3];
    ticks[0] = elapsed(started);
    let mut matched = [true; 3];
    if let Some(second_job) = second {
        let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
        for (slot, matched) in matched.iter_mut().enumerate() {
            let equal = raw_ok_pair && first.partition_equal(second_job, slot);
            *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
        }
    }
    if first.staging_failure.is_some() {
        matched = [false; N];
    }
    let raw_ok = !overflow || first.staging_failure.is_some();
    WeatherCopyReport {
        raw_ok,
        matched,
        copy_ticks: ticks,
        raw_bytes: first.log.raw_bytes,
    }
}

impl Captured {
    /// Apply completed acquisition checks without changing rows or outcomes.
    pub(crate) fn apply_weather_report(
        &mut self,
        ctx: WeatherCopyContext,
        report: &WeatherCopyReport,
    ) {
        self.limits.checked = ctx.needed[0];
        self.limits.mismatches = usize::from(ctx.needed[0] && !report.matched[0]);
        self.regions.checked = ctx.needed[1];
        self.regions.mismatches = usize::from(ctx.needed[1] && !report.matched[1]);
        self.provinces.checked = ctx.needed[2];
        self.provinces.mismatches = usize::from(ctx.needed[2] && !report.matched[2]);
    }

    /// Mark an accepted acquisition failure without retrying live memory.
    pub(crate) fn fail_weather_capture(&mut self, ctx: WeatherCopyContext, reason: &'static str) {
        self.limits.outcome = Outcome::Unavailable { reason };
        self.limits.checked = ctx.needed[0];
        self.limits.mismatches = usize::from(ctx.needed[0]);
        self.regions.outcome = Outcome::Unavailable { reason };
        self.regions.checked = ctx.needed[1];
        self.regions.mismatches = usize::from(ctx.needed[1]);
        self.provinces.outcome = Outcome::Unavailable { reason };
        self.provinces.checked = ctx.needed[2];
        self.provinces.mismatches = usize::from(ctx.needed[2]);
        self.raw_mode = false;
    }

    /// Decode-worker interpretation of a raw hourly payload into the pooled
    /// typed rows. Outcomes match the typed walk exactly because the
    /// identical row parser consumes the exact-field prefix; checked/mismatches
    /// set by the raw compare survive. Clears `raw_mode` so the next capture
    /// starts clean.
    pub(crate) fn decode_weather_payload(
        &mut self,
        base: usize,
        job: &WeatherJob,
        decode_ticks: &mut [u64; 3],
    ) {
        if let Some(reason) = job.staging_failure {
            self.limits.outcome = Outcome::Unavailable { reason };
            self.regions.outcome = Outcome::Unavailable { reason };
            self.provinces.outcome = Outcome::Unavailable { reason };
            self.raw_mode = false;
            return;
        }
        let recorded = crate::raw::Recorded::build(&job.log);
        let started = qpc();
        let reader = cweather_manager::Reader::new(&recorded, base, job.game_state);
        let reader = match reader {
            Ok(reader) => reader,
            Err(reason) => {
                let unavailable = Outcome::Unavailable { reason };
                self.limits.outcome = unavailable;
                self.regions.outcome = unavailable;
                self.provinces.outcome = unavailable;
                decode_ticks[0] += elapsed(started);
                self.raw_mode = false;
                return;
            }
        };
        self.limits.outcome = bounded(reader.limits(&mut self.limits.rows), self.limits.rows.len());
        decode_ticks[0] += elapsed(started);
        let started = qpc();
        self.regions.outcome = job.regions.decode(
            base,
            sweather_per_region::VTABLE_RVA,
            sweather_per_region::FIELDS.len(),
            sweather_per_region::RECORD_BYTES,
            &mut self.regions.rows,
            "weather region array unreadable or invalid",
            sweather_per_region::decode_record,
        );
        decode_ticks[1] += elapsed(started);
        let started = qpc();
        self.provinces.outcome = job.provinces.decode(
            base,
            sweather_per_province::VTABLE_RVA,
            sweather_per_province::FIELDS.len(),
            sweather_per_province::RECORD_BYTES,
            &mut self.provinces.rows,
            "weather province array unreadable or invalid",
            sweather_per_province::decode_record,
        );
        decode_ticks[2] += elapsed(started);
        self.raw_mode = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
    use engine::{cgame_state, memory::SliceMemory};
    use replay::rows::TableSchema;
    use std::cell::Cell;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - 0x1000..address - 0x1000 + value.len()].copy_from_slice(value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x4000];
        put(
            &mut bytes,
            0x1000 + cgame_state::WEATHER_MANAGER,
            &0x2000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x2000,
            &(cweather_manager::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x2000 + cweather_manager::MAX_WATER,
            &i64::MAX.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x2000 + cweather_manager::MAX_SNOW,
            &(-1_i64).to_le_bytes(),
        );
        for (offset, address, vtable) in [
            (
                cweather_manager::REGIONS,
                0x3000_u64,
                sweather_per_region::VTABLE_RVA,
            ),
            (
                cweather_manager::PROVINCES,
                0x4000_u64,
                sweather_per_province::VTABLE_RVA,
            ),
        ] {
            put(&mut bytes, 0x2000 + offset, &address.to_le_bytes());
            put(&mut bytes, 0x2000 + offset + 8, &3_u32.to_le_bytes());
            put(&mut bytes, 0x2000 + offset + 12, &1_u32.to_le_bytes());
            put(&mut bytes, address as usize, &(vtable as u64).to_le_bytes());
        }
        put(
            &mut bytes,
            0x3000 + sweather_per_region::TEMPERATURE,
            &i64::MIN.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x3000 + sweather_per_region::NEXT_CHANGE_HOUR,
            &i32::MIN.to_le_bytes(),
        );
        put(&mut bytes, 0x3000 + sweather_per_region::SNOW, &[255]);
        put(
            &mut bytes,
            0x4000 + sweather_per_province::WATER,
            &((1_i64 << 40) + 3).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x4000 + sweather_per_province::TEMPERATURE_CHANGE,
            &(-(1_i64 << 41) - 7).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x4000 + sweather_per_province::CUSTOM_MUD,
            &[254],
        );
        bytes
    }

    fn small() -> (Capacities, Captured) {
        let capacities = Capacities { rows: [2; N] };
        let captured = Captured::with_capacities(&capacities, &[0; N]);
        (capacities, captured)
    }

    fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
        let TableView::Complete(rows) = view else {
            panic!("expected complete table")
        };
        rows.rows::<S>().unwrap()
    }

    #[test]
    fn acquisition_and_conversion_keep_dense_zero_native_words_and_raw_flags() {
        let (_, mut captured) = small();
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let (matched, _) = captured.capture(&memory, 0, 0x1000, &[true; N]);
        assert_eq!(matched, [true; N]);
        assert_eq!(captured.checked_mismatches(), [0; N]);
        let tables = captured.tables();
        assert_eq!(
            complete::<tables::WeatherLimits>(&tables[0]),
            &[rows::WeatherLimitsRow {
                max_water: i64::MAX,
                max_snow: -1
            }]
        );
        let region = &complete::<tables::WeatherRegions>(&tables[1])[0];
        assert_eq!(
            (
                region.id,
                region.temperature,
                region.next_change_hour,
                region.snow
            ),
            (0, i64::MIN, i32::MIN, 255)
        );
        let province = &complete::<tables::WeatherProvinces>(&tables[2])[0];
        assert_eq!(
            (
                province.id,
                province.temperature,
                province.water,
                province.temperature_change,
                province.custom_mud
            ),
            (0, 0, (1_i64 << 40) + 3, -(1_i64 << 41) - 7, 254)
        );
    }

    #[test]
    fn table_faults_and_truncation_remain_independent_and_do_not_earn_checks() {
        let (mut capacities, _) = small();
        capacities.rows[2] = 0;
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let mut bytes = fixture();
        put(
            &mut bytes,
            0x2000 + cweather_manager::REGIONS_COUNT,
            &(-1_i32).to_le_bytes(),
        );
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let (matched, _) = captured.capture(&memory, 0, 0x1000, &[true; N]);
        assert_eq!(matched, [true, false, false]);
        assert!(matches!(
            captured.outcomes()[1],
            Outcome::Unavailable { .. }
        ));
        assert_eq!(
            captured.outcomes()[2],
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        let tables = captured.tables();
        assert!(matches!(tables[1], TableView::Unavailable { .. }));
        assert!(matches!(tables[2], TableView::Truncated { missing: 1, .. }));
        captured.grow(&mut capacities, &[0; N]);
        assert!(!captured.provinces.rows.is_empty());
        put(
            &mut bytes,
            0x1000 + cgame_state::WEATHER_MANAGER,
            &0_u64.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            captured.capture(&memory, 0, 0x1000, &[true; N]).0,
            [false; N]
        );
        assert!(
            captured
                .outcomes()
                .into_iter()
                .all(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
        );
    }

    struct ChangingWater<'a> {
        bytes: &'a [u8],
        copies: Cell<usize>,
    }
    impl Memory for ChangingWater<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if !(SliceMemory {
                base: 0x1000,
                bytes: self.bytes,
            })
            .copy(address, output)
            {
                return false;
            }
            if address == 0x4000 + sweather_per_province::WATER {
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
    fn changed_table_is_unavailable_and_finished_windows_release_independently() {
        let (mut capacities, mut captured) = small();
        let bytes = fixture();
        let memory = ChangingWater {
            bytes: &bytes,
            copies: Cell::new(0),
        };
        assert_eq!(
            captured.capture(&memory, 0, 0x1000, &[true; N]).0,
            [true, true, false]
        );
        assert_eq!(captured.checked_mismatches(), [0, 0, 1]);
        assert!(matches!(
            captured.tables()[2],
            TableView::Unavailable { .. }
        ));
        let before = captured.allocated_bytes();
        captured.grow(
            &mut capacities,
            &[CONSISTENCY_TARGET, 0, CONSISTENCY_TARGET],
        );
        assert!(captured.limits.second.is_empty() && captured.provinces.second.is_empty());
        assert!(!captured.regions.second.is_empty());
        assert!(captured.allocated_bytes() < before);
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            captured
                .capture(&memory, 0, 0x1000, &[false, true, false])
                .0,
            [false, true, false]
        );
        assert_eq!(captured.checked_mismatches(), [0; N]);
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 1 }; N]);
    }

    fn split_capacities() -> Capacities {
        Capacities { rows: [2, 8, 8] }
    }

    fn direct(memory: &SliceMemory<'_>, capacities: &Capacities) -> Captured {
        let mut captured = Captured::with_capacities(capacities, &[0; N]);
        let (matched, _) = captured.capture(memory, 0, 0x1000, &[true; N]);
        assert_eq!(matched, [true; N]);
        captured
    }

    fn split(memory: &SliceMemory<'_>, capacities: &Capacities) -> (Captured, WeatherCopyReport) {
        let mut captured = Captured::with_capacities(capacities, &[0; N]);
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
            needed: [true; N],
        };
        let mut first = WeatherJob::new();
        let mut second = WeatherJob::new();
        let report = copy_weather_refresh(memory, ctx, &mut first, Some(&mut second));
        captured.apply_weather_report(ctx, &report);
        assert!(report.raw_ok);
        captured.raw_mode = report.raw_ok;
        let mut ticks = [0u64; 3];
        captured.decode_weather_payload(0, &first, &mut ticks);
        (captured, report)
    }

    fn assert_matches_direct(split: &Captured, direct: &Captured) {
        assert_eq!(
            split.limits.outcome, direct.limits.outcome,
            "limits outcome"
        );
        assert_eq!(
            split.regions.outcome, direct.regions.outcome,
            "regions outcome"
        );
        assert_eq!(
            split.provinces.outcome, direct.provinces.outcome,
            "provinces outcome"
        );
        let len = split.limits.outcome.len();
        assert_eq!(
            &split.limits.rows[..len],
            &direct.limits.rows[..len],
            "limits rows"
        );
        let len = split.regions.outcome.len();
        assert_eq!(
            &split.regions.rows[..len],
            &direct.regions.rows[..len],
            "regions rows"
        );
        let len = split.provinces.outcome.len();
        assert_eq!(
            &split.provinces.rows[..len],
            &direct.provinces.rows[..len],
            "provinces rows"
        );
    }

    struct FaultReads<'a> {
        memory: SliceMemory<'a>,
        fault: usize,
        failures: Cell<usize>,
    }
    impl Memory for FaultReads<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self.fault >= address && self.fault < address.saturating_add(output.len()) {
                self.failures.set(self.failures.get() + 1);
                // A fault is allowed to modify the incomplete field; neither
                // the owned payload nor its parser may publish these bytes.
                output.fill(0xab);
                return false;
            }
            self.memory.copy(address, output)
        }
    }

    fn assert_packed_matches(memory: &impl Memory, capacities: &Capacities) -> WeatherJob {
        let mut direct = Captured::with_capacities(capacities, &[0; N]);
        direct.capture(memory, 0, 0x1000, &[false; N]);
        let mut packed = Captured::with_capacities(capacities, &[0; N]);
        let mut job = WeatherJob::new();
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
            needed: [false; N],
        };
        let report = copy_weather_refresh(memory, ctx, &mut job, None);
        packed.apply_weather_report(ctx, &report);
        assert!(report.raw_ok);
        packed.decode_weather_payload(0, &job, &mut [0; N]);
        assert_matches_direct(&packed, &direct);
        // Even on Unavailable, already admitted rows and unused suffixes
        // retain the same prefix convention as the original native walk.
        assert_eq!(packed.regions.rows, direct.regions.rows);
        assert_eq!(packed.provinces.rows, direct.provinces.rows);
        job
    }

    #[test]
    fn packed_weather_field_faults_preserve_prefixes_without_failed_batch_retry() {
        let mut bytes = vec![0; 0x50000];
        bytes[..0x4000].copy_from_slice(&fixture());
        for (header, address, stride, vtable) in [
            (
                cweather_manager::REGIONS,
                0x3000,
                sweather_per_region::STRIDE,
                sweather_per_region::VTABLE_RVA,
            ),
            (
                cweather_manager::PROVINCES,
                0x20000,
                sweather_per_province::STRIDE,
                sweather_per_province::VTABLE_RVA,
            ),
        ] {
            put(&mut bytes, 0x2000 + header, &(address as u64).to_le_bytes());
            put(&mut bytes, 0x2000 + header + 8, &130_u32.to_le_bytes());
            put(&mut bytes, 0x2000 + header + 12, &130_u32.to_le_bytes());
            for index in 0..130 {
                let pointer = address + index * stride;
                bytes[pointer - 0x1000..pointer - 0x1000 + stride].fill(0);
                put(&mut bytes, pointer, &(vtable as u64).to_le_bytes());
                put(&mut bytes, pointer + 8, &(index as u32).to_le_bytes());
            }
        }
        for (table, address, stride, fields) in [
            (
                1,
                0x3000,
                sweather_per_region::STRIDE,
                &sweather_per_region::FIELDS[..],
            ),
            (
                2,
                0x20000,
                sweather_per_province::STRIDE,
                &sweather_per_province::FIELDS[..],
            ),
        ] {
            for row in [0, 63, 64, 129] {
                for (field_index, field) in fields.iter().enumerate() {
                    for admitted in [row, 130] {
                        let mut capacities = Capacities {
                            rows: [1, 130, 130],
                        };
                        capacities.rows[table] = admitted;
                        let memory = FaultReads {
                            memory: SliceMemory {
                                base: 0x1000,
                                bytes: &bytes,
                            },
                            fault: address + row * stride + field.source_offset + field.len / 2,
                            failures: Cell::new(0),
                        };
                        let job = assert_packed_matches(&memory, &capacities);
                        assert_eq!(memory.failures.get(), if admitted <= row { 1 } else { 2 });
                        let payload = if table == 1 {
                            &job.regions
                        } else {
                            &job.provinces
                        };
                        assert_eq!(payload.copied_fields, row * fields.len() + field_index);
                        assert!(job.log.fault[table]);
                        assert!(!job.log.fault[3 - table]);
                    }
                }
            }
        }
    }

    #[test]
    fn packed_weather_array_validation_is_independent_and_custom_headers_are_exact() {
        for (header, other) in [
            (cweather_manager::REGIONS, 2),
            (cweather_manager::PROVINCES, 1),
        ] {
            let mut bytes = fixture();
            put(&mut bytes, 0x2000 + header + 12, &4_u32.to_le_bytes());
            let job = assert_packed_matches(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes,
                },
                &Capacities { rows: [1; N] },
            );
            assert!(
                if other == 1 {
                    job.regions.copied_fields
                } else {
                    job.provinces.copied_fields
                } > 0
            );
        }
        for (offset, value) in [
            (sweather_per_province::CUSTOM_MODIFIERS_CAPACITY, u32::MAX),
            (sweather_per_province::CUSTOM_MODIFIERS_COUNT, 1),
            (sweather_per_province::ID, 3),
        ] {
            let mut bytes = fixture();
            put(&mut bytes, 0x4000 + offset, &value.to_le_bytes());
            assert_packed_matches(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes,
                },
                &Capacities { rows: [1; N] },
            );
        }
        // Aliased arrays are not reconstructed into a fake zero-filled native
        // allocation: each parser still sees the real overlapping identity.
        let mut bytes = fixture();
        put(
            &mut bytes,
            0x2000 + cweather_manager::PROVINCES,
            &0x3000_u64.to_le_bytes(),
        );
        assert_packed_matches(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            &Capacities { rows: [1; N] },
        );
    }

    #[test]
    fn packed_weather_omits_unused_bytes_but_keeps_topology_and_every_declared_field() {
        let bytes = fixture();
        let mut first = WeatherJob::new();
        copy_weather_pass(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            0,
            0x1000,
            &mut first,
        );
        assert_eq!(first.log.raw_bytes, [520, 30, 62]);
        assert_eq!(first.log.spans.len(), 2);
        for (pointer, offsets, table) in [
            (0x3000, vec![0x20, 0x58, 0x159], 1),
            (0x4000, vec![0x20, 0x58, 0x160, 0x17f], 2),
        ] {
            for offset in offsets {
                let mut changed = bytes.clone();
                put(&mut changed, pointer + offset, &[255]);
                let mut second = WeatherJob::new();
                copy_weather_pass(
                    &SliceMemory {
                        base: 0x1000,
                        bytes: &changed,
                    },
                    0,
                    0x1000,
                    &mut second,
                );
                assert!(first.partition_equal(&second, table));
            }
        }
        for (pointer, fields, table) in [
            (0x3000, &sweather_per_region::FIELDS[..], 1),
            (0x4000, &sweather_per_province::FIELDS[..], 2),
        ] {
            for field in fields {
                for byte in 0..field.len {
                    let mut changed = bytes.clone();
                    changed[pointer + field.source_offset + byte - 0x1000] ^= 1;
                    let mut second = WeatherJob::new();
                    copy_weather_pass(
                        &SliceMemory {
                            base: 0x1000,
                            bytes: &changed,
                        },
                        0,
                        0x1000,
                        &mut second,
                    );
                    assert!(!first.partition_equal(&second, table));
                    assert!(first.partition_equal(&second, 3 - table));
                }
            }
        }
        let recorded = crate::raw::Recorded::build(&first.log);
        assert!(!recorded.copy(0x4000 + sweather_per_province::WATER, &mut [0; 16]));
        assert!(!recorded.copy(
            0x4000 + sweather_per_province::TEMPERATURE_CHANGE,
            &mut [0; 17]
        ));
        let mut changed = bytes.clone();
        put(
            &mut changed,
            0x2000 + cweather_manager::PROVINCES + 8,
            &4_u32.to_le_bytes(),
        );
        let mut second = WeatherJob::new();
        copy_weather_pass(
            &SliceMemory {
                base: 0x1000,
                bytes: &changed,
            },
            0,
            0x1000,
            &mut second,
        );
        assert!((0..N).all(|slot| !first.partition_equal(&second, slot)));
    }

    #[test]
    fn packed_weather_capacity_failure_grows_off_capture_and_reset_discards_old_prefix() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut job = WeatherJob {
            log: RawLog::new(16, 1024),
            ..Default::default()
        };
        copy_weather_pass(&memory, 0, 0x1000, &mut job);
        assert!(job.log.overflowed);
        assert!(job.grow_for_next());
        assert_eq!(job.regions.required, 30);
        assert_eq!(job.provinces.required, 62);
        job.grow();
        job.clear();
        let pointers = (job.regions.bytes.as_ptr(), job.provinces.bytes.as_ptr());
        copy_weather_pass(&memory, 0, 0x1000, &mut job);
        assert!(!job.log.overflowed);
        assert!(!job.grow_for_next());
        assert_eq!(job.log.raw_bytes, [520, 30, 62]);
        job.clear();
        assert_eq!(job.regions.copied_fields, 0);
        assert_eq!(job.provinces.array, None);
        assert_eq!(
            (job.regions.bytes.as_ptr(), job.provinces.bytes.as_ptr()),
            pointers
        );
        let fault = FaultReads {
            memory,
            fault: 0x4000,
            failures: Cell::new(0),
        };
        copy_weather_pass(&fault, 0, 0x1000, &mut job);
        assert_eq!(job.provinces.copied_fields, 0);
        assert_eq!(job.provinces.copied_bytes, 0);
        assert_eq!(fault.failures.get(), 1);
        assert!(job.log.fault[2]);
    }

    #[test]
    fn packed_weather_preflights_both_tables_and_both_passes_before_any_dense_fault() {
        let bytes = fixture();
        for short_pass in [0, 1] {
            for short_table in [1, 2] {
                for fault_table in [1, 2] {
                    let memory = FaultReads {
                        memory: SliceMemory {
                            base: 0x1000,
                            bytes: &bytes,
                        },
                        fault: if fault_table == 1 { 0x3000 } else { 0x4000 },
                        failures: Cell::new(0),
                    };
                    let mut first = WeatherJob::new();
                    let mut second = WeatherJob::new();
                    let short = if short_pass == 0 {
                        &mut first
                    } else {
                        &mut second
                    };
                    if short_table == 1 {
                        short.regions.bytes.clear();
                    } else {
                        short.provinces.bytes.clear();
                    }
                    let mut captured =
                        Captured::with_capacities(&Capacities { rows: [1; N] }, &[0; N]);
                    let ctx = WeatherCopyContext {
                        base: 0,
                        game_state: 0x1000,
                        needed: [true; N],
                    };
                    let report = copy_weather_refresh(&memory, ctx, &mut first, Some(&mut second));
                    captured.apply_weather_report(ctx, &report);
                    assert!(!report.raw_ok);
                    assert_eq!(
                        memory.failures.get(),
                        0,
                        "preflight must not consume the guard"
                    );
                    for job in [&first, &second] {
                        assert_eq!(job.regions.copied_fields, 0);
                        assert_eq!(job.provinces.copied_fields, 0);
                        assert_eq!(job.log.raw_bytes, [520, 0, 0]);
                    }
                    // The admitted typed fallback is the first dense read,
                    // not a retry of an earlier failed staging batch.
                    captured.capture(&memory, 0, 0x1000, &[false; N]);
                    assert_eq!(memory.failures.get(), 1);
                }
            }
        }
    }

    #[test]
    fn packed_weather_second_header_growth_is_preflighted_before_first_dense_read() {
        struct Growing<'a> {
            memory: FaultReads<'a>,
            headers: Cell<usize>,
        }
        impl Memory for Growing<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if !self.memory.copy(address, output) {
                    return false;
                }
                if address == 0x2000 && output.len() == 0x200 {
                    self.headers.set(self.headers.get() + 1);
                    if self.headers.get() == 2 {
                        output[cweather_manager::PROVINCES_COUNT
                            ..cweather_manager::PROVINCES_COUNT + 4]
                            .copy_from_slice(&2_u32.to_le_bytes());
                    }
                }
                true
            }
        }
        let bytes = fixture();
        let memory = Growing {
            memory: FaultReads {
                memory: SliceMemory {
                    base: 0x1000,
                    bytes: &bytes,
                },
                fault: 0x3000,
                failures: Cell::new(0),
            },
            headers: Cell::new(0),
        };
        let mut first = WeatherJob::new();
        let mut second = WeatherJob::new();
        second
            .provinces
            .bytes
            .resize(sweather_per_province::RECORD_BYTES, 0);
        let mut captured = Captured::with_capacities(&Capacities { rows: [1; N] }, &[0; N]);
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
            needed: [true; N],
        };
        let report = copy_weather_refresh(&memory, ctx, &mut first, Some(&mut second));
        captured.apply_weather_report(ctx, &report);
        assert!(!report.raw_ok);
        assert_eq!(memory.headers.get(), 2);
        assert_eq!(memory.memory.failures.get(), 0);
        assert_eq!(
            first.provinces.required,
            sweather_per_province::RECORD_BYTES
        );
        assert_eq!(
            second.provinces.required,
            2 * sweather_per_province::RECORD_BYTES
        );
        assert!(second.grow_for_next());
    }

    #[test]
    fn packed_weather_root_fault_and_other_pass_shortage_are_terminal_without_retry() {
        struct Guard<'a> {
            memory: SliceMemory<'a>,
            root_reads: Cell<usize>,
        }
        impl Memory for Guard<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == 0x1000 + cgame_state::WEATHER_MANAGER {
                    self.root_reads.set(self.root_reads.get() + 1);
                    if self.root_reads.get() == 1 {
                        return false;
                    }
                }
                self.memory.copy(address, output)
            }
        }
        let bytes = fixture();
        let memory = Guard {
            memory: SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            root_reads: Cell::new(0),
        };
        let mut first = WeatherJob::new();
        let mut second = WeatherJob::new();
        second.provinces.bytes.clear();
        let mut captured = Captured::with_capacities(&Capacities { rows: [1; N] }, &[0; N]);
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
            needed: [true; N],
        };
        let report = copy_weather_refresh(&memory, ctx, &mut first, Some(&mut second));
        captured.apply_weather_report(ctx, &report);
        assert!(report.raw_ok, "terminal payload forbids typed retry");
        assert_eq!(report.matched, [false; N]);
        assert!(first.staging_failure.is_some());
        assert_eq!(memory.root_reads.get(), 2);
        captured.raw_mode = report.raw_ok;
        captured.capture(&memory, 0, 0x1000, &[true; N]);
        captured.decode_weather_payload(0, &first, &mut [0; N]);
        assert_eq!(memory.root_reads.get(), 2);
        assert!(
            captured
                .outcomes()
                .iter()
                .all(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
        );
        first.clear();
        assert!(first.staging_failure.is_none());
    }

    #[test]
    fn weather_split_copy_matches_direct_walk() {
        let capacities = split_capacities();
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let direct = direct(&memory, &capacities);
        let (split, report) = split(&memory, &capacities);
        assert_eq!(report.matched, [true; 3]);
        assert_matches_direct(&split, &direct);
        assert_eq!(split.limits.outcome, Outcome::Rows { len: 1 });
    }

    #[test]
    fn weather_split_copy_matches_direct_walk_under_truncation() {
        let capacities = Capacities { rows: [1, 1, 0] };
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut direct = Captured::with_capacities(&capacities, &[0; N]);
        let (matched, _) = direct.capture(&memory, 0, 0x1000, &[true; N]);
        assert_eq!(matched, [true, true, false]);
        let mut split = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
            needed: [true; N],
        };
        let mut first = WeatherJob::new();
        let mut second = WeatherJob::new();
        let report = copy_weather_refresh(&memory, ctx, &mut first, Some(&mut second));
        split.apply_weather_report(ctx, &report);
        assert!(report.raw_ok);
        split.raw_mode = true;
        let mut ticks = [0u64; 3];
        split.decode_weather_payload(0, &first, &mut ticks);
        assert_matches_direct(&split, &direct);
        assert!(matches!(split.provinces.outcome, Outcome::Truncated { .. }));
    }

    #[test]
    fn weather_split_copy_matches_direct_walk_on_fault_matrix() {
        let capacities = split_capacities();
        let mut bytes = fixture();
        put(
            &mut bytes,
            0x1000 + cgame_state::WEATHER_MANAGER,
            &0_u64.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut direct = Captured::with_capacities(&capacities, &[0; N]);
        let (matched, _) = direct.capture(&memory, 0, 0x1000, &[true; N]);
        assert_eq!(matched, [false; N]);
        let mut split = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
            needed: [true; N],
        };
        let mut first = WeatherJob::new();
        let mut second = WeatherJob::new();
        let report = copy_weather_refresh(&memory, ctx, &mut first, Some(&mut second));
        split.apply_weather_report(ctx, &report);
        assert!(report.raw_ok);
        // Both copies fail identically; raw-compare still matches (no live
        // bytes differ), but the decode installs the identical Unavailable.
        assert_eq!(report.matched, [true; 3]);
        split.raw_mode = true;
        let mut ticks = [0u64; 3];
        split.decode_weather_payload(0, &first, &mut ticks);
        assert_matches_direct(&split, &direct);
    }

    #[test]
    fn weather_double_copy_compare_blocks_checks_on_tearing() {
        let capacities = split_capacities();
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
            needed: [true; N],
        };
        let mut first = WeatherJob::new();
        let mut second = WeatherJob::new();
        let report = copy_weather_refresh(&memory, ctx, &mut first, Some(&mut second));
        captured.apply_weather_report(ctx, &report);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 3]);
        let mut torn = fixture();
        put(
            &mut torn,
            0x4000 + sweather_per_province::WATER,
            &9_u64.to_le_bytes(),
        );
        let mut after = WeatherJob::new();
        copy_weather_pass(&memory, 0, 0x1000, &mut after);
        let mut after_torn = WeatherJob::new();
        copy_weather_pass(
            &SliceMemory {
                base: 0x1000,
                bytes: &torn,
            },
            0,
            0x1000,
            &mut after_torn,
        );
        assert!(!after.partition_equal(&after_torn, 2));
        assert!(after.partition_equal(&after_torn, 1));
    }

    #[test]
    fn weather_partition_equal_isolates_provinces_private_bytes() {
        let capacities = split_capacities();
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
            needed: [true; N],
        };
        let mut first = WeatherJob::new();
        let mut second = WeatherJob::new();
        let report = copy_weather_refresh(&memory, ctx, &mut first, Some(&mut second));
        captured.apply_weather_report(ctx, &report);
        assert!(report.raw_ok);
        second.provinces.bytes[0] ^= 0xff;
        assert!(first.partition_equal(&second, 0));
        assert!(first.partition_equal(&second, 1));
        assert!(!first.partition_equal(&second, 2));
    }

    #[test]
    fn weather_staging_overflow_falls_back_to_typed_walk() {
        let capacities = split_capacities();
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
            needed: [true; N],
        };
        let mut tiny = WeatherJob {
            log: crate::raw::RawLog::new(8, 8),
            ..Default::default()
        };
        let report = copy_weather_refresh(&memory, ctx, &mut tiny, None);
        captured.apply_weather_report(ctx, &report);
        assert!(!report.raw_ok);
        assert!(tiny.log.overflowed);
        assert!(tiny.grow_for_next());
    }

    #[test]
    fn weather_gate_skips_typed_walks_when_raw_mode_set() {
        let (_, mut captured) = small();
        captured.raw_mode = true;
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let (matched, ticks) = captured.capture(&memory, 0, 0x1000, &[true; N]);
        assert_eq!(matched, [false; N]);
        assert_eq!(ticks, [0; N]);
    }

    #[test]
    fn weather_truncation_matrix_matches_direct_with_same_caps() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        for rows in [[0, 0, 0], [1, 1, 1], [2, 8, 8], [8, 8, 8]] {
            let capacities = Capacities { rows };
            let mut direct = Captured::with_capacities(&capacities, &[0; N]);
            let _ = direct.capture(&memory, 0, 0x1000, &[true; N]);
            let mut split = Captured::with_capacities(&capacities, &[0; N]);
            let ctx = WeatherCopyContext {
                base: 0,
                game_state: 0x1000,
                needed: [true; N],
            };
            let mut first = WeatherJob::new();
            let mut second = WeatherJob::new();
            let report = copy_weather_refresh(&memory, ctx, &mut first, Some(&mut second));
            split.apply_weather_report(ctx, &report);
            assert!(report.raw_ok, "rows={rows:?}");
            split.raw_mode = true;
            let mut ticks = [0u64; 3];
            split.decode_weather_payload(0, &first, &mut ticks);
            assert_matches_direct(&split, &direct);
        }
    }

    #[test]
    fn weather_worst_case_pools_start_above_late_campaign_caps() {
        // Late-campaign worst case: 512 regions + 20,000 provinces (pooled
        // caps). Pools start above that so steady state never overflows;
        // overflow still falls back to the typed walk above.
        let job = WeatherJob::new();
        assert!(job.log.spans.capacity() >= 2);
        assert!(job.log.bytes.capacity() >= 520);
        assert!(job.regions.bytes.len() >= 512 * sweather_per_region::RECORD_BYTES);
        assert!(job.provinces.bytes.len() >= 20_000 * sweather_per_province::RECORD_BYTES);
        let capacities = Capacities::default();
        assert!(capacities.rows[1] >= 512);
        assert!(capacities.rows[2] >= 20_000);
    }

    #[test]
    fn weather_bulk_manager_serves_enclosed_ceiling_word() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut job = WeatherJob::new();
        copy_weather_pass(&memory, 0, 0x1000, &mut job);
        assert!(job.log.has_bulk);
        let recorded = crate::raw::Recorded::build(&job.log);
        let header = job
            .log
            .spans
            .iter()
            .find(|span| span.ok && span.bulk && span.len == 0x200)
            .expect("bulk manager header");
        let mut word = [0u8; 8];
        assert!(recorded.copy(header.addr + cweather_manager::MAX_WATER + 3, &mut word));
        let mut expected = [0u8; 8];
        expected.copy_from_slice(
            &bytes[header.addr - 0x1000 + cweather_manager::MAX_WATER + 3
                ..header.addr - 0x1000 + cweather_manager::MAX_WATER + 11],
        );
        assert_eq!(word, expected);
    }
}
