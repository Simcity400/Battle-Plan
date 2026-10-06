//! Native weather acquisition. Only growth and replay conversion allocate.
//!
//! Hourly weather raw-copy split (WeatherRegions, WeatherProvinces): the game
//! thread copies raw bytes only and the single FIFO decode worker runs the
//! native row parsers over owned exact-field gathers. Root topology, row
//! order, truncation and field-failure prefixes survive without retaining
//! unused dense-array padding. The provinces also carry the manager's display
//! bytes, the stored values the snow/mud map texture averages.

use crate::{
    raw::RawLog,
    read_accounting,
    sampling::qpc,
    spooling::table,
    table_capture::{CaptureRows, bounded},
};
use engine::allocation::{filled, owned_vec, resize};
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

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::WeatherRegions, TableId::WeatherProvinces];

/// Manager authentication is charged once to regions for each acquisition pass.
/// The two dense arrays retain their own read times and outcomes.
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("weather manager and dense tables", 0, &[0, 1])];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
}

impl Default for Capacities {
    fn default() -> Self {
        // Starting sizes only: rows grow to the live counts off the capture.
        Self {
            rows: [512, 20_000],
        }
    }
}

pub struct Captured {
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
            regions: CaptureRows::new(capacity.rows[0]),
            provinces: CaptureRows::new(capacity.rows[1]),
            raw_mode: false,
        }
    }

    pub fn reset(&mut self) {
        self.regions.reset();
        self.provinces.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.regions.allocated_bytes() + self.provinces.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.regions.grow(&mut capacity.rows[0]);
        self.provinces.grow(&mut capacity.rows[1]);
    }

    fn fail(&mut self, reason: &'static str) {
        self.regions.outcome = Outcome::Unavailable { reason };
        self.provinces.outcome = Outcome::Unavailable { reason };
    }

    /// Acquire one manager snapshot and both independent dense tables.
    pub fn capture(&mut self, memory: &impl Memory, base: usize, game_state: usize) -> [u64; N] {
        self.reset();
        if self.raw_mode {
            return [0; N];
        }
        let mut ticks = [0; N];
        let begin = qpc();
        let reader = read_accounting::with_table(TableId::WeatherRegions, || {
            cweather_manager::Reader::new(memory, base, game_state)
        });
        ticks[0] = elapsed(begin);
        let reader = match reader {
            Ok(reader) => reader,
            Err(reason) => {
                self.fail(reason);
                return ticks;
            }
        };
        macro_rules! primary {
            ($index:literal, $field:ident) => {{
                let _reads = read_accounting::table(TABLES[$index]);
                let begin = qpc();
                self.$field.outcome =
                    bounded(reader.$field(&mut self.$field.rows), self.$field.rows.len());
                ticks[$index] = ticks[$index].saturating_add(elapsed(begin));
            }};
        }
        primary!(0, regions);
        primary!(1, provinces);
        ticks
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.regions.outcome, self.provinces.outcome]
    }

    /// Spool-thread conversion; raw values and incomplete-table status survive.
    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        [
            table::<tables::WeatherRegions, _>(outcomes[0], &self.regions.rows, |r| {
                rows::WeatherRegionRow {
                    id: r.id,
                    max_snow: r.max_snow,
                    light_rain: r.light_rain,
                    heavy_rain: r.heavy_rain,
                    snow: r.snow,
                    blizzard: r.blizzard,
                    sandstorm: r.sandstorm,
                    arctic_water: r.arctic_water,
                }
            }),
            table::<tables::WeatherProvinces, _>(outcomes[1], &self.provinces.rows, |r| {
                rows::WeatherProvinceRow {
                    id: r.id,
                    snow: r.snow,
                    permanent_snow: r.permanent_snow,
                    display_snow: r.display_snow,
                    display_mud: r.display_mud,
                    temperature: replay::rows::Recorded::Value(r.temperature),
                    raw_mud: replay::rows::Recorded::Value(r.raw_mud),
                }
            }),
        ]
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

// ===========================================================================
// Hourly weather raw-copy split.
//
// The game thread copies raw bytes only: manager pointer and vtable, the two
// dense array headers and the two display array headers, the dense exact
// fields in bounded gathers, and the two display arrays whole. No row is
// decoded on the game thread. Each array keeps its own failure scope.
// ===========================================================================

/// Fault-attribution bits in family order: [regions, provinces].
const M_W_ALL: u32 = 0x03;
const M_REGIONS: u32 = 0x01;
const M_PROVINCES: u32 = 0x02;

/// One hour of raw weather payload. Flat pools recycle capacities and the
/// steady state allocates on neither thread; growth happens on the decode
/// side only (see `crate::decode::recycle_weather_job`).
#[derive(Debug, Default)]
pub(crate) struct WeatherJob {
    pub log: RawLog<N>,
    pub game_state: usize,
    regions: DensePayload,
    provinces: DensePayload,
    capacity_charge: crate::raw_memory::Charge,
    staging_failure: Option<&'static str>,
}

