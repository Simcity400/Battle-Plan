//! Native weather acquisition. Only growth and replay conversion allocate.
//!
//! Hourly weather raw-copy split (WeatherRegions, WeatherProvinces): the game
//! acquisition worker copies raw bytes and the single FIFO decode worker runs the
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

mod display;
use display::DisplayCapture;
static DISPLAY_DEMAND_FAILURES: crate::country_scale::DemandFailures =
    crate::country_scale::DemandFailures::new();

pub(crate) fn observe_display_capacity(
    memory: &impl Memory,
    base: usize,
) -> Result<usize, &'static str> {
    display::observe_capacity(memory, base, &DISPLAY_DEMAND_FAILURES)
}

pub(crate) fn display_demand_diagnostics() -> impl serde::Serialize {
    DISPLAY_DEMAND_FAILURES.diagnostics()
}

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::WeatherRegions, TableId::WeatherProvinces];

/// Manager authentication is charged once to regions for each acquisition pass.
/// The two dense arrays retain their own read times and outcomes.
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("weather manager and dense tables", 0, &[0, 1])];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub display_bytes: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [0; N],
            display_bytes: display::bytes_hint(),
        }
    }
}

impl Capacities {
    /// Off-thread live headers only; these hints never supply recorded values.
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let reader = cweather_manager::Reader::new(memory, base, state)?;
        let mut refusal = None;
        for (index, demand) in [reader.region_count(), reader.province_count()]
            .into_iter()
            .enumerate()
        {
            match demand {
                Ok(count) => crate::country_scale::raise(
                    &mut self.rows[index],
                    crate::country_scale::rows(count, 1),
                ),
                Err(reason) => {
                    refusal.get_or_insert(reason);
                }
            }
        }
        self.display_bytes = self.display_bytes.max(display::bytes_hint());
        refusal.map_or(Ok(()), Err)
    }
}

