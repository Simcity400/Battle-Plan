//! Province temperatures replayed from recorded inputs and the recorded content stack,
//! with no memory access: the signed fixed5 temperature `province weather+0x118` that the
//! province tooltip and the save serializer read.
//!
//! Each hour that the weather update finds both suspension gates (`+0x349`/`+0x34A`)
//! clear, it adds a synchronized 31-bit random integer to the seed `+0x308` and runs one
//! pass over a rotating window: from the cursor `+0x58`, `provinces_per_update`
//! (`+0x2F0`) consecutive slots, wrapping at the province count `+0x1C`, of which the
//! active ones (`+0x28`) update ([`Simulation::update`]). For each, the temperature pass
//! moves its working drift `+0x170` by a hash of its id and the seed, clamps it to
//! `max_temperature_change` and stages `+0x118 + drift` at `+0x160`, resetting a drift
//! that pushes further outside the date's weather-period range; the threaded smoothing
//! pass then moves the staged temperature and the drift toward the averages of the active
//! neighbours' committed temperature `+0x118` and drift `+0x128`. Only after every
//! staging job joined does the commit pass copy `+0x160`/`+0x170` into `+0x118`/`+0x128`
//! for each province that had an active neighbour (`+0x168`), so the staging order is
//! irrelevant and every thread count gives the same result.
//!
//! The seed is the only input of an hour the content cannot supply; a recording stores it
//! with the state of every province at an anchor hour ([`State`]), from which playback
//! steps forward. Every function preserves the game's integer arithmetic: 64-bit wrapping
//! products and sums, truncating division, the fixed-point division guard that stores
//! `0xFFFFFFFF`, and the date functions' 32-bit arithmetic.

use std::collections::BTreeMap;

use crate::rows::{Recorded, TemperatureRecord, WeatherProvinceRow};

/// `fixed5` scale.
pub const SCALE: i64 = 100_000;
/// What the native fixed-point division stores when its guard refuses the numerator.
const REFUSED: i64 = 0xffff_ffff;
/// The epoch subtracted by weather-period lookup and the date setters.
pub const EPOCH: i32 = 43_800_000;
/// Hours in the game's non-leap year.
const HOURS_PER_YEAR: i32 = 8_760;
const DAYS_PER_YEAR: i32 = 365;
/// Month lengths in the game's non-leap calendar.
const MONTH_LENGTHS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
/// Default date for both weather-period bounds: year one, 1 January, hour 0.
pub const PERIOD_DEFAULT_DATE: i32 = 43_808_760;
/// Default temperature range when a weather period names none.
pub const PERIOD_DEFAULT_TEMPERATURE: [i64; 2] = [-1_000_000, 3_500_000];

