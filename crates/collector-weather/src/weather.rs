use crate::{
    clock::qpc,
    raw::RawLog,
    read_accounting,
    table_capture::CaptureRows,
    table_conversion::{table, table_iter},
};
use engine_allocation::{filled, owned_vec, resize};
use engine_native_memory::{Memory, memory::{RecordLayout, RecordSources}, walk::Outcome};
use engine_native::cweather_manager::{
    self,
    province_weather as sweather_per_province,
    region_weather as sweather_per_region,
};
use replay_data::{rows, rows::TableId, rows::tables};
use replay_frame::frame::TableView;

mod air_efficiency;
pub mod carry;
mod display;
pub mod entries;
pub mod temperature;
use display::DisplayCapture;

pub fn observe_display_capacity(memory: &impl Memory, base: usize) -> Result<usize, &'static str> {
    display::observe_capacity(memory, base)
}

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::WeatherRegions, TableId::WeatherProvinces];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub display_bytes: usize,
}

collector_primitives::capacity_max!(Capacities {
    rows,
    display_bytes
});

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [0; N],
            display_bytes: display::bytes_hint(),
        }
    }
}

impl Capacities {
    pub fn presize(
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
    display: DisplayCapture,
    clocks_updated: bool,
    daylight_direction: Option<[i64; 3]>,
    daylight_feather: Option<[i64; 2]>,
    temperature_anchor: Option<engine_allocation::SharedBox<Vec<replay_data::temperature::State>>>,
    temperature_pass: Option<rows::TemperaturePass>,
    temperature_read: Vec<replay_data::temperature::State>,
    combat: Vec<Option<RegionModifiers>>,
    air_efficiency: Vec<air_efficiency::Tree>,
    province_modifiers: ProvinceModifiers,
    province_statics: Option<engine_allocation::SharedBox<ProvinceStatics>>,
    custom_sea: entries::CustomSea,
    province_window: Option<(u32, u32)>,
    display_snow: Vec<u8>,
    display_mud: Vec<u8>,
    pub raw_mode: bool,
}

impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; N],
            display_bytes: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            regions: CaptureRows::new(capacity.rows[0]),
            provinces: CaptureRows::new(capacity.rows[1]),
            display: DisplayCapture::new(capacity.display_bytes),
            clocks_updated: false,
            daylight_direction: None,
            daylight_feather: None,
            temperature_anchor: None,
            temperature_pass: None,
            temperature_read: Vec::new(),
            combat: Vec::new(),
            air_efficiency: Vec::new(),
            province_modifiers: ProvinceModifiers::default(),
            province_statics: None,
            custom_sea: Default::default(),
            province_window: None,
            display_snow: Vec::new(),
            display_mud: Vec::new(),
            raw_mode: false,
        }
    }

    pub fn reset(&mut self) {
        self.regions.reset();
        self.provinces.reset();
        self.display.clear();
        self.clocks_updated = false;
        self.daylight_direction = None;
        self.daylight_feather = None;
        self.temperature_pass = None;
        self.combat.clear();
        self.air_efficiency.clear();
        self.province_modifiers.clear();
        self.province_statics = None;
        self.custom_sea.clear();
        self.province_window = None;
        self.display_snow.clear();
        self.display_mud.clear();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.regions.allocated_bytes()
            + self.provinces.allocated_bytes()
            + self.display.allocated_bytes()
            + self.temperature_read.capacity() * size_of::<replay_data::temperature::State>()
            + self.combat.capacity() * size_of::<Option<RegionModifiers>>()
            + air_efficiency::allocated_bytes(&self.air_efficiency)
            + self.province_modifiers.allocated_bytes()
            + self.custom_sea.allocated_bytes()
            + self.display_snow.capacity()
            + self.display_mud.capacity()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.regions.grow(&mut capacity.rows[0]);
        self.provinces.grow(&mut capacity.rows[1]);
        self.display.grow(&mut capacity.display_bytes);
    }

    fn fail(&mut self, reason: &'static str) {
        self.display.clear();
        self.regions.outcome = Outcome::Unavailable { reason };
        self.provinces.outcome = Outcome::Unavailable { reason };
    }

    pub fn finish_owned_capture(&mut self) -> [u64; N] {
        self.reset();
        if !self.raw_mode {
            self.fail_weather_capture("weather acquisition not supplied");
        }
        [0; N]
    }

    #[cfg(any(test, feature = "test-support"))]
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
                self.$field.outcome = crate::table_capture::bounded(
                    reader.$field(&mut self.$field.rows),
                    self.$field.rows.len(),
                );
                ticks[$index] = ticks[$index].saturating_add(elapsed(begin));
            }};
        }
        primary!(0, regions);
        primary!(1, provinces);
        {
            let _reads = read_accounting::table(TABLES[1]);
            let begin = qpc();
            self.clocks_updated = reader.suspend_flags() == Ok([0, 0]);
            self.daylight_direction = (!self.clocks_updated)
                .then(|| reader.daylight_direction().ok())
                .flatten();
            self.daylight_feather = (!self.clocks_updated)
                .then(|| reader.daylight_feather().ok())
                .flatten();
            self.display.capture(memory, reader.display_grid());
            ticks[1] = ticks[1].saturating_add(elapsed(begin));
        }
        let words = reader
            .temperature_words()
            .ok()
            .map(|(cursor, window, seed)| temperature::Words {
                cursor,
                window,
                seed,
                gates: reader.suspend_flags().ok(),
            });
        if let Err(reason) = self.track_temperature(0, words, &mut temperature::Tracker::default())
        {
            self.fail(reason);
        }
        ticks
    }

    pub fn display_failure(&self) -> Option<&'static str> {
        (!self.provinces.outcome.is_empty())
            .then(|| self.display.header.err())
            .flatten()
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.regions.outcome, self.provinces.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        [
            self.region_table(),
            table::<tables::WeatherProvinces, _>(
                self.provinces.outcome,
                &self.provinces.rows,
                |row| self.province_row(row),
            ),
        ]
    }

    pub fn table_inputs(
        &self,
        projection: Option<&mut collector_projection::projection::Projection<'_>>,
    ) -> [replay_frame::frame::TableInput; N] {
        [
            self.region_table().into(),
            self.province_table_input(projection),
        ]
    }

    fn province_table_input(
        &self,
        projection: Option<&mut collector_projection::projection::Projection<'_>>,
    ) -> replay_frame::frame::TableInput {
        use replay_data::rows::RowCodec;
        let Some(projection) = projection else {
            return table::<tables::WeatherProvinces, _>(
                self.provinces.outcome,
                &self.provinces.rows,
                |row| self.province_row(row),
            )
            .into();
        };
        let mut held = projection.take_custom::<ProvinceFastState>(TableId::WeatherProvinces);
        if matches!(self.provinces.outcome, Outcome::Rows { len } if len > 0)
            && !replay_data::cadence::is_staggered(TableId::WeatherProvinces)
            && let Some(rows) = self.provinces.rows.get(..self.provinces.outcome.len())
            && self.display_snow.len() == rows.len()
            && self.display_mud.len() == rows.len()
            && let Some((first, visits)) = self.province_window
            && self.clocks_updated
            && self.temperature_pass.is_some()
            && let (Some(curr_anchor), Some(curr_statics)) =
                (&self.temperature_anchor, &self.province_statics)
            && curr_anchor.len() >= rows.len()
            && let Some((token, previous)) =
                projection.slice_baseline_rows::<tables::WeatherProvinces>()
            && previous.len() == rows.len()
            && let Some(state) = held.as_mut()
            && state.stamp == token.baseline()
            && state.clocks_updated
            && state.sequential
            && state.display_snow.len() == rows.len()
            && state.display_mud.len() == rows.len()
            && state
                .temperature_anchor
                .as_ref()
                .is_some_and(|prev| engine_allocation::SharedBox::ptr_eq(prev, curr_anchor))
            && state
                .province_statics
                .as_ref()
                .is_some_and(|prev| engine_allocation::SharedBox::ptr_eq(prev, curr_statics))
            && rows[0].id == 0
            && rows[rows.len() - 1].id as usize == rows.len() - 1
        {
            let input = projection.timed(TableId::WeatherProvinces, || {
                let len = rows.len();
                let candidates = &mut state.candidates;
                candidates.clear();
                let push_candidate = |list: &mut Vec<usize>, index: usize| -> std::io::Result<()> {
                    if list.len() == list.capacity() {
                        engine_allocation::reserve(list, 1)?;
                    }
                    list.push(index);
                    Ok(())
                };
                if push_candidate(candidates, 0).is_err() {
                    return crate::table_conversion::unavailable("collector allocation failed")
                        .into();
                }
                for offset in 0..visits as usize {
                    if push_candidate(candidates, (first as usize + offset) % len).is_err() {
                        return crate::table_conversion::unavailable("collector allocation failed")
                            .into();
                    }
                }
                if push_display_diffs(
                    &self.display_snow,
                    &state.display_snow,
                    &self.display_mud,
                    &state.display_mud,
                    candidates,
                )
                .is_err()
                {
                    return crate::table_conversion::unavailable("collector allocation failed")
                        .into();
                }
                candidates.sort_unstable();
                candidates.dedup();
                let mut keys = Vec::new();
                let mut changed = Vec::new();
                for &idx in candidates.iter() {
                    let row = &rows[idx];
                    let prev = &previous[idx];
                    if row.id != prev.id || !self.province_equal(row, prev) {
                        if keys.len() == keys.capacity()
                            && engine_allocation::reserve(&mut keys, 1).is_err()
                        {
                            return crate::table_conversion::unavailable(
                                "collector allocation failed",
                            )
                            .into();
                        }
                        if changed.len() == changed.capacity()
                            && engine_allocation::reserve(&mut changed, 1).is_err()
                        {
                            return crate::table_conversion::unavailable(
                                "collector allocation failed",
                            )
                            .into();
                        }
                        keys.push(u128::from(row.id));
                        let mut converted = self.province_row(row);
                        converted.round_to_display();
                        changed.push(converted);
                    }
                }
                state.stamp = projection.current_stamp();
                state.display_snow.copy_from_slice(&self.display_snow);
                state.display_mud.copy_from_slice(&self.display_mud);
                if keys.is_empty() {
                    projection.note_compared(TableId::WeatherProvinces, None);
                    replay_frame::frame::TableInput::ComparedEqual(token)
                } else {
                    projection.note_compared(TableId::WeatherProvinces, Some(changed.len()));
                    replay_frame::frame::TableInput::SliceUpdate {
                        token,
                        update: std::sync::Arc::new(replay_data::sliced::SliceUpdate::keyed(
                            keys,
                            rows::TableRows::WeatherProvinces(changed),
                        )),
                    }
                }
            });
            if matches!(
                input,
                replay_frame::frame::TableInput::ComparedEqual(_)
                    | replay_frame::frame::TableInput::SliceUpdate { .. }
            ) {
                projection.put_custom(TableId::WeatherProvinces, held.unwrap());
            }
            return input;
        }
        let input = collector_projection::projection::project_table::<tables::WeatherProvinces, _>(
            Some(projection),
            self.provinces.outcome,
            &self.provinces.rows,
            |row| u128::from(row.id),
            |row, previous| self.province_equal(row, previous),
            |row| self.province_row(row),
        );
        if matches!(
            input,
            replay_frame::frame::TableInput::ComparedEqual(_)
                | replay_frame::frame::TableInput::SliceUpdate { .. }
                | replay_frame::frame::TableInput::SharedComplete(_)
        ) && matches!(self.provinces.outcome, Outcome::Rows { len } if len > 0)
            && let Some(rows) = self.provinces.rows.get(..self.provinces.outcome.len())
            && self.display_snow.len() == rows.len()
            && self.display_mud.len() == rows.len()
        {
            let sequential = rows
                .iter()
                .enumerate()
                .all(|(index, row)| row.id as usize == index);
            if sequential {
                let mut state = match held {
                    Some(state) => state,
                    None => match engine_allocation::boxed(ProvinceFastState::default()) {
                        Ok(state) => state,
                        Err(_) => return input,
                    },
                };
                state.display_snow.clear();
                state.display_mud.clear();
                if engine_allocation::reserve(&mut state.display_snow, self.display_snow.len())
                    .is_ok()
                    && engine_allocation::reserve(&mut state.display_mud, self.display_mud.len())
                        .is_ok()
                {
                    state.display_snow.extend_from_slice(&self.display_snow);
                    state.display_mud.extend_from_slice(&self.display_mud);
                    state.stamp = projection.current_stamp();
                    state.clocks_updated = self.clocks_updated;
                    state.sequential = true;
                    state.temperature_anchor = self
                        .temperature_pass
                        .is_some()
                        .then(|| self.temperature_anchor.clone())
                        .flatten();
                    state.province_statics = self.province_statics.clone();
                    projection.put_custom(TableId::WeatherProvinces, state);
                }
            }
        }
        input
    }

    fn region_table(&self) -> TableView {
        let outcomes = self.outcomes();
        table_iter::<tables::WeatherRegions, _>(
            outcomes[0],
            self.regions.rows.iter().take(outcomes[0].len()).enumerate(),
            |(index, r)| rows::WeatherRegionRow {
                id: r.id,
                attack: rows::Recorded::Value(rows::Witnessed(None)),
                combat: rows::Recorded::Value(self.combat.get(index).copied().flatten().map(
                    |[
                        _detection,
                        _naval_detection,
                        speed,
                        _air_attack,
                        _air_defence,
                        attack,
                        breakthrough,
                        attack_speed,
                    ]| {
                        rows::RegionCombatWeather {
                            army_attack_factor: attack,
                            breakthrough_factor: breakthrough,
                            army_speed_factor: speed,
                            army_attack_speed_factor: attack_speed,
                        }
                    },
                )),
                air_detection: rows::Recorded::Value(
                    self.combat
                        .get(index)
                        .copied()
                        .flatten()
                        .map(|terms| terms[0]),
                ),
                map_modifiers_known: (self.combat.get(index).is_some_and(Option::is_some)).into(),
                naval_detection: (self
                    .combat
                    .get(index)
                    .copied()
                    .flatten()
                    .and_then(|terms| terms[1]))
                .into(),
                air_attack: (self
                    .combat
                    .get(index)
                    .copied()
                    .flatten()
                    .and_then(|terms| terms[3]))
                .into(),
                air_defence: (self
                    .combat
                    .get(index)
                    .copied()
                    .flatten()
                    .and_then(|terms| terms[4]))
                .into(),
                air_efficiency: rows::Recorded::Value(
                    self.air_efficiency
                        .get(index)
                        .and_then(Option::as_ref)
                        .map(std::sync::Arc::clone),
                ),
                custom_sea: rows::Recorded::Value(
                    self.custom_sea
                        .get(index)
                        .map(|values| owned_vec(engine_allocation::copy_vec(values))),
                ),
                max_snow: r.max_snow,
                light_rain: r.light_rain,
                heavy_rain: r.heavy_rain,
                snow: r.snow,
                blizzard: r.blizzard,
                sandstorm: r.sandstorm,
                arctic_water: r.arctic_water,
            },
        )
    }

    fn province_row(&self, r: &sweather_per_province::Row) -> rows::WeatherProvinceRow {
        let modifiers = self.province_modifiers.get(r.id as usize).map(|entries| {
            rows::ProvinceWeatherModifiers {
                entries: entries
                    .iter()
                    .map(|&(key, value)| rows::ProvinceModifierEntry { key, value })
                    .collect(),
            }
        });
        let static_modifiers =
            self.province_statics(r.id)
                .map(|list| rows::ProvinceStaticModifiers {
                    entries: entries::statics(list)
                        .map(|(name, days)| rows::ProvinceStaticModifier {
                            name: name.to_vec(),
                            days,
                        })
                        .collect(),
                });
        rows::WeatherProvinceRow {
            daylight: rows::Recorded::Value(self.province_daylight(r)),
            modifiers: rows::Recorded::Value(modifiers),
            static_modifiers: rows::Recorded::Value(static_modifiers),
            id: r.id,
            snow: r.snow,
            permanent_snow: r.permanent_snow,
            display_snow: r.display_snow,
            display_mud: r.display_mud,
            temperature: self.province_temperature(r),
            raw_mud: replay_data::rows::Recorded::Value(r.raw_mud),
            clock: self.province_clock(r),
            display: rows::Recorded::Value(if r.id == 0 { self.display.row() } else { None }),
            temperature_pass: rows::Recorded::Value(if r.id == 0 {
                self.temperature_pass.clone()
            } else {
                None
            }),
        }
    }

    fn province_statics(&self, slot: u32) -> Option<&[u8]> {
        self.province_statics.as_deref()?.get(slot as usize)
    }

    fn province_daylight(&self, row: &sweather_per_province::Row) -> Option<rows::WeatherDaylight> {
        (row.id == 0).then_some(rows::WeatherDaylight {
            updated: self.clocks_updated,
            held: self.daylight_direction,
            held_feather: self.daylight_feather,
        })
    }

    fn province_temperature(&self, row: &sweather_per_province::Row) -> rows::ProvinceTemperature {
        rows::ProvinceTemperature::anchored(match &self.temperature_pass {
            Some(_) => self
                .temperature_anchor
                .as_deref()
                .and_then(|anchor| anchor.get(row.id as usize))
                .copied()
                .unwrap_or_else(|| words_of(row)),
            None => words_of(row),
        })
    }

    fn province_clock(&self, row: &sweather_per_province::Row) -> rows::DayNightClock {
        if row.id != 0 && self.clocks_updated {
            rows::DayNightClock::UPDATED
        } else {
            rows::DayNightClock::held(replay_data::day_night::Clock {
                local_hour: row.local_hour,
                night_factor: row.night_factor,
            })
        }
    }

    fn province_equal(
        &self,
        row: &sweather_per_province::Row,
        previous: &rows::WeatherProvinceRow,
    ) -> bool {
        let rows::WeatherProvinceRow {
            id,
            snow,
            permanent_snow,
            display_snow,
            display_mud,
            raw_mud,
            temperature,
            clock,
            display,
            temperature_pass,
            modifiers,
            daylight,
            static_modifiers,
        } = previous;
        row.id == *id
            && row.snow == *snow
            && row.permanent_snow == *permanent_snow
            && row.display_snow == *display_snow
            && row.display_mud == *display_mud
            && *raw_mud == rows::Recorded::Value(row.raw_mud)
            && *temperature == self.province_temperature(row)
            && *clock == self.province_clock(row)
            && *daylight == rows::Recorded::Value(self.province_daylight(row))
            && matches!(temperature_pass, rows::Recorded::Value(value)
                if value.as_ref() == if row.id == 0 { self.temperature_pass.as_ref() } else { None })
            && matches!(display, rows::Recorded::Value(value)
                if if row.id == 0 { self.display.equal(value.as_ref()) } else { value.is_none() })
            && match (self.province_modifiers.get(row.id as usize), modifiers) {
                (None, rows::Recorded::Value(None)) => true,
                (Some(entries), rows::Recorded::Value(Some(previous))) => {
                    entries.len() == previous.entries.len()
                        && entries
                            .iter()
                            .zip(&previous.entries)
                            .all(|(&(key, value), previous)| {
                                key == previous.key && value == previous.value
                            })
                }
                _ => false,
            }
            && match (self.province_statics(row.id), static_modifiers) {
                (None, rows::Recorded::Value(None)) => true,
                (Some(list), rows::Recorded::Value(Some(previous))) => {
                    entries::statics(list).eq(previous
                        .entries
                        .iter()
                        .map(|entry| (&entry.name[..], entry.days)))
                }
                _ => false,
            }
    }

    fn track_temperature(
        &mut self,
        hour: i32,
        words: Option<temperature::Words>,
        tracker: &mut temperature::Tracker,
    ) -> Result<(), &'static str> {
        const ALLOCATION: &str = "weather temperature pass allocation failed";
        let complete = matches!(self.provinces.outcome, Outcome::Rows { .. });
        let words = words.filter(|_| complete);
        if words.is_some() {
            let rows = &self.provinces.rows[..self.provinces.outcome.len()];
            self.temperature_read.clear();
            engine_allocation::reserve(&mut self.temperature_read, rows.len())
                .map_err(|_| ALLOCATION)?;
            self.temperature_read.extend(rows.iter().map(words_of));
        }
        let read = words.map(|words| (words, &mut self.temperature_read));
        match tracker.observe(hour, read).map_err(|_| ALLOCATION)? {
            Some((anchor, pass)) => {
                let hours = engine_allocation::copy_vec(&pass.hours).map_err(|_| ALLOCATION)?;
                self.temperature_anchor = Some(anchor);
                self.temperature_pass = Some(rows::TemperaturePass {
                    anchor: pass.anchor,
                    cursor: pass.cursor,
                    anchor_seed: pass.anchor_seed,
                    hours,
                    witness: pass.witness,
                });
            }
            None => self.temperature_pass = None,
        }
        Ok(())
    }
}

