//! Native weather acquisition. Only growth and replay conversion allocate.
//!
//! Hourly weather raw-copy split (WeatherLimits, WeatherRegions,
//! WeatherProvinces): the game thread copies raw bytes only and the single
//! FIFO decode worker replays the existing typed walks against the recorded
//! spans, so rows, order, truncation and failure reasons are identical by
//! construction (see `crate::raw` for the mirror rules).

use crate::{
    raw::RawLog,
    sampling::qpc,
    spooling::table,
    table_capture::CaptureRows,
};
use engine::{Memory, cweather_manager, sweather_per_province, sweather_per_region, walk::Outcome};
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
// header (vtable, both ceilings, both array headers), then each dense array
// as one bulk span. No row is decoded on the game thread. The single FIFO
// decode worker replays the existing `cweather_manager::Reader` walks
// against the recorded spans (`crate::raw::Recorded`), so rows, order,
// truncation and failure reasons are identical by construction (see
// `crate::raw` for the mirror rules). Bulk spans serve the walk's scattered
// field gathers via `Recorded` containment.
//
// The mirror traverses structurally and records every discoverable span,
// never aborting except on overflow: validation, precedence and error scope
// are local parses, so the replay derives every outcome from the same
// recorded spans and later extras are harmless.
// ===========================================================================

/// Fault-attribution bits in family order: [limits, regions, provinces].
/// The manager header carries all three bits; array payloads carry one bit.
const M_W_REGIONS: u32 = 0x02;
const M_W_PROVINCES: u32 = 0x04;
const M_W_ALL: u32 = 0x07;

/// One hour of raw weather payload. Flat pools recycle capacities and the
/// steady state allocates on neither thread; growth happens on the decode
/// side only (see `crate::decode::recycle_weather_job`).
#[derive(Debug)]
pub(crate) struct WeatherJob {
    pub log: RawLog<3>,
    pub game_state: usize,
}

