use std::{borrow::Cow, collections::BTreeMap};

use crate::{
    ProvinceTemperature, Recorded, TemperaturePass, TemperatureRecord, WeatherProvinceRow,
};

pub trait ProvinceRows {
    fn len(&self) -> usize;
    fn row(&self, at: usize) -> Result<Option<(u32, ProvinceTemperature)>, String>;
    fn find(&self, id: u32) -> Option<usize>;
    fn pass(&self) -> Result<Option<Cow<'_, Recorded<Option<TemperaturePass>>>>, String>;
}

impl ProvinceRows for [WeatherProvinceRow] {
    fn len(&self) -> usize {
        <[WeatherProvinceRow]>::len(self)
    }

    fn row(&self, at: usize) -> Result<Option<(u32, ProvinceTemperature)>, String> {
        Ok(self.get(at).map(|row| (row.id, row.temperature)))
    }

    fn find(&self, id: u32) -> Option<usize> {
        self.binary_search_by_key(&id, |row| row.id).ok()
    }

    fn pass(&self) -> Result<Option<Cow<'_, Recorded<Option<TemperaturePass>>>>, String> {
        Ok(self.first().map(|row| Cow::Borrowed(&row.temperature_pass)))
    }
}

pub const SCALE: i64 = 100_000;
const REFUSED: i64 = 0xffff_ffff;
#[cfg(test)]
use weather_period::HOURS_PER_YEAR;
pub use weather_period::{
    EPOCH, PERIOD_DEFAULT_DATE, PERIOD_DEFAULT_TEMPERATURE, Period, apply_bound, find_period,
    normalize_to_year_one, period_date, set_day_of_month_zero_based, set_month_zero_based,
};