/// The division guard excludes the three sentinel numerator words.
fn divisible(numerator: i64) -> bool {
    (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

/// Average active neighbours' words with guarded fixed5 division of their sum by the
/// active count scaled to fixed5; `active` is positive.
pub fn average(sum: i64, active: i32) -> i64 {
    let denominator = i64::from(active).wrapping_mul(SCALE);
    if denominator == 0 || !divisible(sum) {
        REFUSED
    } else {
        sum.wrapping_mul(SCALE).wrapping_div(denominator)
    }
}

/// Add the fixed5-scaled difference toward the neighbours' average to a staged
/// temperature or working drift.
pub fn smooth(value: i64, target: i64, factor: i64) -> i64 {
    (target.wrapping_sub(value).wrapping_mul(factor) / SCALE).wrapping_add(value)
}

/// Normalize a weather-period bound to year one before setting its month and day:
/// subtract the epoch, truncate to years and add the hours needed to reach year one.
pub fn normalize_to_year_one(date: i32) -> i32 {
    let years = date.wrapping_sub(EPOCH) / HOURS_PER_YEAR;
    date.wrapping_add(1_i32.wrapping_sub(years).wrapping_mul(HOURS_PER_YEAR))
}

/// Days before zero-based `month`, summed from the month table. A month past twelve reads
/// past the table and is refused; a negative one adds nothing.
fn days_before(month: i32) -> Result<i32, &'static str> {
    if month > 12 {
        return Err("weather period month past the native month table");
    }
    Ok(MONTH_LENGTHS[..month.max(0) as usize].iter().sum())
}

/// `|y|` as the date setters compute it: `-y`, or `y` when `-y` is negative.
fn magnitude(value: i32) -> i32 {
    let negated = value.wrapping_neg();
    if negated < 0 { value } else { negated }
}

/// `((y >> 31) & -2) + 1`: one for a nonnegative year, minus one otherwise.
fn year_sign(years: i32) -> i32 {
    ((years >> 31) & -2).wrapping_add(1)
}

/// Zero-based month from the date's day of year.
fn month_zero_based(date: i32) -> i32 {
    let day = (date.wrapping_sub(EPOCH) / 24) % DAYS_PER_YEAR;
    let mut month = 0;
    let mut rest = day.wrapping_sub(MONTH_LENGTHS[0]);
    while rest >= 0 && month < 11 {
        month += 1;
        rest = rest.wrapping_sub(MONTH_LENGTHS[month as usize]);
    }
    month
}

/// Shift the date by the difference between the target month's start and its current
/// month's, keeping the day within the month.
pub fn set_month_zero_based(date: i32, month: i32) -> Result<i32, &'static str> {
    let target = days_before(month)?;
    let days = date.wrapping_sub(EPOCH) / 24;
    let years = days / DAYS_PER_YEAR;
    let current = days_before(month_zero_based(date))?;
    let shifted = magnitude(years)
        .wrapping_sub(years)
        .wrapping_mul(DAYS_PER_YEAR)
        .wrapping_sub(current)
        .wrapping_add(days)
        .wrapping_add(target)
        .wrapping_mul(year_sign(years))
        .wrapping_sub(days);
    Ok(date.wrapping_add(shifted.wrapping_mul(24)))
}

/// Set the zero-based day within the date's current month, without clamping it to the
/// month's length.
pub fn set_day_of_month_zero_based(date: i32, day: i32) -> Result<i32, &'static str> {
    let hours = date.wrapping_sub(EPOCH);
    let days = hours / 24;
    let within = days
        .wrapping_add((hours / HOURS_PER_YEAR).wrapping_mul(-DAYS_PER_YEAR))
        .wrapping_sub(MONTH_LENGTHS[0]);
    // The native loop counts the months `within` passes and sums their lengths.
    let mut passed = 0;
    let mut rest = within;
    if rest >= 0 {
        loop {
            passed += 1;
            if passed >= MONTH_LENGTHS.len() {
                return Err("weather period day past the native month table");
            }
            rest = rest.wrapping_sub(MONTH_LENGTHS[passed]);
            if rest < 0 {
                break;
            }
        }
    }
    let start: i32 = MONTH_LENGTHS[..passed].iter().sum();
    let years = days / DAYS_PER_YEAR;
    let offset = magnitude(years)
        .wrapping_mul(DAYS_PER_YEAR)
        .wrapping_sub(days)
        .wrapping_add(start)
        .wrapping_add(day);
    Ok(hours
        .wrapping_add(offset.wrapping_mul(24))
        .wrapping_mul(year_sign(years))
        .wrapping_add(EPOCH))
}

/// Apply a `between` bound `day.month` to its current date: normalize to year one, then
/// set the zero-based month from the second component and the zero-based day from the
/// first, both already converted by the decimal integer parser.
pub fn apply_bound(date: i32, day: i32, month: i32) -> Result<i32, &'static str> {
    set_day_of_month_zero_based(
        set_month_zero_based(normalize_to_year_one(date), month)?,
        day,
    )
}

/// The date compared with weather-period bounds: the hour's day of year in year one, at
/// hour 0, in 32-bit arithmetic.
pub fn period_date(hour: i32) -> i32 {
    let years = (hour.wrapping_sub(EPOCH) / 24) / DAYS_PER_YEAR;
    let shifted = hour.wrapping_add(
        1_i32
            .wrapping_sub(years)
            .wrapping_mul(DAYS_PER_YEAR)
            .wrapping_sub(1_825_000)
            .wrapping_mul(24),
    );
    shifted.wrapping_sub(shifted % 24).wrapping_add(EPOCH)
}