impl WeatherJob {
    pub fn new() -> Self {
        Self {
            // Late-campaign worst case is about 305 regions (~78 KiB) and
            // 13,414 provinces (~5.2 MiB); both pools start above that so
            // steady state never overflows (see the worst-case caps test).
            log: RawLog::new(1 << 14, 8 << 20),
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
pub(crate) struct WeatherCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
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

/// One structural copy pass: manager pointer, bulk manager header, both
/// dense arrays as bulk spans. Stops at the first typed abort point; later
/// extras are harmless.
fn copy_weather_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    log: &mut RawLog<3>,
) {
    use engine::{cgame_state, cpdx_array::Array, cweather_manager};
    let _ = base;
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
    let regions = match Array::from_header(header(cweather_manager::REGIONS), sweather_per_region::STRIDE) {
        Some(a) => a,
        None => return,
    };
    let provinces = match Array::from_header(
        header(cweather_manager::PROVINCES),
        sweather_per_province::STRIDE,
    ) {
        Some(a) => a,
        None => return,
    };
    if regions.count > 0 {
        let bytes = (regions.count as usize).saturating_mul(sweather_per_region::STRIDE);
        let _ = wsbulk(log, memory, Some(regions.data), bytes, M_W_REGIONS);
        if log.overflowed {
            return;
        }
    }
    if provinces.count > 0 {
        let bytes = (provinces.count as usize).saturating_mul(sweather_per_province::STRIDE);
        let _ = wsbulk(log, memory, Some(provinces.data), bytes, M_W_PROVINCES);
    }
}

impl Captured {
    /// Game-thread raw copy for an hourly refresh. Copies `first` always and
    /// `second` while any table still needs double-read checks, compares the
    /// per-table raw partitions, and records copy-only costs plus the
    /// existing checked/mismatches bookkeeping. Returns whether the payload
    /// is complete (`raw_ok`); on staging overflow the caller falls back to
    /// the typed walk and enqueues nothing.
    pub(crate) fn copy_weather_refresh(
        &mut self,
        memory: &impl Memory,
        ctx: WeatherCopyContext,
        first: &mut WeatherJob,
        second: Option<&mut WeatherJob>,
    ) -> WeatherCopyReport {
        let started = qpc();
        first.clear();
        first.game_state = ctx.game_state;
        copy_weather_pass(memory, ctx.base, ctx.game_state, &mut first.log);
        let mut ticks = [0u64; 3];
        ticks[0] = elapsed(started);
        let mut matched = [true; 3];
        if let Some(second_job) = second {
            let second_started = qpc();
            second_job.clear();
            second_job.game_state = ctx.game_state;
            copy_weather_pass(memory, ctx.base, ctx.game_state, &mut second_job.log);
            ticks[0] += elapsed(second_started);
            let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
            for (slot, matched) in matched.iter_mut().enumerate() {
                let equal =
                    raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
                *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
            }
        }
        self.limits.checked = ctx.needed[0];
        self.limits.mismatches = usize::from(ctx.needed[0] && !matched[0]);
        self.regions.checked = ctx.needed[1];
        self.regions.mismatches = usize::from(ctx.needed[1] && !matched[1]);
        self.provinces.checked = ctx.needed[2];
        self.provinces.mismatches = usize::from(ctx.needed[2] && !matched[2]);
        let raw_ok = !first.log.overflowed;
        WeatherCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: first.log.raw_bytes,
        }
    }

    /// Decode-worker interpretation of a raw hourly payload into the pooled
    /// typed rows. Outcomes match the typed walk exactly because the
    /// identical walk runs against the recorded spans; checked/mismatches
    /// set by the raw compare survive. Clears `raw_mode` so the next capture
    /// starts clean.
    pub(crate) fn decode_weather_payload(
        &mut self,
        base: usize,
        job: &WeatherJob,
        decode_ticks: &mut [u64; 3],
    ) {
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
        self.regions.outcome =
            bounded(reader.regions(&mut self.regions.rows), self.regions.rows.len());
        decode_ticks[1] += elapsed(started);
        let started = qpc();
        self.provinces.outcome = bounded(
            reader.provinces(&mut self.provinces.rows),
            self.provinces.rows.len(),
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
        let report = captured.copy_weather_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        captured.raw_mode = report.raw_ok;
        let mut ticks = [0u64; 3];
        captured.decode_weather_payload(0, &first, &mut ticks);
        (captured, report)
    }

    fn assert_matches_direct(split: &Captured, direct: &Captured) {
        assert_eq!(split.limits.outcome, direct.limits.outcome, "limits outcome");
        assert_eq!(split.regions.outcome, direct.regions.outcome, "regions outcome");
        assert_eq!(
            split.provinces.outcome, direct.provinces.outcome,
            "provinces outcome"
        );
        let len = split.limits.outcome.len();
        assert_eq!(&split.limits.rows[..len], &direct.limits.rows[..len], "limits rows");
        let len = split.regions.outcome.len();
        assert_eq!(&split.regions.rows[..len], &direct.regions.rows[..len], "regions rows");
        let len = split.provinces.outcome.len();
        assert_eq!(
            &split.provinces.rows[..len],
            &direct.provinces.rows[..len],
            "provinces rows"
        );
    }

    #[test]
    fn weather_split_copy_matches_direct_walk() {
        let capacities = split_capacities();
        let bytes = fixture();
        let memory = SliceMemory { base: 0x1000, bytes: &bytes };
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
        let memory = SliceMemory { base: 0x1000, bytes: &bytes };
        let mut direct = Captured::with_capacities(&capacities, &[0; N]);
        let (matched, _) = direct.capture(&memory, 0, 0x1000, &[true; N]);
        assert_eq!(matched, [true, true, false]);
        let mut split = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = WeatherCopyContext { base: 0, game_state: 0x1000, needed: [true; N] };
        let mut first = WeatherJob::new();
        let mut second = WeatherJob::new();
        let report = split.copy_weather_refresh(&memory, ctx, &mut first, Some(&mut second));
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
        put(&mut bytes, 0x1000 + cgame_state::WEATHER_MANAGER, &0_u64.to_le_bytes());
        let memory = SliceMemory { base: 0x1000, bytes: &bytes };
        let mut direct = Captured::with_capacities(&capacities, &[0; N]);
        let (matched, _) = direct.capture(&memory, 0, 0x1000, &[true; N]);
        assert_eq!(matched, [false; N]);
        let mut split = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = WeatherCopyContext { base: 0, game_state: 0x1000, needed: [true; N] };
        let mut first = WeatherJob::new();
        let mut second = WeatherJob::new();
        let report = split.copy_weather_refresh(&memory, ctx, &mut first, Some(&mut second));
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
        let memory = SliceMemory { base: 0x1000, bytes: &bytes };
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = WeatherCopyContext { base: 0, game_state: 0x1000, needed: [true; N] };
        let mut first = WeatherJob::new();
        let mut second = WeatherJob::new();
        let report = captured.copy_weather_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 3]);
        let mut torn = fixture();
        put(&mut torn, 0x4000 + sweather_per_province::WATER, &9_u64.to_le_bytes());
        let mut after = WeatherJob::new();
        copy_weather_pass(&memory, 0, 0x1000, &mut after.log);
        let mut after_torn = WeatherJob::new();
        copy_weather_pass(
            &SliceMemory { base: 0x1000, bytes: &torn },
            0,
            0x1000,
            &mut after_torn.log,
        );
        assert!(!after.log.logs_equal(&after_torn.log));
        assert!(!after.log.partition_equal(&after_torn.log, 2));
        assert!(after.log.partition_equal(&after_torn.log, 1));
    }

