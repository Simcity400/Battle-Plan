//! Native weather acquisition. Only growth and replay conversion allocate.
//!
//! Hourly weather raw-copy split (WeatherLimits, WeatherRegions,
//! WeatherProvinces): the game thread copies raw bytes only and the single
//! FIFO decode worker runs the native row parsers over owned exact-field
//! gathers. Root topology, row order, truncation and field-failure prefixes
//! survive without retaining unused dense-array padding or rendering bytes.

use crate::{
    raw::RawLog, read_accounting, sampling::qpc, spooling::table, table_capture::CaptureRows,
};
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
/// The two dense arrays retain their own read times and outcomes.
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
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            limits: CaptureRows::new(capacity.rows[0]),
            regions: CaptureRows::new(capacity.rows[1]),
            provinces: CaptureRows::new(capacity.rows[2]),
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

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.limits.grow(&mut capacity.rows[0]);
        self.regions.grow(&mut capacity.rows[1]);
        self.provinces.grow(&mut capacity.rows[2]);
    }

    /// Acquire one manager snapshot and both independent dense tables.
    pub fn capture(&mut self, memory: &impl Memory, base: usize, game_state: usize) -> [u64; N] {
        self.reset();
        if self.raw_mode {
            return [0; N];
        }
        let mut ticks = [0; N];
        let begin = qpc();
        let first = read_accounting::with_table(TableId::WeatherLimits, || {
            cweather_manager::Reader::new(memory, base, game_state)
        });
        ticks[0] = elapsed(begin);
        let first = match first {
            Ok(reader) => reader,
            Err(reason) => {
                self.limits.outcome = Outcome::Unavailable { reason };
                self.regions.outcome = Outcome::Unavailable { reason };
                self.provinces.outcome = Outcome::Unavailable { reason };
                return ticks;
            }
        };
        macro_rules! primary {
            ($index:literal, $field:ident) => {{
                let _reads = read_accounting::table(TABLES[$index]);
                let begin = qpc();
                self.$field.outcome =
                    bounded(first.$field(&mut self.$field.rows), self.$field.rows.len());
                ticks[$index] = ticks[$index].saturating_add(elapsed(begin));
            }};
        }
        primary!(0, limits);
        primary!(1, regions);
        primary!(2, provinces);
        ticks
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.limits.outcome,
            self.regions.outcome,
            self.provinces.outcome,
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
}

/// Exact declared fields only, not an address-space substitute. Skipped native
/// padding, rendering bytes and modifier contents are omitted; every recorded
/// field, identity and full custom-array header is retained.
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
    pub copy_ticks: [u64; 3],
    pub raw_bytes: [u64; 3],
}

/// Game-thread copy parameters, bundled to keep the tick entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WeatherCopyContext {
    pub base: usize,
    pub game_state: usize,
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