/// One `weather period` (stride `0xE0`) of a strategic region template's weather
/// (`+0xB0`, count `+0xBC`): inclusive bounds `+0x10`/`+0x28` and the temperature range
/// the `weather range` at `+0x38` holds (`+0x40`, `+0x48`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Period {
    pub start: i32,
    pub end: i32,
    pub minimum: i64,
    pub maximum: i64,
}

/// The first period, in template order, whose bounds hold the hour's [`period_date`].
pub fn find_period(periods: &[Period], hour: i32) -> Option<&Period> {
    let date = period_date(hour);
    periods
        .iter()
        .find(|period| period.start <= date && date <= period.end)
}

/// One `terrain_modifiers` `temperature` entry (`weather terrain modifier`, stride
/// `0x18`): the terrain's database index (`+0x88`) and the fixed5 degrees added to both
/// bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerrainModifier {
    pub terrain: i32,
    pub value: i64,
}

/// Settings loaded from `common/weather.txt`, every value zero when the file leaves it
/// unset (manager construction clears `+0x138..+0x308`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// `temperature_variation` (`+0x138`).
    pub variation: i64,
    /// `max_temperature_change` (`+0x140`).
    pub max_change: i64,
    /// `temperature_neighbor_smoothing` (`+0x148`).
    pub smoothing: i64,
    /// `temperature_change_neighbor_smoothing` (`+0x150`).
    pub change_smoothing: i64,
    /// `provinces_per_update` (`+0x2F0`).
    pub provinces_per_update: i32,
    /// The `temperature` kind's terrain modifiers (`+0x78`, count `+0x84`) in loaded order.
    pub terrain: Vec<TerrainModifier>,
}

/// Initialization inputs for one province slot of the weather array.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Province {
    /// `+0x28`: a positive id whose definition exists and has pixels.
    pub active: bool,
    /// `+0x10`: the slot of every neighbour record of the definition (`+0x70`), in order.
    pub neighbours: Vec<u32>,
    /// `province+0xC8`'s strategic region id, `None` for a null region.
    pub region: Option<u32>,
    /// `+0x2C`: the definition terrain's `terrain type+0x88`, `None` for -1; an error
    /// names what the content cannot settle, refused only when the province reaches the
    /// terrain lookup.
    pub terrain: Result<Option<i32>, String>,
}

/// Every content fact the hourly pass reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Content {
    /// By weather slot, the province id.
    pub provinces: Vec<Province>,
    /// Each strategic region's periods in template order, or why the content cannot settle them.
    pub periods: BTreeMap<u32, Result<Vec<Period>, String>>,
    pub settings: Settings,
}

impl Content {
    /// The period the temperature pass finds for `slot` at native date `hour`; `None`
    /// where it returns before touching the record: a null region (`province+0xC8`) or no
    /// period.
    pub fn period(&self, slot: u32, hour: i32) -> Result<Option<&Period>, String> {
        let province = self
            .provinces
            .get(slot as usize)
            .ok_or_else(|| format!("weather province {slot} outside the content"))?;
        let Some(region) = province.region else {
            return Ok(None);
        };
        match self.periods.get(&region) {
            Some(Ok(periods)) => Ok(find_period(periods, hour)),
            Some(Err(reason)) => Err(format!(
                "weather province {slot}: region {region}: {reason}"
            )),
            None => Err(format!(
                "weather province {slot}: region {region} has no recorded-content template"
            )),
        }
    }