impl WeatherJob {
    pub fn new() -> Self {
        let mut job = Self {
            // Headers plus both display arrays of a 20,000-province map; the
            // log grows from the live count when a larger map overflows it.
            log: RawLog::new(16, 48 * 1024),
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

    /// Double every pool off the game thread until copies fit. Overflowed
    /// hours fall back to the typed walk in the meantime, so sizing is a pure
    /// performance knob.
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

/// Exact declared fields only, not an address-space substitute. The first
/// failed field is retained as a prefix, never retried or zero-filled.
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
            bytes: owned_vec(filled(capacity, 0)),
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
            let required = self.required.max(self.bytes.len().saturating_mul(2));
            let _ = resize(&mut self.bytes, required, 0).map_err(crate::allocation::io);
        }
    }
    fn capture(&mut self, memory: &impl Memory, stride: usize, layout: &RecordLayout<'_>) {
        let Some(array) = self.array else {
            return;
        };
        let record_bytes = layout.stride();
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
            self.copied_fields += completed;
            self.copied_bytes += layout.completed_bytes(completed);
            if completed != count * layout.field_count() {
                break;
            }
            first += count;
        }
    }
    fn faulted(&self, fields: usize) -> bool {
        self.array
            .is_some_and(|array| self.copied_fields < array.count as usize * fields)
    }
    fn decode<R>(
        &self,
        vtable: usize,
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
    pub copy_ticks: [u64; N],
    pub raw_bytes: [u64; N],
}

/// Game-thread copy parameters, bundled to keep the tick entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WeatherCopyContext {
    pub game_state: usize,
}

/// One array header recorded under `mask`, parsed when it reads.
fn header(
    log: &mut RawLog<N>,
    memory: &impl Memory,
    address: Option<usize>,
    stride: usize,
    mask: u32,
) -> Option<engine::cpdx_array::Array> {
    let start = log.take(memory, address?, 16, mask)?;
    engine::cpdx_array::Array::from_header(log.bytes[start..start + 16].try_into().unwrap(), stride)
}

/// Acquire the manager topology once and preflight every destination. Returns
/// the display array headers when both read.
fn prepare_weather_pass(
    memory: &impl Memory,
    game_state: usize,
    job: &mut WeatherJob,
) -> Option<[engine::cpdx_array::Array; 2]> {
    use engine::cgame_state;
    let mut reads = read_accounting::table(TableId::WeatherRegions);
    let log = &mut job.log;
    let pointer_start = log.take(
        memory,
        game_state.checked_add(cgame_state::WEATHER_MANAGER)?,
        8,
        M_W_ALL,
    )?;
    let pointer = usize::try_from(u64::from_le_bytes(
        log.bytes[pointer_start..pointer_start + 8]
            .try_into()
            .unwrap(),
    ))
    .ok()
    .filter(|&pointer| pointer != 0)?;
    log.take(memory, pointer, 8, M_W_ALL)?;
    // The snow ceiling every region row carries; a failed read fails only them.
    if let Some(address) = pointer.checked_add(cweather_manager::MAX_SNOW) {
        let _ = log.take(memory, address, 8, M_REGIONS);
    }
    let regions = header(
        log,
        memory,
        pointer.checked_add(cweather_manager::REGIONS),
        sweather_per_region::STRIDE,
        M_REGIONS,
    );
    reads.select(TableId::WeatherProvinces);
    let provinces = header(
        log,
        memory,
        pointer.checked_add(cweather_manager::PROVINCES),
        sweather_per_province::STRIDE,
        M_PROVINCES,
    );
    let display = provinces.and_then(|_| {
        let snow = header(
            log,
            memory,
            pointer.checked_add(cweather_manager::DISPLAY_SNOW),
            1,
            M_PROVINCES,
        )?;
        let mud = header(
            log,
            memory,
            pointer.checked_add(cweather_manager::DISPLAY_MUD),
            1,
            M_PROVINCES,
        )?;
        Some([snow, mud])
    });
    job.regions
        .prepare(regions, sweather_per_region::RECORD_BYTES);
    job.provinces
        .prepare(provinces, sweather_per_province::RECORD_BYTES);
    display
}