#[derive(Default)]
struct ProvinceFastState {
    stamp: (u64, replay_data::BranchId, replay_data::Hour),
    clocks_updated: bool,
    sequential: bool,
    temperature_anchor: Option<engine_allocation::SharedBox<Vec<replay_data::temperature::State>>>,
    province_statics: Option<engine_allocation::SharedBox<ProvinceStatics>>,
    display_snow: Vec<u8>,
    display_mud: Vec<u8>,
    candidates: Vec<usize>,
}

fn push_display_diffs(
    snow: &[u8],
    prev_snow: &[u8],
    mud: &[u8],
    prev_mud: &[u8],
    out: &mut Vec<usize>,
) -> std::io::Result<()> {
    let chunks = snow.len() / 8;
    for chunk in 0..chunks {
        let base = chunk * 8;
        let a_s = u64::from_ne_bytes(snow[base..base + 8].try_into().unwrap());
        let b_s = u64::from_ne_bytes(prev_snow[base..base + 8].try_into().unwrap());
        let a_m = u64::from_ne_bytes(mud[base..base + 8].try_into().unwrap());
        let b_m = u64::from_ne_bytes(prev_mud[base..base + 8].try_into().unwrap());
        if (a_s ^ b_s) | (a_m ^ b_m) != 0 {
            for offset in 0..8 {
                let idx = base + offset;
                if snow[idx] != prev_snow[idx] || mud[idx] != prev_mud[idx] {
                    if out.len() == out.capacity() {
                        engine_allocation::reserve(out, 1)?;
                    }
                    out.push(idx);
                }
            }
        }
    }
    for idx in chunks * 8..snow.len() {
        if snow[idx] != prev_snow[idx] || mud[idx] != prev_mud[idx] {
            if out.len() == out.capacity() {
                engine_allocation::reserve(out, 1)?;
            }
            out.push(idx);
        }
    }
    Ok(())
}

