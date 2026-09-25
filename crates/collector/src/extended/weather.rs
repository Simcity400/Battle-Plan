//! Native weather acquisition. Only growth and replay conversion allocate.
//!
//! Hourly weather raw-copy split (WeatherRegions, WeatherProvinces): the game
//! thread copies raw bytes only and the single FIFO decode worker runs the
//! native row parsers over owned exact-field gathers. Root topology, row
//! order, truncation and field-failure prefixes survive without retaining
//! unused dense-array padding. The provinces also carry the manager's display
//! bytes, the stored values the snow/mud map texture averages.

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
        Self { rows: [512, 20_000] }
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
    engine::cpdx_array::Array::from_header(
        log.bytes[start..start + 16].try_into().unwrap(),
        stride,
    )
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
        log.bytes[pointer_start..pointer_start + 8].try_into().unwrap(),
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
    let provinces_complete = !job
        .provinces
        .faulted(sweather_per_province::FIELDS.len());
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
mod tests {
    use super::*;
    use engine::{cgame_state, memory::SliceMemory};
    use std::cell::Cell;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - 0x1000..address - 0x1000 + value.len()].copy_from_slice(value);
    }

    /// Manager 0x2000, one region at 0x3000, one province at 0x4000, display
    /// snow at 0x4800 and mud at 0x4810.
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
        for (offset, address, value) in [
            (cweather_manager::DISPLAY_SNOW, 0x4800_u64, 200),
            (cweather_manager::DISPLAY_MUD, 0x4810, 255),
        ] {
            put(&mut bytes, 0x2000 + offset, &address.to_le_bytes());
            put(&mut bytes, 0x2000 + offset + 8, &4_u32.to_le_bytes());
            put(&mut bytes, 0x2000 + offset + 12, &1_u32.to_le_bytes());
            put(&mut bytes, address as usize, &[value]);
        }
        put(&mut bytes, 0x3000 + sweather_per_region::SNOW, &[255]);
        put(
            &mut bytes,
            0x2000 + cweather_manager::MAX_SNOW,
            &8_000_000_i64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x4000 + sweather_per_province::PERMANENT_SNOW,
            &40_000_i64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x4000 + sweather_per_province::SNOW,
            &((1_i64 << 40) + 3).to_le_bytes(),
        );
        bytes
    }

    fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory {
            base: 0x1000,
            bytes,
        }
    }

    fn direct(memory: &impl Memory, capacities: &Capacities) -> Captured {
        let mut captured = Captured::with_capacities(capacities);
        let _ticks = captured.capture(memory, 0, 0x1000);
        captured
    }

    fn split(memory: &impl Memory, capacities: &Capacities) -> (Captured, WeatherJob) {
        let mut captured = Captured::with_capacities(capacities);
        let mut job = WeatherJob::new();
        let ctx = WeatherCopyContext {
            game_state: 0x1000,
        };
        let report = copy_weather_refresh(memory, ctx, &mut job);
        assert!(report.raw_ok);
        captured.raw_mode = true;
        assert_eq!(captured.capture(memory, 0, 0x1000), [0; N]);
        captured.decode_weather_payload(0, &job, &mut [0; N]);
        (captured, job)
    }

    fn assert_matches(split: &Captured, direct: &Captured) {
        assert_eq!(split.outcomes(), direct.outcomes());
        let len = split.regions.outcome.len();
        assert_eq!(split.regions.rows[..len], direct.regions.rows[..len]);
        let len = split.provinces.outcome.len();
        assert_eq!(split.provinces.rows[..len], direct.provinces.rows[..len]);
    }

    #[test]
    fn acquisition_keeps_native_snow_display_bytes_and_raw_flags() {
        let bytes = fixture();
        let captured = direct(&memory(&bytes), &Capacities { rows: [2; N] });
        let tables = captured.tables();
        let TableView::Complete(regions) = &tables[0] else {
            panic!("regions")
        };
        let region = &regions.rows::<tables::WeatherRegions>().unwrap()[0];
        assert_eq!((region.snow, region.max_snow), (255, 8_000_000));
        let TableView::Complete(provinces) = &tables[1] else {
            panic!("provinces")
        };
        assert_eq!(
            provinces.rows::<tables::WeatherProvinces>().unwrap(),
            &[rows::WeatherProvinceRow {
                id: 0,
                snow: (1_i64 << 40) + 3,
                permanent_snow: 40_000,
                display_snow: 200,
                display_mud: 255,
            }]
        );
    }

    #[test]
    fn raw_copy_matches_the_typed_walk_under_truncation_and_faults() {
        let clean = fixture();
        let mut cases = vec![clean.clone()];
        for (address, value) in [
            // Region header invalid: provinces stay available.
            (0x2000 + cweather_manager::REGIONS_COUNT, (-1_i32).to_le_bytes()),
            // Province identity mismatch.
            (0x4000 + sweather_per_province::ID, 7_i32.to_le_bytes()),
            // Display array sized for another province count.
            (0x2000 + cweather_manager::DISPLAY_MUD + 12, 2_i32.to_le_bytes()),
        ] {
            let mut bytes = clean.clone();
            put(&mut bytes, address, &value);
            cases.push(bytes);
        }
        let mut absent = clean.clone();
        put(
            &mut absent,
            0x1000 + cgame_state::WEATHER_MANAGER,
            &0_u64.to_le_bytes(),
        );
        cases.push(absent);
        for bytes in &cases {
            for rows in [[0, 0], [1, 1], [8, 8]] {
                let capacities = Capacities { rows };
                let (split, _) = split(&memory(bytes), &capacities);
                assert_matches(&split, &direct(&memory(bytes), &capacities));
            }
        }
    }

    /// More provinces than one display-array read chunk: the decode worker
    /// serves chunked reads from the recorded bulk spans.
    #[test]
    fn display_arrays_longer_than_a_read_chunk_decode_from_the_bulk_copy() {
        const PROVINCES: usize = 1500;
        let stride = sweather_per_province::STRIDE;
        let (array, snow, mud) = (0x10000, 0x10000 + PROVINCES * stride, 0x10000 + PROVINCES * stride + 0x1000);
        let mut bytes = fixture();
        bytes.resize(mud + PROVINCES - 0x1000, 0);
        put(&mut bytes, 0x2000 + cweather_manager::PROVINCES, &(array as u64).to_le_bytes());
        for (offset, data) in [
            (cweather_manager::PROVINCES, array),
            (cweather_manager::DISPLAY_SNOW, snow),
            (cweather_manager::DISPLAY_MUD, mud),
        ] {
            put(&mut bytes, 0x2000 + offset, &(data as u64).to_le_bytes());
            put(&mut bytes, 0x2000 + offset + 8, &(PROVINCES as u32).to_le_bytes());
            put(&mut bytes, 0x2000 + offset + 12, &(PROVINCES as u32).to_le_bytes());
        }
        for index in 0..PROVINCES {
            let row = array + index * stride;
            put(&mut bytes, row, &(sweather_per_province::VTABLE_RVA as u64).to_le_bytes());
            put(&mut bytes, row + 8, &(index as u32).to_le_bytes());
            put(&mut bytes, snow + index, &[index as u8]);
            put(&mut bytes, mud + index, &[(index % 2 * 255) as u8]);
        }
        let capacities = Capacities { rows: [8, PROVINCES] };
        let (split, _) = split(&memory(&bytes), &capacities);
        assert_eq!(split.provinces.outcome, Outcome::Rows { len: PROVINCES });
        assert_eq!(
            (split.provinces.rows[1499].display_snow, split.provinces.rows[1499].display_mud),
            (1499_u32 as u8, 255)
        );
        assert_matches(&split, &direct(&memory(&bytes), &capacities));
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
                output.fill(0xab);
                return false;
            }
            self.memory.copy(address, output)
        }
    }

    #[test]
    fn a_faulted_field_or_display_array_is_unavailable_in_both_paths() {
        let bytes = fixture();
        for fault in [
            0x3000 + sweather_per_region::SNOW,
            0x2000 + cweather_manager::MAX_SNOW + 3,
            0x4000 + sweather_per_province::SNOW + 3,
            0x4810,
        ] {
            let memory = FaultReads {
                memory: memory(&bytes),
                fault,
                failures: Cell::new(0),
            };
            let capacities = Capacities { rows: [2; N] };
            let (split, job) = split(&memory, &capacities);
            assert!(job.log.fault.iter().any(|&fault| fault));
            assert_matches(&split, &direct(&memory, &capacities));
            assert!(
                split
                    .outcomes()
                    .iter()
                    .any(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
            );
        }
    }

    #[test]
    fn staging_overflow_falls_back_to_the_typed_walk_and_grows() {
        let bytes = fixture();
        let ctx = WeatherCopyContext {
            game_state: 0x1000,
        };
        let mut tiny = WeatherJob {
            log: RawLog::new(8, 8),
            ..Default::default()
        };
        let report = copy_weather_refresh(&memory(&bytes), ctx, &mut tiny);
        assert!(!report.raw_ok);
        // Growth converges within a few hours; each overflowed hour takes the
        // typed walk.
        let mut hours = 0;
        while tiny.grow_for_next() {
            tiny.grow();
            hours += 1;
            assert!(hours < 16);
            copy_weather_refresh(&memory(&bytes), ctx, &mut tiny);
        }
        assert!(copy_weather_refresh(&memory(&bytes), ctx, &mut tiny).raw_ok);
    }
}