    /// The sums of the active neighbours' words for smoothing, and their count.
    pub fn neighbour_sums(&self, slot: u32, states: &[State]) -> Result<(i64, i64, i32), String> {
        let (mut temperatures, mut drifts, mut active) = (0_i64, 0_i64, 0_i32);
        for &neighbour in &self.provinces[slot as usize].neighbours {
            let index = neighbour as usize;
            let (Some(other), Some(state)) = (self.provinces.get(index), states.get(index)) else {
                return Err(format!(
                    "weather province {slot}: neighbour {neighbour} outside the weather array"
                ));
            };
            if other.active {
                temperatures = temperatures.wrapping_add(state.temperature);
                drifts = drifts.wrapping_add(state.drift);
                active = active.wrapping_add(1);
            }
        }
        Ok((temperatures, drifts, active))
    }
}

/// The simulation words of one province record that the pass reads and writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct State {
    /// `+0x118`, the committed temperature.
    pub temperature: i64,
    /// `+0x128`, the committed drift neighbours average.
    pub drift: i64,
    /// `+0x160`, the staged temperature.
    pub staged: i64,
    /// `+0x170`, the working drift.
    pub working: i64,
}

/// The integer hash used by temperature drift and the raw-mud roll: hash the wrapped sum
/// of `id`, `pass` and `seed` to a value in `0..100000`.
pub fn roll(input: u32) -> i32 {
    let a = input.wrapping_mul(0x7eb8_da44);
    let a = ((a >> 8) ^ a).wrapping_add(0x10d8_365e);
    let first = ((a << 8) ^ a).wrapping_mul(0x6d22_f31b);
    let b = input.wrapping_mul(0x205a_d647);
    let b = ((b >> 8) ^ b).wrapping_add(0xb4f3_c315);
    let b = ((b << 8) ^ b).wrapping_mul(0x387a_9f3a);
    let c = ((first >> 8) ^ first).wrapping_sub(((b >> 8) ^ b).wrapping_mul(0x4ad6_85b3));
    let c = ((c >> 8) ^ c).wrapping_add(0x68e3_1da4);
    let c = ((c << 8) ^ c).wrapping_mul(0x1b56_c4e9);
    let c = ((c >> 8) ^ c).wrapping_mul(0x92d6_8ca2);
    ((((c >> 8) ^ c) & 0x7fff_ffff) % 100_000) as i32
}

/// How many slots, from `cursor` and wrapping, the update [`window`] lists visits, and the
/// cursor it leaves, without listing them.
pub fn window_span(cursor: i32, window: i32, count: i32) -> Result<(i32, i32), &'static str> {
    if cursor < 0 {
        return Err("weather update cursor negative");
    }
    if cursor >= count {
        return Ok((0, cursor));
    }
    let span = window.max(1).min(count);
    Ok((
        span,
        ((i64::from(cursor) + i64::from(span)) % i64::from(count)) as i32,
    ))
}

/// The slots an update starting at `cursor` visits, and the cursor it leaves, following
/// the rotating cursor `+0x58`: `window` slots (at least one), wrapping to zero at the
/// count and stopping early on returning to the start. A cursor at or past the count
/// visits nothing and stays; a negative one would index before the array and is refused.
pub fn window(cursor: i32, window: i32, count: i32) -> Result<(Vec<u32>, i32), &'static str> {
    if cursor < 0 {
        return Err("weather update cursor negative");
    }
    let mut slots = Vec::new();
    if cursor >= count {
        return Ok((slots, cursor));
    }
    let mut next = cursor;
    let mut remaining = window;
    loop {
        slots.push(next as u32);
        next += 1;
        if next == count {
            next = 0;
        }
        if remaining.wrapping_sub(1) <= 0 {
            break;
        }
        remaining -= 1;
        if next == cursor {
            break;
        }
    }
    Ok((slots, next))
}

/// FNV-1a over every slot's temperature in slot order, eight little-endian bytes each: the
/// recorded witness that a derived hour equals the read one.
pub fn checksum(temperatures: impl IntoIterator<Item = i64>) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for temperature in temperatures {
        for byte in temperature.to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

/// Every province record's simulation words at one hour, and the update cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Simulation {
    pub hour: i32,
    pub cursor: i32,
    pub states: Vec<State>,
}