fn fill_display_buffers(
    memory: &impl Memory,
    [snow, mud]: [engine_native_memory::cpdx_array::Array; 2],
    provinces: u32,
    display_snow: &mut Vec<u8>,
    display_mud: &mut Vec<u8>,
    rows: &mut [sweather_per_province::Row],
) -> Result<(), &'static str> {
    if snow.count != provinces || mud.count != provinces {
        return Err("weather display arrays differ from the province array");
    }
    let count = provinces as usize;
    if display_snow.len() < count {
        resize(display_snow, count, 0).map_err(|_| "weather display buffer allocation failed")?;
    }
    display_snow.truncate(count);
    if display_mud.len() < count {
        resize(display_mud, count, 0).map_err(|_| "weather display buffer allocation failed")?;
    }
    display_mud.truncate(count);
    if !memory.copy(snow.data, &mut display_snow[..count])
        || !memory.copy(mud.data, &mut display_mud[..count])
    {
        display_snow.clear();
        display_mud.clear();
        return Err("weather display bytes unreadable");
    }
    for ((row, &s), &m) in rows.iter_mut().zip(&display_snow[..count]).zip(&display_mud[..count]) {
        row.display_snow = s;
        row.display_mud = m;
    }
    Ok(())
}

fn words_of(row: &sweather_per_province::Row) -> replay_data::temperature::State {
    replay_data::temperature::State {
        temperature: row.temperature,
        drift: row.drift,
        staged: row.staged_temperature,
        working: row.working_drift,
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}


const M_W_ALL: u32 = 0x03;
const M_REGIONS: u32 = 0x01;
const M_PROVINCES: u32 = 0x02;

#[derive(Debug, Default)]
pub struct WeatherJob {
    pub log: RawLog<N>,
    pub game_state: usize,
    regions: DensePayload,
    provinces: DensePayload,
    clocks: DensePayload,
    capacity_charge: crate::raw_memory::Charge,
    staging_failure: Option<&'static str>,
    display: DisplayCapture,
    manager: usize,
    manager_authenticated: bool,
    combat: Vec<Option<RegionModifiers>>,
    air_efficiency: Vec<air_efficiency::Tree>,
    province_modifiers: ProvinceModifiers,
    province_scratch: Vec<entries::Entry>,
    province_statics: Option<engine_allocation::SharedBox<ProvinceStatics>>,
    static_headers: Vec<u8>,
    static_scratch: Vec<u8>,
    static_vtable: Option<usize>,
    custom_sea: entries::CustomSea,
    custom_bytes: Vec<u8>,
    custom_values: Vec<i16>,
    province_window: Option<(u32, u32)>,
    modifiers_windowed: bool,
    statics_held: bool,
    pub carry: Option<Box<carry::WeatherCarry>>,
}

impl WeatherJob {
    pub fn staging_failed(&self) -> bool {
        self.staging_failure.is_some()
    }

    pub fn new() -> Self {
        let mut job = Self {
            log: RawLog::new(0, 0),
            game_state: 0,
            regions: DensePayload::new(0),
            provinces: DensePayload::new(0),
            clocks: DensePayload::new(0),
            capacity_charge: Default::default(),
            staging_failure: None,
            display: DisplayCapture::new(display::bytes_hint()),
            manager: 0,
            manager_authenticated: false,
            combat: Vec::new(),
            air_efficiency: Vec::new(),
            province_modifiers: ProvinceModifiers::default(),
            province_scratch: Vec::new(),
            province_statics: None,
            static_headers: Vec::new(),
            static_scratch: Vec::new(),
            static_vtable: None,
            custom_sea: Default::default(),
            custom_bytes: Vec::new(),
            custom_values: Vec::new(),
            province_window: None,
            modifiers_windowed: false,
            statics_held: false,
            carry: None,
        };
        job.update_capacity_charge();
        job
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.regions.clear();
        self.provinces.clear();
        self.clocks.clear();
        self.staging_failure = None;
        self.display.clear();
        self.manager = 0;
        self.manager_authenticated = false;
        self.combat.clear();
        self.air_efficiency.clear();
        self.province_modifiers.clear();
        self.province_statics = None;
        self.static_vtable = None;
        self.custom_sea.clear();
        self.province_window = None;
        self.modifiers_windowed = false;
        self.statics_held = false;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next() || self.display.needs_growth()
    }

    pub fn grow(&mut self) {
        self.log.grow();
        let mut display_capacity = display::bytes_hint();
        self.display.grow(&mut display_capacity);
        self.update_capacity_charge();
    }

    fn update_capacity_charge(&mut self) {
        self.capacity_charge.set(crate::raw_memory::Capacity {
            payload_vectors: crate::raw_memory::vector_bytes(&self.regions.bytes)
                + crate::raw_memory::vector_bytes(&self.provinces.bytes)
                + crate::raw_memory::vector_bytes(&self.clocks.bytes)
                + self.display.allocated_bytes() as u64
                + air_efficiency::allocated_bytes(&self.air_efficiency) as u64
                + self.custom_sea.allocated_bytes() as u64
                + crate::raw_memory::vector_bytes(&self.static_headers)
                + crate::raw_memory::vector_bytes(&self.custom_bytes)
                + crate::raw_memory::vector_bytes(&self.custom_values),
            ..Default::default()
        });
    }
}

#[derive(Debug, Default)]
struct DensePayload {
    array: Option<engine_native_memory::cpdx_array::Array>,
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
    fn prepare(&mut self, array: Option<engine_native_memory::cpdx_array::Array>, record_bytes: usize) {
        self.array = array;
        self.required = array.map_or(0, |array| {
            (array.count as usize).saturating_mul(record_bytes)
        });
    }
    fn grow(&mut self) {
        if self.required > self.bytes.len() {
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
    fn decode<R: Default>(
        &self,
        vtable: Option<usize>,
        fields: usize,
        record_bytes: usize,
        output: &mut Vec<R>,
        invalid_array: &'static str,
        decode: impl Fn(u32, &[u8], usize, usize) -> Result<R, &'static str>,
    ) -> Outcome {
        let Some(array) = self.array else {
            return Outcome::Unavailable {
                reason: invalid_array,
            };
        };
        let Some(vtable) = vtable else {
            return Outcome::Unavailable {
                reason: "weather record vtable withheld",
            };
        };
        let count = array.count as usize;
        if let Err(reason) = engine_native_memory::walk::Buffer::Growing(output).ensure(count) {
            return Outcome::Unavailable { reason };
        }
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
        Outcome::Rows { len: count }
    }

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
pub struct WeatherCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; N],
    pub raw_bytes: [u64; N],
}

#[derive(Debug, Clone, Copy)]
pub struct WeatherCopyContext {
    pub game_state: usize,
    pub base: usize,
}

fn header(
    log: &mut RawLog<N>,
    memory: &impl Memory,
    address: Option<usize>,
    stride: usize,
    mask: u32,
) -> Option<engine_native_memory::cpdx_array::Array> {
    let start = log.take(memory, address?, 16, mask)?;
    engine_native_memory::cpdx_array::Array::from_header(
        log.bytes[start..start + 16].try_into().unwrap(),
        stride,
    )
}

fn prepare_weather_pass(
    memory: &impl Memory,
    game_state: usize,
    base: usize,
    job: &mut WeatherJob,
) -> Option<[engine_native_memory::cpdx_array::Array; 2]> {
    use engine_native::cgame_state;
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
        engine_sites::at(base, cweather_manager::VTABLE_RVA)
            .is_some_and(|vtable| {
                u64::from_le_bytes(
                    log.bytes[vtable_start..vtable_start + 8]
                        .try_into()
                        .unwrap(),
                ) == vtable as u64
            });
    if let Some(address) = pointer.checked_add(cweather_manager::MAX_SNOW) {
        let _ = log.take(memory, address, 8, M_REGIONS);
    }
    if let Some(address) = pointer.checked_add(cweather_manager::SUSPEND_FLAGS) {
        let updated = log
            .take(memory, address, 2, 0)
            .is_some_and(|start| log.bytes[start..start + 2] == [0, 0]);
        if !updated
            && job.manager_authenticated
            && let Some(direction) = pointer.checked_add(cweather_manager::DAYLIGHT_DIRECTION)
        {
            let _ = log.take(memory, direction, 24, 0);
            for offset in [
                engine_native_parts::air_fuel_defines::CONTEXT_SUBTRACT_RVA,
                engine_native_parts::air_fuel_defines::CONTEXT_SCALE_RVA,
            ] {
                if let Some(address) = engine_sites::at(base, offset) {
                    let _ = log.take(memory, address, 8, 0);
                }
            }
        }
    }
    for offset in [
        cweather_manager::UPDATE_CURSOR,
        cweather_manager::PROVINCES_PER_UPDATE,
        cweather_manager::SEED,
    ] {
        if let Some(address) = pointer.checked_add(offset) {
            let _ = log.take(memory, address, 4, 0);
        }
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
    display: Option<[engine_native_memory::cpdx_array::Array; 2]>,
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
    carry::start(job, memory);
    let mut reads = read_accounting::table(TableId::WeatherRegions);
    carry::capture_regions(memory, job, &REGIONS_LAYOUT);
    reads.select(TableId::WeatherProvinces);
    carry::capture_provinces(memory, job, &PROVINCES_LAYOUT);
    let provinces_complete = !job.provinces.faulted(sweather_per_province::FIELDS.len());
    let log = &mut job.log;
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

pub type RegionModifiers = [Option<i64>; 8];

const COMBAT_KEYS: [u32; 8] = [
    1,
    engine_executable::modifier_keys::NAVAL_DETECTION,
    engine_executable::modifier_keys::ARMY_SPEED_FACTOR,
    engine_executable::modifier_keys::AIR_ATTACK_FACTOR,
    engine_executable::modifier_keys::AIR_DEFENCE_FACTOR,
    engine_executable::modifier_keys::ARMY_ATTACK_FACTOR,
    engine_executable::modifier_keys::BREAKTHROUGH_FACTOR,
    engine_executable::modifier_keys::ARMY_ATTACK_SPEED_FACTOR,
];

pub use entries::{ProvinceModifiers, ProvinceStatics};

pub fn copy_weather_refresh(
    memory: &impl Memory,
    ctx: WeatherCopyContext,
    job: &mut WeatherJob,
) -> WeatherCopyReport {
    let started = qpc();
    job.clear();
    job.game_state = ctx.game_state;
    let display = prepare_weather_pass(memory, ctx.game_state, ctx.base, job);
    let display_bytes = display.map_or(0, |arrays| {
        arrays.iter().map(|array| array.count as usize).sum()
    });
    job.regions.grow();
    job.provinces.grow();
    job.clocks.grow();
    if crate::allocation::failed() {
        job.staging_failure = Some("weather capture allocation failed");
        return WeatherCopyReport {
            raw_ok: true,
            copy_ticks: [elapsed(started), 0],
            raw_bytes: job.log.raw_bytes,
        };
    }
    job.update_capacity_charge();
    job.log.reserve(display_bytes);
    let overflow = job.log.overflowed;
    let root_fault = job.log.fault.iter().any(|&fault| fault);
    if overflow && root_fault {
        job.staging_failure = Some("weather staging unavailable after source read failure");
    }
    if !overflow {
        copy_weather_dense(memory, job, display);
        {
            let _reads = read_accounting::table(TableId::WeatherRegions);
            carry::capture_combat(memory, job);
            carry::capture_custom_sea(memory, job);
            air_efficiency::capture(memory, ctx.base, job);
            job.update_capacity_charge();
        }
        {
            let _reads = read_accounting::table(TableId::WeatherProvinces);
            job.static_vtable =
                engine_sites::at(ctx.base, engine_native_parts::cstatic_modifier::VTABLE_RVA);
            carry::capture_province_modifiers(memory, job);
            carry::capture_province_statics(memory, job);
        }
        carry::finish(job);
        let _reads = read_accounting::table(TABLES[1]);
        if job.manager_authenticated {
            let grid = cweather_manager::read_display_grid(memory, ctx.base, job.manager);
            job.display.capture(memory, grid);
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
    pub fn fail_weather_capture(&mut self, reason: &'static str) {
        self.fail(reason);
        self.raw_mode = false;
    }

    pub fn decode_weather_payload(
        &mut self,
        base: usize,
        job: &WeatherJob,
        hour: i32,
        tracker: &mut temperature::Tracker,
        decode_ticks: &mut [u64; N],
    ) {
        let words = self.decode_weather_rows(base, job, decode_ticks);
        if let Err(reason) = self.track_temperature(hour, words, tracker) {
            self.fail(reason);
        }
    }

    fn decode_weather_rows(
        &mut self,
        base: usize,
        job: &WeatherJob,
        decode_ticks: &mut [u64; N],
    ) -> Option<temperature::Words> {
        self.raw_mode = false;
        let mut display_bytes = job.display.required;
        self.display.grow(&mut display_bytes);
        self.display.copy_from(&job.display);
        if let Some(reason) = job.staging_failure {
            self.fail(reason);
            return None;
        }
        let recorded = match crate::raw::Recorded::build(&job.log) {
            Ok(recorded) => recorded,
            Err(reason) => {
                self.fail(reason);
                return None;
            }
        };
        let started = qpc();
        let reader = match cweather_manager::Reader::new(&recorded, base, job.game_state) {
            Ok(reader) => reader,
            Err(reason) => {
                self.fail(reason);
                decode_ticks[0] += elapsed(started);
                return None;
            }
        };
        if !self.province_modifiers.copy_from(&job.province_modifiers) {
            self.province_modifiers.clear();
        }
        self.province_statics = job.province_statics.clone();
        self.regions.outcome = match reader.max_snow() {
            Ok(max_snow) => {
                let outcome = job.regions.decode(
                    engine_sites::resolve(base, sweather_per_region::VTABLE_RVA).ok(),
                    sweather_per_region::FIELDS.len(),
                    sweather_per_region::RECORD_BYTES,
                    &mut self.regions.rows,
                    "weather region array unreadable or invalid",
                    sweather_per_region::decode_record,
                );
                for row in &mut self.regions.rows[..outcome.len()] {
                    row.max_snow = max_snow;
                }
                self.combat.clear();
                if engine_allocation::reserve_exact(&mut self.combat, job.combat.len()).is_ok() {
                    self.combat.extend_from_slice(&job.combat);
                }
                self.air_efficiency.clear();
                if engine_allocation::reserve_exact(
                    &mut self.air_efficiency,
                    job.air_efficiency.len(),
                )
                .is_ok()
                {
                    self.air_efficiency
                        .extend(job.air_efficiency.iter().cloned());
                }
                if !self.custom_sea.copy_from(&job.custom_sea) {
                    self.custom_sea.clear();
                }
                outcome
            }
            Err(reason) => Outcome::Unavailable { reason },
        };
        decode_ticks[0] += elapsed(started);
        let started = qpc();
        let outcome = job.provinces.decode(
            engine_sites::resolve(base, sweather_per_province::VTABLE_RVA).ok(),
            sweather_per_province::FIELDS.len(),
            sweather_per_province::RECORD_BYTES,
            &mut self.provinces.rows,
            "weather province array unreadable or invalid",
            sweather_per_province::decode_record,
        );
        self.provinces.outcome = match (outcome, job.provinces.array) {
            (Outcome::Unavailable { .. }, _) | (_, None) => {
                self.display_snow.clear();
                self.display_mud.clear();
                outcome
            }
            (_, Some(array)) => {
                let len = outcome.len();
                match reader.display_arrays().and_then(|arrays| {
                    fill_display_buffers(
                        &recorded,
                        arrays,
                        array.count,
                        &mut self.display_snow,
                        &mut self.display_mud,
                        &mut self.provinces.rows[..len],
                    )?;
                    let clocks = reader.province_clocks()?;
                    if clocks.count != array.count {
                        return Err("weather clock array differs from the province array");
                    }
                    job.clocks
                        .fill_clocks(array.count, &mut self.provinces.rows[..len])
                }) {
                    Ok(()) => outcome,
                    Err(reason) => Outcome::Unavailable { reason },
                }
            }
        };
        self.province_window = matches!(self.provinces.outcome, Outcome::Rows { .. })
            .then(|| {
                job.province_window
                    .filter(|_| job.modifiers_windowed && job.statics_held)
            })
            .flatten();
        self.clocks_updated = reader.suspend_flags() == Ok([0, 0]);
        self.daylight_direction = (!self.clocks_updated)
            .then(|| reader.daylight_direction().ok())
            .flatten();
        self.daylight_feather = (!self.clocks_updated)
            .then(|| reader.daylight_feather().ok())
            .flatten();
        decode_ticks[1] += elapsed(started);
        reader
            .temperature_words()
            .ok()
            .map(|(cursor, window, seed)| temperature::Words {
                cursor,
                window,
                seed,
                gates: reader.suspend_flags().ok(),
            })
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "weather/bench/tests.rs"]
mod bench;
