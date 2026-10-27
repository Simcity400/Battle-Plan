//! Province temperatures replayed from recorded inputs and the recorded content stack, with no
//! memory access: the signed fixed5 committed temperature of each province weather record, which
//! the province tooltip and the save serializer read.
//!
//! Each hour that the weather update finds both suspension gates clear, it adds a synchronized
//! 31-bit random integer to the manager's seed and runs one pass over a rotating window: from the
//! manager's cursor, `provinces_per_update` consecutive slots, wrapping at the province count, of
//! which the active ones update ([`Simulation::update`]). For each, the temperature pass moves its
//! working drift by a hash of its id and the seed, clamps it to `max_temperature_change` and stages
//! the committed temperature plus the drift, resetting a drift that pushes further outside the
//! date's weather-period range; the threaded smoothing pass then moves the staged temperature and
//! the drift toward the averages of the active neighbours' committed temperature and committed
//! drift. Only after every staging job joined does the commit pass copy the staged temperature and
//! working drift into the committed temperature and drift for each province that had an active
//! neighbour, so the staging order is irrelevant and every thread count gives the same result.
//!
//! The seed is the only input of an hour the content cannot supply; a recording stores it
//! with the state of every province at an anchor hour ([`State`]), from which playback
//! steps forward. Every function preserves the game's integer arithmetic: 64-bit wrapping
//! products and sums, truncating division, the fixed-point division guard that stores
//! `0xFFFFFFFF`, and the date functions' 32-bit arithmetic.

use std::{borrow::Cow, collections::BTreeMap};

mod input;
pub use input::Input;

use crate::rows::{Recorded, TemperatureRecord, WeatherProvinceRow};

/// `fixed5` scale.
pub const SCALE: i64 = 100_000;
/// What the native fixed-point division stores when its guard refuses the numerator.
const REFUSED: i64 = 0xffff_ffff;
#[cfg(test)]
use weather_period::HOURS_PER_YEAR;
pub use weather_period::{
    EPOCH, PERIOD_DEFAULT_DATE, PERIOD_DEFAULT_TEMPERATURE, Period, apply_bound, find_period,
    normalize_to_year_one, period_date, set_day_of_month_zero_based, set_month_zero_based,
};

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

/// One `terrain_modifiers` `temperature` entry (`weather terrain modifier`, stride `0x18`): the
/// terrain's database index and the fixed5 degrees added to both bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerrainModifier {
    pub terrain: i32,
    pub value: i64,
}

/// Settings loaded from `common/weather.txt`, every value zero when the file leaves it unset
/// (manager construction clears every setting).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// `temperature_variation`.
    pub variation: i64,
    /// `max_temperature_change`.
    pub max_change: i64,
    /// `temperature_neighbor_smoothing`.
    pub smoothing: i64,
    /// `temperature_change_neighbor_smoothing`.
    pub change_smoothing: i64,
    /// `provinces_per_update`.
    pub provinces_per_update: i32,
    /// The `temperature` kind's terrain modifiers in loaded order.
    pub terrain: Vec<TerrainModifier>,
}

/// Initialization inputs for one province slot of the weather array.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Province {
    /// A positive id whose definition exists and has pixels.
    pub active: bool,
    /// The slot of every neighbour record of the definition, in order.
    pub neighbours: Vec<u32>,
    /// The province's strategic region id, `None` for a null region.
    pub region: Option<u32>,
    /// The definition terrain's database index, `None` for -1; an error names what the content
    /// cannot settle, refused only when the province reaches the terrain lookup.
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
    /// The period the temperature pass finds for `slot` at native date `hour`; `None` where it
    /// returns before touching the record: a null strategic region or no period.
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
    /// The committed temperature.
    pub temperature: i64,
    /// The committed drift neighbours average.
    pub drift: i64,
    /// The staged temperature.
    pub staged: i64,
    /// The working drift.
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
    let span = window_capacity(window, count as usize) as i32;
    Ok((
        span,
        ((i64::from(cursor) + i64::from(span)) % i64::from(count)) as i32,
    ))
}