    #[test]
    fn weather_partition_equal_isolates_provinces_private_bytes() {
        let capacities = split_capacities();
        let bytes = fixture();
        let memory = SliceMemory { base: 0x1000, bytes: &bytes };
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = WeatherCopyContext { base: 0, game_state: 0x1000, needed: [true; N] };
        let mut first = WeatherJob::new();
        let mut second = WeatherJob::new();
        let report = captured.copy_weather_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let provinces_idx = second
            .log
            .spans
            .iter()
            .position(|span| span.ok && span.mask == M_W_PROVINCES)
            .expect("provinces-private span");
        let start = second.log.spans[provinces_idx].start as usize;
        second.log.bytes[start] ^= 0xff;
        assert!(first.log.partition_equal(&second.log, 0));
        assert!(first.log.partition_equal(&second.log, 1));
        assert!(!first.log.partition_equal(&second.log, 2));
    }

    #[test]
    fn weather_staging_overflow_falls_back_to_typed_walk() {
        let capacities = split_capacities();
        let bytes = fixture();
        let memory = SliceMemory { base: 0x1000, bytes: &bytes };
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = WeatherCopyContext { base: 0, game_state: 0x1000, needed: [true; N] };
        let mut tiny = WeatherJob { log: crate::raw::RawLog::new(8, 8), game_state: 0 };
        let report = captured.copy_weather_refresh(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
        assert!(tiny.log.overflowed);
        assert!(tiny.grow_for_next());
    }

    #[test]
    fn weather_gate_skips_typed_walks_when_raw_mode_set() {
        let (_, mut captured) = small();
        captured.raw_mode = true;
        let bytes = fixture();
        let memory = SliceMemory { base: 0x1000, bytes: &bytes };
        let (matched, ticks) = captured.capture(&memory, 0, 0x1000, &[true; N]);
        assert_eq!(matched, [false; N]);
        assert_eq!(ticks, [0; N]);
    }

    #[test]
    fn weather_truncation_matrix_matches_direct_with_same_caps() {
        let bytes = fixture();
        let memory = SliceMemory { base: 0x1000, bytes: &bytes };
        for rows in [[0, 0, 0], [1, 1, 1], [2, 8, 8], [8, 8, 8]] {
            let capacities = Capacities { rows };
            let mut direct = Captured::with_capacities(&capacities, &[0; N]);
            let _ = direct.capture(&memory, 0, 0x1000, &[true; N]);
            let mut split = Captured::with_capacities(&capacities, &[0; N]);
            let ctx = WeatherCopyContext { base: 0, game_state: 0x1000, needed: [true; N] };
            let mut first = WeatherJob::new();
            let mut second = WeatherJob::new();
            let report = split.copy_weather_refresh(&memory, ctx, &mut first, Some(&mut second));
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
        assert!(job.log.spans.capacity() >= 8192);
        assert!(job.log.bytes.capacity() >= (8 << 20));
        let capacities = Capacities::default();
        assert!(capacities.rows[1] >= 512);
        assert!(capacities.rows[2] >= 20_000);
    }

    #[test]
    fn weather_bulk_manager_serves_enclosed_ceiling_word() {
        let bytes = fixture();
        let memory = SliceMemory { base: 0x1000, bytes: &bytes };
        let mut job = WeatherJob::new();
        copy_weather_pass(&memory, 0, 0x1000, &mut job.log);
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