pub struct Captured {
    regions: CaptureRows<sweather_per_region::Row>,
    provinces: CaptureRows<sweather_per_province::Row>,
    attack: crate::table_capture::Arena<engine::selected_attack::WeatherAttack>,
    attack_metadata: engine::selected_attack::ScalarMetadata,
    attack_hour: i32,
    display: DisplayCapture,
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
            attack: crate::table_capture::Arena::new(capacity.rows[0]),
            attack_metadata: engine::selected_attack::ScalarMetadata::try_new(capacity.rows[0])
                .map_err(crate::allocation::io)
                .unwrap_or_default(),
            attack_hour: 0,
            display: DisplayCapture::new(capacity.display_bytes),
            raw_mode: false,
        }
    }

    pub fn reset(&mut self) {
        self.regions.reset();
        self.provinces.reset();
        self.attack.reset();
        self.attack_metadata.clear();
        self.attack_hour = 0;
        self.display.clear();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.regions.allocated_bytes()
            + self.provinces.allocated_bytes()
            + self.attack.allocated_bytes()
            + self.attack_metadata.allocated_bytes()
            + self.display.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.attack.grow(&mut capacity.rows[0]);
        self.regions.grow(&mut capacity.rows[0]);
        let _ = self
            .attack_metadata
            .reserve(capacity.rows[0])
            .map_err(crate::allocation::io);
        self.provinces.grow(&mut capacity.rows[1]);
        self.display.grow(&mut capacity.display_bytes);
    }

    fn fail(&mut self, reason: &'static str) {
        self.display.clear();
        self.regions.outcome = Outcome::Unavailable { reason };
        self.provinces.outcome = Outcome::Unavailable { reason };
    }

    /// Prepare the decode handoff; missing acquisition cannot retry live memory.
    pub(crate) fn finish_owned_capture(
        &mut self,
        attack_guard: Result<(), &'static str>,
    ) -> [u64; N] {
        self.reset();
        self.attack_metadata.set_guard(attack_guard);
        if !self.raw_mode {
            self.fail_weather_capture("weather acquisition not supplied");
        }
        [0; N]
    }

    /// Acquire one manager snapshot and both independent dense tables.
    #[cfg(test)]
    pub fn capture(&mut self, memory: &impl Memory, base: usize, game_state: usize) -> [u64; N] {
        self.capture_at(
            memory,
            base,
            game_state,
            0,
            Err("selected attack modifier layout unverified"),
        )
    }
    #[cfg(test)]
    pub(crate) fn capture_at(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        attack_guard: Result<(), &'static str>,
    ) -> [u64; N] {
        self.reset();
        self.attack_metadata.set_guard(attack_guard);
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
        {
            let _reads = read_accounting::table(TABLES[0]);
            let begin = qpc();
            self.attack_hour = hour;
            self.attack
                .first
                .fill(engine::selected_attack::WeatherAttack::refused(
                    hour,
                    "selected attack weather append unobserved",
                ));
            self.regions.outcome = bounded(
                reader.regions_with_attack(
                    &mut self.regions.rows,
                    hour,
                    &mut self.attack.first,
                    &mut self.attack_metadata,
                ),
                self.regions.rows.len(),
            );
            self.attack.len = self.regions.outcome.len().min(self.attack.first.len());
            self.attack.required = self.regions.outcome.len();
            ticks[0] = ticks[0].saturating_add(elapsed(begin));
        }
        primary!(1, provinces);
        {
            let _reads = read_accounting::table(TABLES[1]);
            let begin = qpc();
            self.display.capture(memory, reader.display_grid());
            ticks[1] = ticks[1].saturating_add(elapsed(begin));
        }
        self.attack_metadata.finish_native();
        ticks
    }

    pub(super) fn attack_refusals(&self) -> [u64; 2] {
        let mut refused = [0_u64; 2];
        for index in 0..self.regions.outcome.len() {
            let value = self
                .attack
                .first
                .get(index)
                .filter(|_| index < self.attack.len);
            refused[0] += u64::from(value.is_none_or(|value| value.attack.is_err()));
            refused[1] += u64::from(value.is_none_or(|value| value.breakthrough.is_err()));
        }
        refused
    }

    pub(super) fn display_failure(&self) -> Option<&'static str> {
        (!self.provinces.outcome.is_empty())
            .then(|| self.display.header.err())
            .flatten()
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
                    attack: rows::Recorded::Value(super::attack_inputs::weather(
                        self.attack
                            .first
                            .get(r.id as usize)
                            .filter(|_| (r.id as usize) < self.attack.len)
                            .copied()
                            .unwrap_or_else(|| {
                                engine::selected_attack::WeatherAttack::refused(
                                    self.attack_hour,
                                    "selected attack weather append capacity exhausted",
                                )
                            }),
                    )),
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
                    local_hour: replay::rows::Recorded::Value(r.local_hour),
                    night_factor: replay::rows::Recorded::Value(r.night_factor),
                    display: rows::Recorded::Value(if r.id == 0 {
                        self.display.row()
                    } else {
                        None
                    }),
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
// The acquisition worker copies raw bytes only: manager pointer and vtable, the
// weather and clock array headers and the two display array headers, the exact
// fields in bounded gathers, and the two display arrays whole. No row is
// decoded during acquisition. Each array keeps its own failure scope.
// ===========================================================================

/// Fault-attribution bits in family order: [regions, provinces].
const M_W_ALL: u32 = 0x03;
const M_REGIONS: u32 = 0x01;
const M_PROVINCES: u32 = 0x02;

/// One hour of raw weather payload. Worker acquisition grows from original
/// headers and owned decoding grows the rows before publication.
#[derive(Debug, Default)]
pub(crate) struct WeatherJob {
    pub log: RawLog<N>,
    // Fixed mode exists for callback-capacity/fault fixtures only.
    worker_growth: bool,
    pub game_state: usize,
    regions: DensePayload,
    provinces: DensePayload,
    clocks: DensePayload,
    capacity_charge: crate::raw_memory::Charge,
    staging_failure: Option<&'static str>,
    attack: Vec<engine::selected_attack::WeatherAttack>,
    attack_metadata: engine::selected_attack::ScalarMetadata,
    attack_required: usize,
    attack_hour: i32,
    display: DisplayCapture,
    manager: usize,
    manager_authenticated: bool,
}

impl WeatherJob {
    pub fn is_provisioned(&self) -> bool {
        self.worker_growth || self.log.spans.capacity() != 0
    }