fn window_capacity(width: i32, count: usize) -> usize {
    // The first native signed decrement wraps at i32::MIN.
    if width == i32::MIN {
        count
    } else {
        (width.max(1) as usize).min(count)
    }
}

/// The slots an update starting at `cursor` visits, and the cursor it leaves, following the
/// manager's rotating cursor: `window` slots (at least one), wrapping to zero at the count and
/// stopping early on returning to the start. A cursor at or past the count visits nothing and
/// stays; a negative one would index before the array and is refused.
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
        self.advance_changed(content, seed, None)
    }

    fn advance_changed(
        &mut self,
        content: &Content,
        seed: Option<u32>,
        changed: Option<&mut Vec<(u32, i64, i64)>>,
    ) -> Result<(), String> {
        let hour = self
            .hour
            .checked_add(1)
            .ok_or("weather temperature hour overflow")?;
        if let Some(seed) = seed {
            self.update_changed(content, hour, seed, changed)?;
        }
        self.hour = hour;
        Ok(())
    }

    /// One ordinary update (the manager's pass flag clear, pass index zero) at native date `hour`.
    pub fn update(&mut self, content: &Content, hour: i32, seed: u32) -> Result<(), String> {
        self.update_changed(content, hour, seed, None)
    }

    fn update_changed(
        &mut self,
        content: &Content,
        hour: i32,
        seed: u32,
        mut changed: Option<&mut Vec<(u32, i64, i64)>>,
    ) -> Result<(), String> {
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
            let before = self.states[slot as usize].temperature;
            if before != state.temperature
                && let Some(changed) = changed.as_deref_mut()
            {
                changed
                    .try_reserve(1)
                    .map_err(|error| format!("weather temperature changes: {error}"))?;
                changed.push((slot, before, state.temperature));
            }
            self.states[slot as usize] = state;
        }
        self.cursor = cursor;
        Ok(())
    }

    /// Temperature drift and neighbour smoothing for one slot, reading the committed words of the
    /// hour before; whether the record's commit flag is set.
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
        // Smoothing clears the commit flag and sets it only when an active neighbour exists.
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
/// Holding is opt-in and follows the owner's live frames, not a separate cache limit.
#[derive(Debug, Default)]
pub struct Memo {
    hold: bool,
    branch: u64,
    /// The anchor hour and cursor of the kept pass.
    anchor: Option<(i32, i32)>,
    anchored: Vec<State>,
    /// The kept pass's hour entries, from the anchor, that every held derivation agrees with.
    consumed: Vec<Option<u32>>,
    simulation: Option<Simulation>,
    /// Derivations of the kept pass, at most one per hour; always empty unless holding.
    held: Vec<Simulation>,
    changed: Vec<(u32, i64, i64)>,
    changed_credit: Option<engine_allocation::credit::Credit>,
    incremental: bool,
    detached_words: Option<std::sync::Weak<input::Words>>,
}

impl Memo {
    /// Admit the native update window before any committed edit can be appended.
    pub fn admit_changes(&mut self, content: &Content) -> std::io::Result<()> {
        self.admit_changes_in(
            content,
            &engine_allocation::credit::MemoryBudget::preparation(),
        )
    }