impl Simulation {
    /// The words the slots hold at `hour`, read or recorded.
    pub fn new(hour: i32, cursor: i32, states: Vec<State>) -> Self {
        Self {
            hour,
            cursor,
            states,
        }
    }

    pub fn checksum(&self) -> u64 {
        checksum(self.states.iter().map(|state| state.temperature))
    }

    /// The next hour: the weather pass runs with the seed after its increment, or, for
    /// `None`, an hour whose suspended manager updated nothing.
    pub fn advance(&mut self, content: &Content, seed: Option<u32>) -> Result<(), String> {
        let hour = self
            .hour
            .checked_add(1)
            .ok_or("weather temperature hour overflow")?;
        if let Some(seed) = seed {
            self.update(content, hour, seed)?;
        }
        self.hour = hour;
        Ok(())
    }

    /// One ordinary update (`+0x348` clear, pass index zero) at native date `hour`.
    pub fn update(&mut self, content: &Content, hour: i32, seed: u32) -> Result<(), String> {
        let count = self.states.len();
        if content.provinces.len() != count {
            return Err(format!(
                "content has {} weather province slots, the recording {count}",
                content.provinces.len()
            ));
        }
        let count = i32::try_from(count).map_err(|_| "weather province count exceeds i32")?;
        let (slots, cursor) = window(self.cursor, content.settings.provinces_per_update, count)?;
        // update_passes collects the active slots of the window.
        let staged: Vec<(u32, State, bool)> = slots
            .into_iter()
            .filter(|&slot| content.provinces[slot as usize].active)
            .map(|slot| {
                self.stage(content, hour, seed, slot)
                    .map(|(state, commit)| (slot, state, commit))
            })
            .collect::<Result<_, _>>()?;
        // Commit temperatures only after every staging job joined.
        for (slot, mut state, commit) in staged {
            if commit {
                state.temperature = state.staged;
                state.drift = state.working;
            }
            self.states[slot as usize] = state;
        }
        self.cursor = cursor;
        Ok(())
    }

    /// Temperature drift and neighbour smoothing for one slot, reading the committed
    /// words of the hour before; whether `+0x168` is set for the commit.
    fn stage(
        &self,
        content: &Content,
        hour: i32,
        seed: u32,
        slot: u32,
    ) -> Result<(State, bool), String> {
        let province = &content.provinces[slot as usize];
        let settings = &content.settings;
        let mut state = self.states[slot as usize];
        if let Some(period) = content.period(slot, hour)? {
            let (mut minimum, mut maximum) = (period.minimum, period.maximum);
            let terrain = province
                .terrain
                .as_ref()
                .map_err(|reason| format!("weather province {slot}: {reason}"))?;
            if let Some(modifier) = terrain.and_then(|terrain| {
                settings
                    .terrain
                    .iter()
                    .find(|modifier| modifier.terrain == terrain)
            }) {
                minimum = minimum.wrapping_add(modifier.value);
                maximum = maximum.wrapping_add(modifier.value);
            }
            // id + pass index (zero) + seed, in 32-bit arithmetic.
            let roll = i64::from(roll(slot.wrapping_add(seed)));
            let step = (roll - 50_000).wrapping_mul(settings.variation) / SCALE;
            state.working = state.working.wrapping_add(step.wrapping_mul(2));
            let limit = settings.max_change;
            let lower = limit.wrapping_neg();
            if state.working < lower {
                state.working = lower;
            } else if state.working > limit {
                state.working = limit;
            }
            let clamped = state.working;
            let temperature = state.temperature.wrapping_add(clamped);
            if temperature < minimum {
                state.working = clamped.max(0);
            } else if temperature > maximum {
                state.working = clamped.min(0);
            }
            state.staged = temperature;
        }
        // Smoothing clears +0x168 and sets it only when an active neighbour exists.
        let (temperatures, drifts, active) = content.neighbour_sums(slot, &self.states)?;
        if active <= 0 {
            return Ok((state, false));
        }
        // Both averages divide by the active count times 100000, each behind its own guard.
        state.staged = smooth(
            state.staged,
            average(temperatures, active),
            settings.smoothing,
        );
        state.working = smooth(
            state.working,
            average(drifts, active),
            settings.change_smoothing,
        );
        Ok((state, true))
    }
}