/// Acquire the manager topology once and preflight both dense destinations.
fn prepare_weather_pass(memory: &impl Memory, game_state: usize, job: &mut WeatherJob) {
    use engine::{cgame_state, cpdx_array::Array, cweather_manager};
    let _reads = read_accounting::table(TableId::WeatherLimits);
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
    // table before these reads, so a later shortage cannot cause a typed
    // retry after an earlier read consumed a PAGE_GUARD or otherwise failed.
    let mut reads = read_accounting::table(TableId::WeatherRegions);
    log.overflowed |= !job.regions.capture(
        memory,
        job.regions.array,
        sweather_per_region::STRIDE,
        &REGIONS_LAYOUT,
    );
    reads.select(TableId::WeatherProvinces);
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

/// Acquire one refresh into its owned payload.
pub(crate) fn copy_weather_refresh(
    memory: &impl Memory,
    ctx: WeatherCopyContext,
    job: &mut WeatherJob,
) -> WeatherCopyReport {
    let _ = ctx.base;
    let started = qpc();
    job.clear();
    job.game_state = ctx.game_state;
    prepare_weather_pass(memory, ctx.game_state, job);
    preflight_weather(job);
    let overflow = job.log.overflowed;
    let root_fault = job.log.fault.iter().any(|&fault| fault);
    // Keep a failed source terminal when storage is also exhausted; retrying
    // the typed path would otherwise observe the failed native source again.
    if overflow && root_fault {
        job.staging_failure = Some("weather staging unavailable after source read failure");
    }
    if !overflow {
        copy_weather_dense(memory, job);
    }
    WeatherCopyReport {
        raw_ok: !overflow || job.staging_failure.is_some(),
        copy_ticks: [elapsed(started), 0, 0],
        raw_bytes: job.log.raw_bytes,
    }
}

impl Captured {
    /// Mark an accepted acquisition failure without retrying live memory.
    pub(crate) fn fail_weather_capture(&mut self, reason: &'static str) {
        self.limits.outcome = Outcome::Unavailable { reason };
        self.regions.outcome = Outcome::Unavailable { reason };
        self.provinces.outcome = Outcome::Unavailable { reason };
        self.raw_mode = false;
    }

    /// Decode-worker interpretation of a raw hourly payload into the pooled
    /// typed rows. Outcomes match the typed walk exactly because the
    /// identical row parser consumes the exact-field prefix. Clears `raw_mode` so the next capture
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
    use engine::{
        cgame_state,
        memory::{CountedMemory, ReadCounts, SliceMemory},
    };
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
        let captured = Captured::with_capacities(&capacities);
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
        let _ticks = captured.capture(&memory, 0, 0x1000);
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
    fn table_faults_and_truncation_remain_independent() {
        let (mut capacities, _) = small();
        capacities.rows[2] = 0;
        let mut captured = Captured::with_capacities(&capacities);
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
        let _ticks = captured.capture(&memory, 0, 0x1000);
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
        captured.grow(&mut capacities);
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
        captured.capture(&memory, 0, 0x1000);
        assert!(
            captured
                .outcomes()
                .into_iter()
                .all(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
        );
    }

    fn split_capacities() -> Capacities {
        Capacities { rows: [2, 8, 8] }
    }

    fn direct(memory: &SliceMemory<'_>, capacities: &Capacities) -> Captured {
        let mut captured = Captured::with_capacities(capacities);
        let _ticks = captured.capture(memory, 0, 0x1000);
        captured
    }

    fn split(memory: &SliceMemory<'_>, capacities: &Capacities) -> (Captured, WeatherCopyReport) {
        let mut captured = Captured::with_capacities(capacities);
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
        };
        let mut first = WeatherJob::new();
        let report = copy_weather_refresh(memory, ctx, &mut first);
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
        let mut direct = Captured::with_capacities(capacities);
        direct.capture(memory, 0, 0x1000);
        let mut packed = Captured::with_capacities(capacities);
        let mut job = WeatherJob::new();
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
        };
        let report = copy_weather_refresh(memory, ctx, &mut job);
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
        fn payload(job: &WeatherJob, table: usize) -> &[u8] {
            let dense = if table == 1 {
                &job.regions
            } else {
                &job.provinces
            };
            &dense.bytes[..dense.copied_bytes]
        }
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
                assert_eq!(payload(&first, table), payload(&second, table));
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
                    assert_ne!(payload(&first, table), payload(&second, table));
                    assert_eq!(payload(&first, 3 - table), payload(&second, 3 - table));
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
        assert_ne!(first.log.bytes, second.log.bytes);
        // Capacity is retained in the raw header; Array describes data/count.
        assert_eq!(first.provinces.array, second.provinces.array);
        assert_eq!(payload(&first, 2), payload(&second, 2));
        put(
            &mut changed,
            0x2000 + cweather_manager::PROVINCES + 12,
            &2_u32.to_le_bytes(),
        );
        second.clear();
        copy_weather_pass(
            &SliceMemory {
                base: 0x1000,
                bytes: &changed,
            },
            0,
            0x1000,
            &mut second,
        );
        assert_ne!(first.provinces.array, second.provinces.array);
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
    fn packed_weather_preflights_both_tables_before_any_dense_fault() {
        let bytes = fixture();
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
                let mut job = WeatherJob::new();
                if short_table == 1 {
                    job.regions.bytes.clear();
                } else {
                    job.provinces.bytes.clear();
                }
                let mut captured = Captured::with_capacities(&Capacities { rows: [1; N] });
                let ctx = WeatherCopyContext {
                    base: 0,
                    game_state: 0x1000,
                };
                let report = copy_weather_refresh(&memory, ctx, &mut job);
                assert!(!report.raw_ok);
                assert_eq!(
                    memory.failures.get(),
                    0,
                    "preflight must not consume the guard"
                );
                assert_eq!(job.regions.copied_fields, 0);
                assert_eq!(job.provinces.copied_fields, 0);
                assert_eq!(job.log.raw_bytes, [520, 0, 0]);
                captured.capture(&memory, 0, 0x1000);
                assert_eq!(memory.failures.get(), 1);
            }
        }
    }

    #[test]
    fn packed_weather_first_root_fault_is_terminal_without_retry() {
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
        let memory = CountedMemory::new(Guard {
            memory: SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            root_reads: Cell::new(0),
        });
        let mut first = WeatherJob::new();
        let mut captured = Captured::with_capacities(&Capacities { rows: [1; N] });
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
        };
        let counts = read_accounting::Capture::new();
        let report = copy_weather_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok, "terminal payload forbids typed retry");
        assert!(first.log.fault.iter().all(|fault| *fault));
        assert_eq!(memory.0.root_reads.get(), 1);
        captured.raw_mode = report.raw_ok;
        captured.capture(&memory, 0, 0x1000);
        captured.decode_weather_payload(0, &first, &mut [0; N]);
        assert_eq!(memory.0.root_reads.get(), 1);
        let mut expected = read_accounting::ZERO;
        expected[TableId::WeatherLimits.index()] = ReadCounts {
            copy_calls: 1,
            spans: 1,
            bytes: 8,
            ..ReadCounts::ZERO
        };
        assert_eq!(counts.finish(), expected);
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
        assert!(report.raw_ok);
        assert_matches_direct(&split, &direct);
        assert_eq!(split.limits.outcome, Outcome::Rows { len: 1 });
    }

    #[test]
    fn weather_read_accounting_partitions_native_gathers_and_excludes_owned_decode() {
        let capacities = split_capacities();
        let bytes = fixture();
        let memory = CountedMemory::new(SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        });
        let direct = direct(&memory.0, &capacities);
        let mut captured = Captured::with_capacities(&capacities);
        let mut job = WeatherJob::new();
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
        };
        let counts = read_accounting::Capture::new();
        let report = copy_weather_refresh(&memory, ctx, &mut job);
        assert!(report.raw_ok);
        let mut expected = read_accounting::ZERO;
        expected[TableId::WeatherLimits.index()] = ReadCounts {
            copy_calls: 2,
            spans: 2,
            bytes: 520,
            ..ReadCounts::ZERO
        };
        expected[TableId::WeatherRegions.index()] = ReadCounts {
            record_calls: 1,
            spans: 9,
            bytes: 30,
            ..ReadCounts::ZERO
        };
        expected[TableId::WeatherProvinces.index()] = ReadCounts {
            record_calls: 1,
            spans: 6,
            bytes: 62,
            ..ReadCounts::ZERO
        };
        assert_eq!(read_accounting::snapshot(), expected);
        captured.raw_mode = true;
        assert_eq!(captured.capture(&memory, 0, 0x1000), [0; N]);
        captured.decode_weather_payload(0, &job, &mut [0; N]);
        assert_matches_direct(&captured, &direct);
        assert_eq!(counts.finish(), expected);
    }

    #[test]
    fn weather_split_copy_matches_direct_walk_under_truncation() {
        let capacities = Capacities { rows: [1, 1, 0] };
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut direct = Captured::with_capacities(&capacities);
        let _ticks = direct.capture(&memory, 0, 0x1000);
        let mut split = Captured::with_capacities(&capacities);
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
        };
        let mut first = WeatherJob::new();
        let report = copy_weather_refresh(&memory, ctx, &mut first);
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
        let mut direct = Captured::with_capacities(&capacities);
        let _ticks = direct.capture(&memory, 0, 0x1000);
        let mut split = Captured::with_capacities(&capacities);
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
        };
        let mut first = WeatherJob::new();
        let report = copy_weather_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        // The owned first fault decodes to the identical Unavailable outcome.
        split.raw_mode = true;
        let mut ticks = [0u64; 3];
        split.decode_weather_payload(0, &first, &mut ticks);
        assert_matches_direct(&split, &direct);
    }

    #[test]
    fn weather_staging_overflow_falls_back_to_typed_walk() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
        };
        let mut tiny = WeatherJob {
            log: crate::raw::RawLog::new(8, 8),
            ..Default::default()
        };
        let report = copy_weather_refresh(&memory, ctx, &mut tiny);
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
        let ticks = captured.capture(&memory, 0, 0x1000);
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
            let mut direct = Captured::with_capacities(&capacities);
            let _ = direct.capture(&memory, 0, 0x1000);
            let mut split = Captured::with_capacities(&capacities);
            let ctx = WeatherCopyContext {
                base: 0,
                game_state: 0x1000,
            };
            let mut first = WeatherJob::new();
            let report = copy_weather_refresh(&memory, ctx, &mut first);
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