    fn admit_changes_in(
        &mut self,
        content: &Content,
        budget: &engine_allocation::credit::MemoryBudget,
    ) -> std::io::Result<()> {
        let count = window_capacity(
            content.settings.provinces_per_update,
            content.provinces.len(),
        );
        if count <= self.changed.capacity() {
            return Ok(());
        }
        let bytes = engine_allocation::mul(count, size_of::<(u32, i64, i64)>())?;
        let credit = budget.admit(bytes).map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("regional-weather temperature change capacity: {error}"),
            )
        })?;
        let mut changed = Vec::new();
        engine_allocation::reserve_exact(&mut changed, count)?;
        self.changed = changed;
        self.changed_credit = Some(credit);
        Ok(())
    }

    pub fn change_bytes(&self) -> usize {
        self.changed.capacity() * size_of::<(u32, i64, i64)>()
    }

    /// The owner must trim held derivations to its live frames with [`Self::retain_held`].
    pub fn holding() -> Self {
        Self {
            hold: true,
            ..Self::default()
        }
    }

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
        self.states_changed(branch, hour, rows, content, None)
    }

    /// `changed` certifies exactly the changed/deleted keys since this memo's last successful hour.
    pub fn states_changed(
        &mut self,
        branch: u64,
        hour: i32,
        rows: &[WeatherProvinceRow],
        content: Result<&Content, &str>,
        changed: Option<&[u128]>,
    ) -> Result<&[State], String> {
        self.incremental = false;
        self.changed.clear();
        self.detached_words = None;
        let pass = rows.first().and_then(|row| match &row.temperature_pass {
            Recorded::Value(Some(pass)) => Some(pass),
            _ => None,
        });
        let same_anchor = changed.is_some_and(|changed| {
            self.branch == branch
                && pass.is_some_and(|pass| self.anchor == Some((pass.anchor, pass.cursor)))
                && rows.len() == self.anchored.len()
                && !rows.is_empty()
                && changed.iter().all(|&key| {
                    usize::try_from(key).ok().and_then(|slot| {
                        let row = rows.get(slot)?;
                        Some(
                            row.id as usize == slot
                                && row.temperature.record().ok()
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
                self.held.clear();
                self.simulation = Some(Simulation::new(hour, 0, recorded));
                return Ok(&self.simulation.as_ref().unwrap().states);
            }
            Some(Cow::Owned(anchored))
        };
        self.states_anchored(
            branch,
            hour,
            anchored,
            rows.first().map(|row| &row.temperature_pass),
            content,
            changed.is_some(),
        )
    }

    fn states_anchored(
        &mut self,
        branch: u64,
        hour: i32,
        anchored: Option<Cow<'_, [State]>>,
        pass: Option<&Recorded<Option<crate::rows::TemperaturePass>>>,
        content: Result<&Content, &str>,
        collect_changes: bool,
    ) -> Result<&[State], String> {
        let pass = match pass {
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
        if self.branch != branch
            || self.anchor != Some((pass.anchor, pass.cursor))
            || anchored
                .as_ref()
                .is_some_and(|anchored| self.anchored.as_slice() != anchored.as_ref())
        {
            self.branch = branch;
            self.anchor = Some((pass.anchor, pass.cursor));
            self.consumed.clear();
            self.simulation = None;
            self.held.clear();
            self.anchored = anchored.map(Cow::into_owned).unwrap_or_default();
        }
        // Derivations past the shared seed prefix cannot certify this frame.
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
            self.simulation = None;
        }
        // Resume only from the shared prefix, never from a later hour.
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
            self.simulation = Some(start);
        }
        let from = steps(self.simulation.as_ref().unwrap());
        let incremental = collect_changes
            && continued
            && self.simulation.as_ref().unwrap().hour.checked_add(1) == Some(hour);
        if incremental {
            self.admit_changes(content)
                .map_err(|error| error.to_string())?;
        }
        for at in from..reach {
            let entry = pass.hours[at];
            let simulation = self.simulation.as_mut().unwrap();
            if let Err(reason) =
                simulation.advance_changed(content, entry, incremental.then_some(&mut self.changed))
            {
                self.anchor = None;
                self.simulation = None;
                self.held.clear();
                return Err(reason);
            }
            if self.consumed.len() == at {
                self.consumed.push(entry);
            }
        }
        let simulation = self.simulation.as_ref().unwrap();
        if simulation.checksum() != pass.checksum {
            return Err(format!(
                "derived temperatures at hour {hour} differ from the recorded checksum"
            ));
        }
        if self.hold && !self.held.iter().any(|held| held.hour == simulation.hour) {
            self.held.push(simulation.clone());
        }
        self.incremental = incremental;
        Ok(&simulation.states)
    }

    /// Committed temperature edits from the preceding successful hour; None after any root reset.
    pub fn changes(&self) -> Option<&[(u32, i64, i64)]> {
        self.incremental.then_some(&self.changed)
    }

    /// Trims held derivations by branch and hour without discarding the last simulation.
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

    /// The kept derivation, at the hour of the last successful call.
    pub fn simulation(&self) -> Option<&Simulation> {
        self.simulation.as_ref()
    }
}

#[cfg(test)]
mod tests;