/// Every province record's words at a frame's hour, as its WeatherProvinces rows establish
/// them, keeping the last derivation so that the next hour of the same anchor steps once.
#[derive(Debug, Default)]
pub struct Memo {
    branch: u64,
    /// The anchor hour and cursor of the kept derivation.
    anchor: Option<(i32, i32)>,
    anchored: Vec<State>,
    consumed: Vec<Option<u32>>,
    simulation: Option<Simulation>,
}

impl Memo {
    /// Every slot's words at the frame's hour. Formats 93 to 101 recorded each temperature at
    /// its hour, which this returns with no other word. From format 102 the rows hold the words
    /// at the frame's anchor and slot zero the pass since; the derivation
    /// steps through every recorded hour with the content and must reproduce the checksum of
    /// the temperatures the collector read at the frame's hour ([`crate::rows::TemperaturePass`]).
    pub fn states(
        &mut self,
        branch: u64,
        hour: i32,
        rows: &[WeatherProvinceRow],
        content: Result<&Content, &str>,
    ) -> Result<&[State], String> {
        let mut anchored = Vec::new();
        let mut recorded = Vec::new();
        for (slot, row) in rows.iter().enumerate() {
            if row.id as usize != slot {
                return Err("WeatherProvinces slots are not dense".into());
            }
            match row.temperature.record()? {
                TemperatureRecord::Anchored(state) => anchored.push(state),
                TemperatureRecord::AtHour(temperature) => recorded.push(State {
                    temperature,
                    ..State::default()
                }),
                TemperatureRecord::NotRecorded => {
                    return Err("WeatherProvinces temperature not recorded".into());
                }
            }
        }
        if !recorded.is_empty() {
            if !anchored.is_empty() {
                return Err("WeatherProvinces mixes anchored and hourly temperatures".into());
            }
            self.anchor = None;
            self.simulation = Some(Simulation::new(hour, 0, recorded));
            return Ok(&self.simulation.as_ref().unwrap().states);
        }
        let pass = match rows.first().map(|row| &row.temperature_pass) {
            Some(Recorded::Value(Some(pass))) => pass,
            Some(Recorded::Value(None)) => {
                return Err("the collector did not read this hour's temperature pass whole".into());
            }
            _ => return Err("WeatherProvinces temperature pass not recorded".into()),
        };
        if i64::from(hour) - i64::from(pass.anchor) != pass.hours.len() as i64 {
            return Err(format!(
                "the temperature pass records {} hours after its anchor {}, the frame is hour {hour}",
                pass.hours.len(),
                pass.anchor
            ));
        }
        let content = content.map_err(|reason| format!("temperature content: {reason}"))?;
        let reuse = self.branch == branch
            && self.anchor == Some((pass.anchor, pass.cursor))
            && self.anchored == anchored
            && pass.hours.starts_with(&self.consumed)
            && self.simulation.is_some();
        if !reuse {
            self.branch = branch;
            self.anchor = Some((pass.anchor, pass.cursor));
            self.consumed.clear();
            self.simulation = Some(Simulation::new(pass.anchor, pass.cursor, anchored.clone()));
            self.anchored = anchored;
        }
        for &entry in &pass.hours[self.consumed.len()..] {
            let simulation = self.simulation.as_mut().unwrap();
            if let Err(reason) = simulation.advance(content, entry) {
                self.anchor = None;
                self.simulation = None;
                return Err(reason);
            }
            self.consumed.push(entry);
        }
        let simulation = self.simulation.as_ref().unwrap();
        if simulation.checksum() != pass.checksum {
            return Err(format!(
                "derived temperatures at hour {hour} differ from the recorded checksum"
            ));
        }
        Ok(&simulation.states)
    }

    /// The kept derivation, at the hour of the last successful call.
    pub fn simulation(&self) -> Option<&Simulation> {
        self.simulation.as_ref()
    }
}

#[cfg(test)]
mod tests;