    pub fn new() -> Self {
        let mut job = Self {
            log: RawLog::new(0, 0),
            worker_growth: true,
            game_state: 0,
            regions: DensePayload::new(0),
            provinces: DensePayload::new(0),
            clocks: DensePayload::new(0),
            capacity_charge: Default::default(),
            staging_failure: None,
            attack: owned_vec(filled(0, engine::selected_attack::WeatherAttack::default())),
            attack_metadata: engine::selected_attack::ScalarMetadata::try_new(0)
                .map_err(crate::allocation::io)
                .unwrap_or_default(),
            attack_required: 0,
            attack_hour: 0,
            display: DisplayCapture::new(display::bytes_hint()),
            manager: 0,
            manager_authenticated: false,
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
        self.clocks.clear();
        self.staging_failure = None;
        self.attack.fill(Default::default());
        self.attack_metadata.clear();
        self.attack_required = 0;
        self.attack_hour = 0;
        self.display.clear();
        self.manager = 0;
        self.manager_authenticated = false;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
            || self.regions.needs_growth()
            || self.provinces.needs_growth()
            || self.clocks.needs_growth()
            || self.attack_required > self.attack.len()
            || self.display.needs_growth()
    }

    /// Recycled fixed fixtures retain their observed demand. Production workers
    /// grow these pools from the original headers before gathering fields.
    pub fn grow(&mut self) {
        self.log.grow();
        self.regions.grow();
        self.provinces.grow();
        self.clocks.grow();
        let mut display_capacity = display::bytes_hint();
        self.display.grow(&mut display_capacity);
        if self.attack_required > self.attack.len() {
            let _ = resize(
                &mut self.attack,
                self.attack_required,
                engine::selected_attack::WeatherAttack::default(),
            )
            .map_err(crate::allocation::io);
        }
        let _ = self
            .attack_metadata
            .reserve(self.attack.len())
            .map_err(crate::allocation::io);
        self.update_capacity_charge();
    }

    fn update_capacity_charge(&mut self) {
        self.capacity_charge.set(crate::raw_memory::Capacity {
            payload_vectors: crate::raw_memory::vector_bytes(&self.regions.bytes)
                + crate::raw_memory::vector_bytes(&self.provinces.bytes)
                + crate::raw_memory::vector_bytes(&self.clocks.bytes)
                + crate::raw_memory::vector_bytes(&self.attack)
                + self.display.allocated_bytes() as u64,
            auxiliary_vectors: self.attack_metadata.allocated_bytes() as u64,
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

    /// Apply owned exact clock records; never follow the retained native source.
    fn fill_clocks(
        &self,
        provinces: u32,
        rows: &mut [sweather_per_province::Row],
    ) -> Result<(), &'static str> {
        let array = self
            .array
            .ok_or("weather clock array unreadable or invalid")?;
        if array.count != provinces || rows.len() > provinces as usize {
            return Err("weather clock array differs from the province array");
        }
        for (index, row) in rows.iter_mut().enumerate() {
            let begin = index * sweather_per_province::CLOCK_RECORD_BYTES;
            let record = self
                .bytes
                .get(begin..begin + sweather_per_province::CLOCK_RECORD_BYTES)
                .ok_or("weather clock fields unreadable")?;
            let fields = sweather_per_province::CLOCK_FIELDS.len();
            sweather_per_province::fill_clock_record(
                record,
                self.copied_fields
                    .saturating_sub(index * fields)
                    .min(fields),
                row,
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct WeatherCopyReport {
    /// Fixed fixtures can report preflight shortages. A source-fault/capacity
    /// pair retains an explicit terminal-unavailable payload without retry.
    pub raw_ok: bool,
    pub copy_ticks: [u64; N],
    pub raw_bytes: [u64; N],
}

/// Acquisition-worker copy parameters, bundled to keep dispatch small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WeatherCopyContext {
    pub game_state: usize,
    pub base: usize,
    pub hour: i32,
    pub attack_guard: Result<(), &'static str>,
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
    base: usize,
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
    let vtable_start = log.take(memory, pointer, 8, M_W_ALL)?;
    job.manager = pointer;
    job.manager_authenticated =
        base.checked_add(cweather_manager::VTABLE_RVA)
            .is_some_and(|vtable| {
                u64::from_le_bytes(
                    log.bytes[vtable_start..vtable_start + 8]
                        .try_into()
                        .unwrap(),
                ) == vtable as u64
            });
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
    let clocks = provinces.and_then(|_| {
        header(
            log,
            memory,
            pointer.checked_add(cweather_manager::PROVINCE_CLOCKS),
            sweather_per_province::CLOCK_STRIDE,
            M_PROVINCES,
        )
    });
    job.regions
        .prepare(regions, sweather_per_region::RECORD_BYTES);
    job.provinces
        .prepare(provinces, sweather_per_province::RECORD_BYTES);
    job.clocks
        .prepare(clocks, sweather_per_province::CLOCK_RECORD_BYTES);
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
    const CLOCKS_LAYOUT: RecordLayout<'static> = RecordLayout::new(
        &sweather_per_province::CLOCK_FIELDS,
        sweather_per_province::CLOCK_RECORD_BYTES,
    )
    .expect("valid clock layout");
    let mut reads = read_accounting::table(TableId::WeatherRegions);
    job.regions
        .capture(memory, sweather_per_region::STRIDE, &REGIONS_LAYOUT);
    reads.select(TableId::WeatherProvinces);
    job.provinces
        .capture(memory, sweather_per_province::STRIDE, &PROVINCES_LAYOUT);
    let provinces_complete = !job.provinces.faulted(sweather_per_province::FIELDS.len());
    let log = &mut job.log;
    // Raw copying sees the full census, but typed output can admit a shorter
    // healthy prefix. Keep its display/clock inputs even if a later row failed;
    // the worker still decodes ordinary fields first and preserves that error.
    let mut display_complete = false;
    if let Some(arrays) = display {
        display_complete = true;
        for array in arrays {
            if log
                .take_bulk(memory, array.data, array.count as usize, M_PROVINCES)
                .is_none()
            {
                display_complete = false;
                break;
            }
        }
    }
    // The native clock has no row identity. Equal live counts prove its slot join.
    if display_complete
        && job.clocks.array.map(|array| array.count) == job.provinces.array.map(|array| array.count)
    {
        job.clocks
            .capture(memory, sweather_per_province::CLOCK_STRIDE, &CLOCKS_LAYOUT);
    }
    log.raw_bytes[0] += job.regions.copied_bytes as u64;
    log.raw_bytes[1] += (job.provinces.copied_bytes + job.clocks.copied_bytes) as u64;
    log.fault[0] |= job.regions.faulted(sweather_per_region::FIELDS.len());
    log.fault[1] |= !provinces_complete;
    log.fault[1] |= job
        .clocks
        .faulted(sweather_per_province::CLOCK_FIELDS.len());
}

/// Append numeric facts to the original successful region gathers. Only
/// selecting controls are decoded on acquisition; no native pointer reaches
/// row parsing on decode. A short append arena never falls back/retries.
fn capture_attack_append(memory: &impl Memory, base: usize, job: &mut WeatherJob) {
    let Some(array) = job.regions.array else {
        return;
    };
    job.attack_required = array.count as usize;
    job.attack
        .fill(engine::selected_attack::WeatherAttack::refused(
            job.attack_hour,
            "selected attack weather append unobserved",
        ));
    let Some(vtable) = base.checked_add(sweather_per_region::VTABLE_RVA) else {
        return;
    };
    for index in 0..array.count as usize {
        let Some(destination) = job.attack.get_mut(index) else {
            break;
        };
        let begin = index * sweather_per_region::RECORD_BYTES;
        let Some(record) = job
            .regions
            .bytes
            .get(begin..begin + sweather_per_region::RECORD_BYTES)
        else {
            break;
        };
        let copied = job
            .regions
            .copied_fields
            .saturating_sub(index * sweather_per_region::FIELDS.len())
            .min(sweather_per_region::FIELDS.len());
        if sweather_per_region::decode_record(index as u32, record, copied, vtable).is_err() {
            break;
        }
        *destination = match array.element(index as u32, sweather_per_region::STRIDE) {
            Some(pointer) => engine::selected_attack::WeatherAttack::capture_authenticated(
                memory,
                base,
                pointer,
                job.attack_hour,
                &mut job.attack_metadata,
            ),
            None => engine::selected_attack::WeatherAttack::refused(
                job.attack_hour,
                "selected attack weather region address invalid",
            ),
        };
    }
    job.attack_metadata.finish_native();
}

/// Acquire one refresh into its owned payload.
pub(crate) fn copy_weather_refresh_growing(
    memory: &impl Memory,
    ctx: WeatherCopyContext,
    job: &mut WeatherJob,
) -> WeatherCopyReport {
    job.worker_growth = true;
    copy_weather_refresh(memory, ctx, job)
}

pub(crate) fn copy_weather_refresh(
    memory: &impl Memory,
    ctx: WeatherCopyContext,
    job: &mut WeatherJob,
) -> WeatherCopyReport {
    let started = qpc();
    job.clear();
    if job.worker_growth {
        job.log.enable_worker_growth();
    }
    job.game_state = ctx.game_state;
    job.attack_hour = ctx.hour;
    job.attack_metadata.set_guard(ctx.attack_guard);
    let display = prepare_weather_pass(memory, ctx.game_state, ctx.base, job);
    let display_bytes = display.map_or(0, |arrays| {
        arrays.iter().map(|array| array.count as usize).sum()
    });
    if job.worker_growth {
        // 140F07550/140F04E70 walk every live province/region. Continue
        // from the headers already acquired; no source read is repeated.
        job.regions.grow();
        job.provinces.grow();
        job.clocks.grow();
        job.attack_required = job.regions.array.map_or(0, |array| array.count as usize);
        let _ = engine::walk::Buffer::Growing(&mut job.attack).ensure(job.attack_required);
        let _ = job
            .attack_metadata
            .reserve(job.attack_required)
            .map_err(crate::allocation::io);
        if crate::allocation::failed() {
            job.staging_failure = Some("weather capture allocation failed");
            return WeatherCopyReport {
                raw_ok: true,
                copy_ticks: [elapsed(started), 0],
                raw_bytes: job.log.raw_bytes,
            };
        }
        job.update_capacity_charge();
    }
    job.log.reserve(display_bytes);
    job.log.overflowed |=
        job.regions.needs_growth() || job.provinces.needs_growth() || job.clocks.needs_growth();
    let overflow = job.log.overflowed;
    let root_fault = job.log.fault.iter().any(|&fault| fault);
    // Keep a failed source terminal when storage is also exhausted; retrying
    // the typed path would otherwise observe the failed native source again.
    if overflow && root_fault {
        job.staging_failure = Some("weather staging unavailable after source read failure");
    }
    if !overflow {
        copy_weather_dense(memory, job, display);
        capture_attack_append(memory, ctx.base, job);
        let _reads = read_accounting::table(TABLES[1]);
        if job.manager_authenticated {
            let grid = cweather_manager::read_display_grid(memory, ctx.base, job.manager);
            if job.worker_growth {
                job.display.capture_growing(memory, grid);
            } else {
                job.display.capture(memory, grid);
            }
            if job.display.header.is_ok() {
                job.log.raw_bytes[1] += job.display.required as u64;
            }
        }
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
        if job.worker_growth {
            let rows = job.regions.array.map_or(0, |array| array.count as usize);
            let provinces = job.provinces.array.map_or(0, |array| array.count as usize);
            let grown = (|| {
                engine::walk::Buffer::Growing(&mut self.regions.rows).ensure(rows)?;
                engine::walk::Buffer::Growing(&mut self.provinces.rows).ensure(provinces)?;
                engine::walk::Buffer::Growing(&mut self.attack.first).ensure(rows)?;
                self.attack_metadata
                    .reserve(rows)
                    .map_err(|_| "weather attack metadata allocation failed")?;
                let mut display_bytes = job.display.required;
                self.display.grow(&mut display_bytes);
                Ok::<(), &'static str>(())
            })();
            if let Err(reason) = grown {
                self.fail(reason);
                return;
            }
        }
        self.display.copy_from(&job.display);
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
        self.attack_hour = job.attack_hour;
        self.attack
            .first
            .fill(engine::selected_attack::WeatherAttack::refused(
                job.attack_hour,
                "selected attack weather append capacity exhausted",
            ));
        self.attack.required = self.regions.outcome.len();
        self.attack.len = self.attack.required.min(self.attack.first.len());
        for (index, destination) in self.attack.first[..self.attack.len].iter_mut().enumerate() {
            *destination = job
                .attack
                .get(index)
                .copied()
                .filter(|_| index < job.attack_required)
                .unwrap_or_else(|| {
                    engine::selected_attack::WeatherAttack::refused(
                        job.attack_hour,
                        "selected attack weather append capacity exhausted",
                    )
                });
        }
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
                    )?;
                    let clocks = reader.province_clocks()?;
                    if clocks.count != array.count {
                        return Err("weather clock array differs from the province array");
                    }
                    job.clocks
                        .fill_clocks(array.count, &mut self.provinces.rows[..len])
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