fn copy_weather_dense(
    memory: &impl Memory,
    job: &mut WeatherJob,
    display: Option<[engine::cpdx_array::Array; 2]>,
) {
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
    let mut reads = read_accounting::table(TableId::WeatherRegions);
    job.regions
        .capture(memory, sweather_per_region::STRIDE, &REGIONS_LAYOUT);
    reads.select(TableId::WeatherProvinces);
    job.provinces
        .capture(memory, sweather_per_province::STRIDE, &PROVINCES_LAYOUT);
    let provinces_complete = !job.provinces.faulted(sweather_per_province::FIELDS.len());
    let log = &mut job.log;
    // The display arrays are only read after a clean province census, as the
    // typed walk reads them only after its rows.
    if let Some(arrays) = display.filter(|_| provinces_complete) {
        for array in arrays {
            if log
                .take_bulk(memory, array.data, array.count as usize, M_PROVINCES)
                .is_none()
            {
                break;
            }
        }
    }
    log.raw_bytes[0] += job.regions.copied_bytes as u64;
    log.raw_bytes[1] += job.provinces.copied_bytes as u64;
    log.fault[0] |= job.regions.faulted(sweather_per_region::FIELDS.len());
    log.fault[1] |= !provinces_complete;
}

/// Acquire one refresh into its owned payload.
pub(crate) fn copy_weather_refresh(
    memory: &impl Memory,
    ctx: WeatherCopyContext,
    job: &mut WeatherJob,
) -> WeatherCopyReport {
    let started = qpc();
    job.clear();
    job.game_state = ctx.game_state;
    let display = prepare_weather_pass(memory, ctx.game_state, job);
    let display_bytes = display.map_or(0, |arrays| 2 * arrays[0].count as usize);
    job.log.reserve(display_bytes);
    job.log.overflowed |= job.regions.needs_growth() || job.provinces.needs_growth();
    let overflow = job.log.overflowed;
    let root_fault = job.log.fault.iter().any(|&fault| fault);
    // Keep a failed source terminal when storage is also exhausted; retrying
    // the typed path would otherwise observe the failed native source again.
    if overflow && root_fault {
        job.staging_failure = Some("weather staging unavailable after source read failure");
    }
    if !overflow {
        copy_weather_dense(memory, job, display);
    }
    WeatherCopyReport {
        raw_ok: !overflow || job.staging_failure.is_some(),
        copy_ticks: [elapsed(started), 0],
        raw_bytes: job.log.raw_bytes,
    }
}

impl Captured {
    /// Mark an accepted acquisition failure without retrying live memory.
    pub(crate) fn fail_weather_capture(&mut self, reason: &'static str) {
        self.fail(reason);
        self.raw_mode = false;
    }

    /// Decode-worker interpretation of a raw hourly payload into the pooled
    /// typed rows. Outcomes match the typed walk exactly because the same row
    /// parsers and display join consume the recorded bytes. Clears `raw_mode`
    /// so the next capture starts clean.
    pub(crate) fn decode_weather_payload(
        &mut self,
        base: usize,
        job: &WeatherJob,
        decode_ticks: &mut [u64; N],
    ) {
        self.raw_mode = false;
        if let Some(reason) = job.staging_failure {
            self.fail(reason);
            return;
        }
        let recorded = crate::raw::Recorded::build(&job.log);
        let started = qpc();
        let reader = match cweather_manager::Reader::new(&recorded, base, job.game_state) {
            Ok(reader) => reader,
            Err(reason) => {
                self.fail(reason);
                decode_ticks[0] += elapsed(started);
                return;
            }
        };
        self.regions.outcome = match reader.max_snow() {
            Ok(max_snow) => {
                let outcome = job.regions.decode(
                    base.wrapping_add(sweather_per_region::VTABLE_RVA),
                    sweather_per_region::FIELDS.len(),
                    sweather_per_region::RECORD_BYTES,
                    &mut self.regions.rows,
                    "weather region array unreadable or invalid",
                    sweather_per_region::decode_record,
                );
                for row in &mut self.regions.rows[..outcome.len()] {
                    row.max_snow = max_snow;
                }
                outcome
            }
            Err(reason) => Outcome::Unavailable { reason },
        };
        decode_ticks[0] += elapsed(started);
        let started = qpc();
        let outcome = job.provinces.decode(
            base.wrapping_add(sweather_per_province::VTABLE_RVA),
            sweather_per_province::FIELDS.len(),
            sweather_per_province::RECORD_BYTES,
            &mut self.provinces.rows,
            "weather province array unreadable or invalid",
            sweather_per_province::decode_record,
        );
        self.provinces.outcome = match (outcome, job.provinces.array) {
            (Outcome::Unavailable { .. }, _) | (_, None) => outcome,
            (_, Some(array)) => {
                let len = outcome.len();
                match reader.display_arrays().and_then(|arrays| {
                    sweather_per_province::fill_display(
                        &recorded,
                        arrays,
                        array.count,
                        &mut self.provinces.rows[..len],
                    )
                }) {
                    Ok(()) => bounded(outcome, self.provinces.rows.len()),
                    Err(reason) => Outcome::Unavailable { reason },
                }
            }
        };
        self.regions.outcome = bounded(self.regions.outcome, self.regions.rows.len());
        decode_ticks[1] += elapsed(started);
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "weather/bench/tests.rs"]
mod bench;