fn divisible(numerator: i64) -> bool {
    (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

pub fn average(sum: i64, active: i32) -> i64 {
    let denominator = i64::from(active).wrapping_mul(SCALE);
    if denominator == 0 || !divisible(sum) {
        REFUSED
    } else {
        sum.wrapping_mul(SCALE).wrapping_div(denominator)
    }
}

fn reciprocal(count: u32) -> (u64, u32) {
    if count == 0 {
        return (0, 0);
    }
    let shift = 31 + (32 - (count - 1).leading_zeros());
    let count = u128::from(count);
    ((((1_u128 << shift) + count - 1) / count) as u64, shift)
}

fn average_by(sum: i64, active: i32, (m, shift): (u64, u32)) -> i64 {
    let magnitude = sum.unsigned_abs();
    if m == 0 || magnitude >= 1 << 31 {
        return average(sum, active);
    }
    let quotient = ((magnitude * m) >> shift) as i64;
    if sum < 0 { -quotient } else { quotient }
}

pub fn smooth(value: i64, target: i64, factor: i64) -> i64 {
    (target.wrapping_sub(value).wrapping_mul(factor) / SCALE).wrapping_add(value)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerrainModifier {
    pub terrain: i32,
    pub value: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub variation: i64,
    pub max_change: i64,
    pub smoothing: i64,
    pub change_smoothing: i64,
    pub provinces_per_update: i32,
    pub terrain: Vec<TerrainModifier>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Province {
    pub active: bool,
    pub neighbours: Vec<u32>,
    pub region: Option<u32>,
    pub terrain: Result<Option<i32>, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Content {
    pub provinces: Vec<Province>,
    pub periods: BTreeMap<u32, Result<Vec<Period>, String>>,
    pub settings: Settings,
}

impl Content {
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct State {
    pub temperature: i64,
    pub drift: i64,
    pub staged: i64,
    pub working: i64,
}

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

pub fn window_span(cursor: i32, window: i32, count: i32) -> Result<(i32, i32), &'static str> {
    if cursor < 0 {
        return Err("weather update cursor negative");
    }
    if cursor >= count {
        return Ok((0, cursor));
    }
    let span = window_capacity(window, count as usize) as i32;
    Ok((
        span,
        ((i64::from(cursor) + i64::from(span)) % i64::from(count)) as i32,
    ))
}

fn window_capacity(width: i32, count: usize) -> usize {
    if width == i32::MIN {
        count
    } else {
        (width.max(1) as usize).min(count)
    }
}

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
        remaining = remaining.wrapping_sub(1);
        if next == cursor {
            break;
        }
    }
    Ok((slots, next))
}

pub fn witness(temperatures: impl IntoIterator<Item = i64>) -> u64 {
    temperatures
        .into_iter()
        .enumerate()
        .fold(0, |sum, (slot, temperature)| {
            sum.wrapping_add(province_witness(slot as u32, temperature))
        })
}

pub const fn province_witness(slot: u32, temperature: i64) -> u64 {
    mix(mix((slot as u64).wrapping_add(0x9e37_79b9_7f4a_7c15)) ^ temperature as u64)
}

const fn mix(mut word: u64) -> u64 {
    word = (word ^ (word >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    word = (word ^ (word >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    word ^ (word >> 31)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Simulation {
    pub hour: i32,
    pub cursor: i32,
    pub states: Vec<State>,
}

impl Simulation {
    pub fn new(hour: i32, cursor: i32, states: Vec<State>) -> Self {
        Self {
            hour,
            cursor,
            states,
        }
    }

    pub fn witness(&self) -> u64 {
        witness(self.states.iter().map(|state| state.temperature))
    }

    pub fn advance(&mut self, content: &Content, seed: Option<u32>) -> Result<(), String> {
        self.advance_with(content, seed, &mut Vec::new(), None, &mut None)
    }

    fn advance_with<P: Pass + ?Sized>(
        &mut self,
        pass: &P,
        seed: Option<u32>,
        staged: &mut Vec<(u32, State, bool)>,
        changed: Option<&mut Vec<(u32, i64, i64)>>,
        witness: &mut Option<u64>,
    ) -> Result<(), String> {
        let hour = self
            .hour
            .checked_add(1)
            .ok_or("weather temperature hour overflow")?;
        if let Some(seed) = seed {
            self.update_with(pass, hour, seed, staged, changed, witness)?;
        }
        self.hour = hour;
        Ok(())
    }

    pub fn update(&mut self, content: &Content, hour: i32, seed: u32) -> Result<(), String> {
        self.update_changed(content, hour, seed, None)
    }

    fn update_changed(
        &mut self,
        content: &Content,
        hour: i32,
        seed: u32,
        changed: Option<&mut Vec<(u32, i64, i64)>>,
    ) -> Result<(), String> {
        self.update_with(content, hour, seed, &mut Vec::new(), changed, &mut None)
    }

    fn update_with<P: Pass + ?Sized>(
        &mut self,
        pass: &P,
        hour: i32,
        seed: u32,
        staged: &mut Vec<(u32, State, bool)>,
        mut changed: Option<&mut Vec<(u32, i64, i64)>>,
        witness: &mut Option<u64>,
    ) -> Result<(), String> {
        let content = pass.content();
        let count = self.states.len();
        if content.provinces.len() != count {
            return Err(format!(
                "content has {} weather province slots, the recording {count}",
                content.provinces.len()
            ));
        }
        let count = i32::try_from(count).map_err(|_| "weather province count exceeds i32")?;
        let (span, cursor) =
            window_span(self.cursor, content.settings.provinces_per_update, count)?;
        let date = period_date(hour);
        staged.clear();
        staged
            .try_reserve(span as usize)
            .map_err(|error| format!("weather temperature pass: {error}"))?;
        let mut slot = self.cursor as u32;
        for _ in 0..span {
            if pass.active(slot) {
                let staging = pass.slot(slot, hour, date, &self.states)?;
                let (state, commit) = stage(
                    &content.settings,
                    self.states[slot as usize],
                    slot,
                    seed,
                    staging,
                );
                staged.push((slot, state, commit));
            }
            slot += 1;
            if slot == count as u32 {
                slot = 0;
            }
        }
        for &(slot, mut state, commit) in staged.iter() {
            if commit {
                state.temperature = state.staged;
                state.drift = state.working;
            }
            let before = self.states[slot as usize].temperature;
            if before != state.temperature {
                if let Some(witness) = witness.as_mut() {
                    *witness = witness
                        .wrapping_sub(province_witness(slot, before))
                        .wrapping_add(province_witness(slot, state.temperature));
                }
                if let Some(changed) = changed.as_deref_mut() {
                    changed
                        .try_reserve(1)
                        .map_err(|error| format!("weather temperature changes: {error}"))?;
                    changed.push((slot, before, state.temperature));
                }
            }
            self.states[slot as usize] = state;
        }
        self.cursor = cursor;
        Ok(())
    }
}

fn stage(
    settings: &Settings,
    mut state: State,
    slot: u32,
    seed: u32,
    (bounds, (temperatures, drifts, active), divisor): Staging,
) -> (State, bool) {
    if let Some((minimum, maximum)) = bounds {
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
    if active <= 0 {
        return (state, false);
    }
    state.staged = smooth(
        state.staged,
        average_by(temperatures, active, divisor),
        settings.smoothing,
    );
    state.working = smooth(
        state.working,
        average_by(drifts, active, divisor),
        settings.change_smoothing,
    );
    (state, true)
}

fn shifted(period: &Period, shift: Option<i64>) -> (i64, i64) {
    match shift {
        Some(value) => (
            period.minimum.wrapping_add(value),
            period.maximum.wrapping_add(value),
        ),
        None => (period.minimum, period.maximum),
    }
}

pub type Staging = (Option<(i64, i64)>, (i64, i64, i32), (u64, u32));

pub trait Pass {
    fn content(&self) -> &Content;

    fn active(&self, slot: u32) -> bool;

    fn slot(&self, slot: u32, hour: i32, date: i32, states: &[State])
    -> Result<Staging, String>;
}

impl Pass for Content {
    fn content(&self) -> &Content {
        self
    }

    fn active(&self, slot: u32) -> bool {
        self.provinces[slot as usize].active
    }

    fn slot(&self, slot: u32, hour: i32, _: i32, states: &[State]) -> Result<Staging, String> {
        let bounds = match self.period(slot, hour)? {
            Some(period) => {
                let terrain = self.provinces[slot as usize]
                    .terrain
                    .as_ref()
                    .map_err(|reason| format!("weather province {slot}: {reason}"))?;
                let shift = terrain.and_then(|terrain| {
                    self.settings
                        .terrain
                        .iter()
                        .find(|modifier| modifier.terrain == terrain)
                        .map(|modifier| modifier.value)
                });
                Some(shifted(period, shift))
            }
            None => None,
        };
        Ok((bounds, self.neighbour_sums(slot, states)?, (0, 0)))
    }
}

const NO_REGION: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
enum Shift {
    None,
    By(i64),
    Refused,
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    active: bool,
    broken: bool,
    region: u32,
    shift: Shift,
    first: u32,
    end: u32,
    divisor: (u64, u32),
}

#[derive(Clone, Debug, Default)]
pub struct Plan {
    slots: Vec<Slot>,
    neighbours: Vec<u32>,
    periods: Vec<Option<Vec<Period>>>,
}

impl Plan {
    pub fn new(content: &Content) -> Self {
        let count = content.provinces.len();
        let mut regions = BTreeMap::new();
        let mut periods = Vec::new();
        let mut neighbours = Vec::new();
        let slots = content
            .provinces
            .iter()
            .map(|province| {
                let region = match province.region {
                    None => NO_REGION,
                    Some(region) => *regions.entry(region).or_insert_with(|| {
                        periods.push(
                            content
                                .periods
                                .get(&region)
                                .and_then(|periods| periods.as_ref().ok())
                                .cloned(),
                        );
                        (periods.len() - 1) as u32
                    }),
                };
                let shift = match &province.terrain {
                    Err(_) => Shift::Refused,
                    Ok(terrain) => terrain
                        .and_then(|terrain| {
                            content
                                .settings
                                .terrain
                                .iter()
                                .find(|modifier| modifier.terrain == terrain)
                        })
                        .map_or(Shift::None, |modifier| Shift::By(modifier.value)),
                };
                let first = neighbours.len() as u32;
                let broken = province
                    .neighbours
                    .iter()
                    .any(|&neighbour| neighbour as usize >= count);
                if !broken {
                    neighbours.extend(
                        province
                            .neighbours
                            .iter()
                            .copied()
                            .filter(|&neighbour| content.provinces[neighbour as usize].active),
                    );
                }
                let end = neighbours.len() as u32;
                Slot {
                    active: province.active,
                    broken,
                    region,
                    shift,
                    first,
                    end,
                    divisor: reciprocal(end - first),
                }
            })
            .collect();
        Self {
            slots,
            neighbours,
            periods,
        }
    }

    pub fn bytes(&self) -> usize {
        self.slots.capacity() * size_of::<Slot>()
            + self.neighbours.capacity() * size_of::<u32>()
            + self.periods.capacity() * size_of::<Option<Vec<Period>>>()
            + self
                .periods
                .iter()
                .flatten()
                .map(|periods| periods.capacity() * size_of::<Period>())
                .sum::<usize>()
    }
}

#[derive(Clone, Copy)]
pub struct Planned<'a> {
    content: &'a Content,
    plan: &'a Plan,
}

impl<'a> Planned<'a> {
    pub fn new(content: &'a Content, plan: &'a Plan) -> Self {
        debug_assert_eq!(content.provinces.len(), plan.slots.len());
        Self { content, plan }
    }
}

fn refusal<T>(result: Result<T, String>) -> String {
    match result {
        Err(reason) => reason,
        Ok(_) => "weather temperature plan differs from its content".into(),
    }
}

impl Pass for Planned<'_> {
    fn content(&self) -> &Content {
        self.content
    }

    fn active(&self, slot: u32) -> bool {
        self.plan.slots[slot as usize].active
    }

    fn slot(&self, slot: u32, hour: i32, date: i32, states: &[State]) -> Result<Staging, String> {
        let at = self.plan.slots[slot as usize];
        let bounds = if at.region == NO_REGION {
            None
        } else {
            match &self.plan.periods[at.region as usize] {
                None => return Err(refusal(self.content.period(slot, hour))),
                Some(periods) => match periods
                    .iter()
                    .find(|period| period.start <= date && date <= period.end)
                {
                    None => None,
                    Some(period) => Some(match at.shift {
                        Shift::None => shifted(period, None),
                        Shift::By(value) => shifted(period, Some(value)),
                        Shift::Refused => {
                            return Err(refusal(
                                self.content.provinces[slot as usize]
                                    .terrain
                                    .as_ref()
                                    .map_err(|reason| format!("weather province {slot}: {reason}")),
                            ));
                        }
                    }),
                },
            }
        };
        if at.broken {
            return Err(refusal(self.content.neighbour_sums(slot, states)));
        }
        let (mut temperatures, mut drifts, mut active) = (0_i64, 0_i64, 0_i32);
        for &neighbour in &self.plan.neighbours[at.first as usize..at.end as usize] {
            let state = &states[neighbour as usize];
            temperatures = temperatures.wrapping_add(state.temperature);
            drifts = drifts.wrapping_add(state.drift);
            active = active.wrapping_add(1);
        }
        Ok((bounds, (temperatures, drifts, active), at.divisor))
    }
}

#[derive(Debug, Default)]
pub struct Memo {
    hold: bool,
    branch: u64,
    anchor: Option<(i32, i32)>,
    anchored: Vec<State>,
    consumed: Vec<Option<u32>>,
    simulation: Option<Simulation>,
    held: Vec<Simulation>,
    changed: Vec<(u32, i64, i64)>,
    incremental: bool,
    staged: Vec<(u32, State, bool)>,
    witness: Option<u64>,
}

impl Memo {
    pub fn reserve_changes(&mut self, content: &Content) -> std::io::Result<()> {
        let count = window_capacity(
            content.settings.provinces_per_update,
            content.provinces.len(),
        );
        if count <= self.changed.capacity() {
            return Ok(());
        }
        let mut changed = Vec::new();
        engine_allocation::reserve_exact(&mut changed, count)?;
        self.changed = changed;
        Ok(())
    }

    pub fn change_bytes(&self) -> usize {
        self.changed.capacity() * size_of::<(u32, i64, i64)>()
            + self.staged.capacity() * size_of::<(u32, State, bool)>()
    }

    fn keep(&mut self, simulation: Option<Simulation>) {
        self.simulation = simulation;
        self.witness = None;
    }

    pub fn holding() -> Self {
        Self {
            hold: true,
            ..Self::default()
        }
    }

    pub fn states(
        &mut self,
        branch: u64,
        hour: i32,
        rows: &[WeatherProvinceRow],
        content: Result<&Content, &str>,
    ) -> Result<&[State], String> {
        self.states_changed(branch, hour, rows, content, None)
    }

    pub fn states_changed<P: Pass + ?Sized, R: ProvinceRows + ?Sized>(
        &mut self,
        branch: u64,
        hour: i32,
        rows: &R,
        content: Result<&P, &str>,
        changed: Option<&[u128]>,
    ) -> Result<&[State], String> {
        self.incremental = false;
        self.changed.clear();
        let first = rows.pass()?;
        let pass = first.as_deref().and_then(|pass| match pass {
            Recorded::Value(Some(pass)) => Some(pass),
            _ => None,
        });
        let count = rows.len();
        let same_anchor = changed.is_some_and(|changed| {
            self.branch == branch
                && pass.is_some_and(|pass| self.anchor == Some((pass.anchor, pass.cursor)))
                && count == self.anchored.len()
                && count != 0
                && changed.iter().all(|&key| {
                    usize::try_from(key).ok().and_then(|slot| {
                        let (id, temperature) = rows.row(slot).ok().flatten()?;
                        Some(
                            id as usize == slot
                                && temperature.record().ok()
                                    == Some(TemperatureRecord::Anchored(self.anchored[slot])),
                        )
                    }) == Some(true)
                })
        });
        let anchored = if same_anchor {
            None
        } else {
            let mut anchored = Vec::new();
            let mut recorded = Vec::new();
            for slot in 0..count {
                let (id, temperature) = rows
                    .row(slot)?
                    .ok_or("WeatherProvinces holds fewer rows than it counts")?;
                if id as usize != slot {
                    return Err("WeatherProvinces slots are not dense".into());
                }
                match temperature.record()? {
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
                self.held.clear();
                self.keep(Some(Simulation::new(hour, 0, recorded)));
                return Ok(&self.simulation.as_ref().unwrap().states);
            }
            Some(anchored)
        };
        self.states_anchored(
            branch,
            hour,
            anchored,
            first.as_deref(),
            content,
            changed.is_some(),
        )
    }

    fn states_anchored<P: Pass + ?Sized>(
        &mut self,
        branch: u64,
        hour: i32,
        anchored: Option<Vec<State>>,
        pass: Option<&Recorded<Option<crate::TemperaturePass>>>,
        content: Result<&P, &str>,
        collect_changes: bool,
    ) -> Result<&[State], String> {
        let pass = match pass {
            Some(Recorded::Value(Some(pass))) => pass,
            Some(Recorded::Value(None)) => {
                return Err("the collector did not read this hour's temperature pass whole".into());
            }
            _ => return Err("WeatherProvinces temperature pass not recorded".into()),
        };
        let Some(recorded) = pass.witness.get() else {
            return Err(format!(
                "this recording predates the province temperature witness (format {}): no derived temperature can be certified",
                replay_rows::rows::compatibility::PROVINCE_WITNESS_FORMAT
            ));
        };
        if i64::from(hour) - i64::from(pass.anchor) != pass.hours.len() as i64 {
            return Err(format!(
                "the temperature pass records {} hours after its anchor {}, the frame is hour {hour}",
                pass.hours.len(),
                pass.anchor
            ));
        }
        let content = content.map_err(|reason| format!("temperature content: {reason}"))?;
        if self.branch != branch
            || self.anchor != Some((pass.anchor, pass.cursor))
            || anchored
                .as_ref()
                .is_some_and(|anchored| self.anchored.as_slice() != anchored.as_slice())
        {
            self.branch = branch;
            self.anchor = Some((pass.anchor, pass.cursor));
            self.consumed.clear();
            self.keep(None);
            self.held.clear();
            self.anchored = anchored.unwrap_or_default();
        }
        let agreed = self
            .consumed
            .iter()
            .zip(&pass.hours)
            .take_while(|(kept, entry)| kept == entry)
            .count();
        self.consumed.truncate(agreed);
        let steps = |simulation: &Simulation| {
            (i64::from(simulation.hour) - i64::from(pass.anchor)) as usize
        };
        self.held.retain(|held| steps(held) <= agreed);
        if self
            .simulation
            .as_ref()
            .is_some_and(|simulation| steps(simulation) > agreed)
        {
            self.keep(None);
        }
        let reach = pass.hours.len();
        let kept = self
            .simulation
            .as_ref()
            .filter(|simulation| steps(simulation) <= reach)
            .map(|simulation| simulation.hour);
        let held = self
            .held
            .iter()
            .filter(|held| steps(held) <= reach)
            .max_by_key(|held| held.hour);
        let start = match (kept, held) {
            (Some(kept), Some(held)) if held.hour > kept => Some(held.clone()),
            (Some(_), _) => None,
            (None, Some(held)) => Some(held.clone()),
            (None, None) => Some(Simulation::new(
                pass.anchor,
                pass.cursor,
                self.anchored.clone(),
            )),
        };
        let continued = start.is_none();
        if let Some(start) = start {
            if let Some(previous) = self.simulation.take()
                && self.hold
                && !self.held.iter().any(|held| held.hour == previous.hour)
            {
                self.held.push(previous);
            }
            self.keep(Some(start));
        }
        let from = steps(self.simulation.as_ref().unwrap());
        let incremental = collect_changes
            && continued
            && self.simulation.as_ref().unwrap().hour.checked_add(1) == Some(hour);
        if incremental {
            self.reserve_changes(content.content())
                .map_err(|error| error.to_string())?;
        }
        for at in from..reach {
            let entry = pass.hours[at];
            let simulation = self.simulation.as_mut().unwrap();
            if let Err(reason) = simulation.advance_with(
                content,
                entry,
                &mut self.staged,
                incremental.then_some(&mut self.changed),
                &mut self.witness,
            ) {
                self.anchor = None;
                self.keep(None);
                self.held.clear();
                return Err(reason);
            }
            if self.consumed.len() == at {
                self.consumed.push(entry);
            }
        }
        let simulation = self.simulation.as_ref().unwrap();
        if *self.witness.get_or_insert_with(|| simulation.witness()) != recorded {
            return Err(format!(
                "derived temperatures at hour {hour} differ from the recorded witness"
            ));
        }
        if self.hold && !self.held.iter().any(|held| held.hour == simulation.hour) {
            self.held.push(simulation.clone());
        }
        self.incremental = incremental;
        Ok(&simulation.states)
    }

    pub fn changes(&self) -> Option<&[(u32, i64, i64)]> {
        self.incremental.then_some(&self.changed)
    }

    pub fn retain_held(&mut self, mut alive: impl FnMut(u64, i32) -> bool) {
        let branch = self.branch;
        self.held.retain(|held| alive(branch, held.hour));
    }

    pub fn held(&self) -> usize {
        self.held.len()
    }

    pub fn held_bytes(&self) -> usize {
        self.held
            .iter()
            .map(|held| std::mem::size_of_val(held.states.as_slice()))
            .sum()
    }

    pub fn simulation(&self) -> Option<&Simulation> {
        self.simulation.as_ref()
    }
}

#[cfg(test)]
mod tests;
