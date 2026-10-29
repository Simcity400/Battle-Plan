//! The selected army's attack modifiers against a hovered province, derived at playback
//! with no memory access: the list built for each representative division the
//! selected-army view shows, including river, fort, snow and fort-base modifiers.
//!
//! The view uses the last two provinces of the retained route (the division's own
//! province when the route has one node) and prints each entry's first value: the
//! modifier's name, a colon and space, and a percentage. [`preview`] follows the native
//! order: the outer gates, then
//!
//! 1. terrain (kind `0x1B`): attack for the target's terrain (the stat block's
//!    adjuster entry at the terrain's database index minus one, plus the
//!    terrain's own adjuster), plus the direct commander's terrain attack;
//!    listed when nonzero;
//! 2. river (`0x0A`), fort (`0x09`) and snow (`0x1C`);
//! 3. region weather (`0x1C`): the target region's weather `army_attack_factor`
//!    and `breakthrough_factor` times the division's acclimatization factor;
//! 4. night (`0x1D`), when the target's night factor exceeds 0.8;
//! 5. enemy air superiority (`0x0E`, first value zero), when the division's own
//!    region holds a positive enemy score for its logical owner.
//!
//! Each list entry has two values; only the first is printed, and the second
//! only decides whether the river, fort and region weather entries appear, so
//! [`Row`] keeps the printed value. Every product is the native 64-bit wrapping
//! one and every division truncates toward zero.
//!
//! [`Inputs`] answers each operand only where the native code reaches it, so a
//! missing operand refuses its own entry and nothing else. [`FrameInputs`] reads
//! them from a frame and the recorded content ([`Context`]): from format 102
//! the inputs the rows record ([`crate::rows::DivisionAttackStats`] and its
//! siblings), and before it the native operands recorded then
//! ([`crate::rows::DivisionAttackInputs`] and its siblings).

use crate::{
    frame::DecodedFrame,
    rows::{self, TableSchema, tables},
};

pub type Fact<T> = Result<T, String>;

/// `fixed5` one.
pub const ONE: i64 = 100_000;
/// Modifier kinds used by the selected-army view.
pub const TERRAIN: u32 = 0x1b;
pub const RIVER: u32 = 0x0a;
pub const FORT: u32 = 0x09;
/// Snow and region weather share this kind.
pub const WEATHER: u32 = 0x1c;
pub const NIGHT: u32 = 0x1d;
pub const ENEMY_AIR: u32 = 0x0e;
/// The night entry appears only above this night factor.
pub const NIGHT_THRESHOLD: i64 = 80_000;

/// One printed list entry: its kind and first value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub kind: u32,
    pub value: i64,
}

/// The list in native order and the entries that could not be decided, by
/// name, each with its reason. A refused entry never stands in as zero.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    pub rows: Vec<Row>,
    pub refusals: Vec<(&'static str, String)>,
}

/// Attack-modifier scalars of a stat block (the river and fort pairs, the snow term and the night
/// term), each refused alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scalars {
    pub river: [Fact<i64>; 2],
    pub fort: [Fact<i64>; 2],
    pub snow: Fact<i64>,
    pub night: Fact<i64>,
}

/// The direct commander's values for the target (river, fort, snow and night adjusters and the
/// terrain adjuster's attack), each refused alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commander {
    pub terrain: Fact<i64>,
    pub river: Fact<i64>,
    pub fort: Fact<i64>,
    pub snow: Fact<i64>,
    pub night: Fact<i64>,
}

/// The four `NMilitary` defines the attack preview reads: `RIVER_CROSSING_PENALTY`,
/// `RIVER_CROSSING_PENALTY_LARGE`, `BASE_NIGHT_ATTACK_PENALTY` and `BASE_FORT_PENALTY`,
/// each refused alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Defines {
    pub river_small: Fact<i64>,
    pub river_large: Fact<i64>,
    pub night: Fact<i64>,
    pub fort: Fact<i64>,
}

/// The operands the list reads for one division against one move.
pub trait Inputs {
    /// Raw controller (the province's).
    fn controller(&self, province: u32) -> Fact<u32>;
    /// Whether both raw references resolve to the same country.
    fn same_country(&self, first: u32, second: u32) -> Fact<bool>;
    /// The division's logical owner's direct-war cell toward the target's controller: its war
    /// pointer is set and the relation's retired byte is zero.
    fn at_war(&self, target_controller: u32) -> Fact<bool>;
    /// Attack for the target province's terrain.
    fn terrain_attack(&self, target: u32) -> Fact<i64>;
    /// The direct commander; `None` without one.
    fn commander(&self, target: u32) -> Fact<Option<Commander>>;
    /// The directed neighbour entry's crossing kind, `None` when the target is not a neighbour.
    fn crossing(&self, from: u32, to: u32) -> Fact<Option<u8>>;
    /// Whether the target province has positive effective snow.
    fn has_snow(&self, target: u32) -> Fact<bool>;
    fn scalars(&self) -> Fact<Scalars>;
    fn defines(&self) -> Fact<Defines>;
    /// Healthy land-fort levels, summed with signed 32-bit wrap.
    fn fort_level(&self, target: u32) -> Fact<i32>;
    /// Noncoastal fort base for the target's controller.
    fn fort_base(&self, target: u32) -> Fact<i64>;
    /// The target region weather modifier's effective `army_attack_factor` and
    /// `breakthrough_factor`; `None` when no region weather modifier exists.
    fn region_weather(&self, target: u32) -> Fact<Option<[i64; 2]>>;
    /// The division's weather acclimatization factor.
    fn weather_factor(&self) -> Fact<i64>;
    /// The target province's held night factor.
    fn night_factor(&self, target: u32) -> Fact<i64>;
    /// The effective `land_night_attack` of the division's owner (the
    /// expeditionary owner when positive, otherwise the roster owner).
    fn night_modifier(&self) -> Fact<i64>;
    /// A positive enemy score in the logical owner's strategic-air entry for the division's own
    /// region.
    fn enemy_air(&self) -> Fact<bool>;
}

fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}

/// River pair for crossing define `define` and commander river `leader`, with the outer
/// multiplier one.
pub fn river(unit: [i64; 2], leader: i64, define: i64) -> [i64; 2] {
    let adjustment = mul(define.wrapping_add(leader), ONE);
    unit.map(|value| value.wrapping_add(adjustment))
}

/// Fort base without the coastal key: the define alone for a null controller; otherwise
/// add country, one and state, then multiply by the define.
pub fn fort_base(country: Option<i64>, state: i64, define: i64) -> i64 {
    match country {
        None => define,
        Some(country) => mul(country.wrapping_add(ONE).wrapping_add(state), define),
    }
}

/// Fort pair with one direction and both multipliers one; `None` when the healthy level
/// is not positive.
pub fn fort(unit: [i64; 2], leader: i64, level: i32, base: i64) -> Option<[i64; 2]> {
    if i64::from(level) * ONE <= 0 {
        return None;
    }
    // max(level - (directions - 1), 0) * ONE, at least ONE.
    let levels = (i64::from(level) * ONE).max(ONE);
    let factor = mul(mul(levels, base), ONE).wrapping_add(leader);
    Some(unit.map(|value| mul(value.wrapping_add(factor).min(0), ONE)))
}

/// Region weather pair times the acclimatization factor.
pub fn weather(pair: [i64; 2], factor: i64) -> [i64; 2] {
    pair.map(|value| value.wrapping_mul(factor) / ONE)
}

/// Night attack value.
pub fn night(stat: i64, define: i64, country: i64, leader: i64) -> i64 {
    stat.wrapping_add(define)
        .wrapping_add(country)
        .wrapping_add(leader)
}

/// Effective snow is positive after adding permanent snow and applying its ceiling.
pub fn has_snow(snow: i64, permanent: i64, ceiling: i64) -> bool {
    permanent.wrapping_add(snow).min(ceiling) > 0
}

fn push(out: &mut Preview, name: &'static str, row: Fact<Option<Row>>) {
    match row {
        Ok(Some(row)) => out.rows.push(row),
        Ok(None) => {}
        Err(reason) => out.refusals.push((name, reason)),
    }
}

fn leader(
    commander: &Fact<Option<Commander>>,
    field: impl Fn(&Commander) -> &Fact<i64>,
) -> Fact<i64> {
    match commander {
        Ok(None) => Ok(0),
        Ok(Some(commander)) => field(commander).clone(),
        Err(reason) => Err(reason.clone()),
    }
}

/// Build the attack preview: `None` when an outer gate returns before the list (either
/// controller not positive, the same raw controller or country, or no direct war of the
/// logical owner toward the target), the list otherwise.
pub fn preview(inputs: &impl Inputs, from: u32, to: u32) -> Fact<Option<Preview>> {
    let source = inputs.controller(from)?;
    if (source as i32) < 1 {
        return Ok(None);
    }
    let target = inputs.controller(to)?;
    if (target as i32) < 1 || source == target {
        return Ok(None);
    }
    if inputs.same_country(source, target)? || !inputs.at_war(target)? {
        return Ok(None);
    }
    let mut out = Preview::default();
    let commander = inputs.commander(to);
    let scalars = inputs.scalars();
    let scalar = |field: fn(&Scalars) -> &Fact<i64>| match &scalars {
        Ok(scalars) => field(scalars).clone(),
        Err(reason) => Err(reason.clone()),
    };
    let defines = inputs.defines();
    let define = |field: fn(&Defines) -> &Fact<i64>| match &defines {
        Ok(defines) => field(defines).clone(),
        Err(reason) => Err(reason.clone()),
    };
    push(
        &mut out,
        "terrain",
        (|| {
            let value = inputs
                .terrain_attack(to)?
                .wrapping_add(leader(&commander, |c| &c.terrain)?);
            Ok((value != 0).then_some(Row {
                kind: TERRAIN,
                value,
            }))
        })(),
    );
    // find_neighbour; a crossing kind 2 is a river, 3 a large one, and only a neighbour
    // checks for positive effective snow.
    let crossing = inputs.crossing(from, to);
    let snow = crossing.clone().and_then(|kind| match kind {
        Some(_) => inputs.has_snow(to),
        None => Ok(false),
    });
    push(
        &mut out,
        "river",
        (|| {
            let define = match crossing.clone()? {
                Some(2) => define(|d| &d.river_small)?,
                Some(3) => define(|d| &d.river_large)?,
                _ => return Ok(None),
            };
            let unit = [scalar(|s| &s.river[0])?, scalar(|s| &s.river[1])?];
            let pair = river(unit, leader(&commander, |c| &c.river)?, define);
            Ok((pair != [0, 0]).then_some(Row {
                kind: RIVER,
                value: pair[0],
            }))
        })(),
    );
    push(
        &mut out,
        "fort",
        (|| {
            let level = inputs.fort_level(to)?;
            if i64::from(level) * ONE <= 0 {
                return Ok(None);
            }
            let base = inputs.fort_base(to)?;
            let unit = [scalar(|s| &s.fort[0])?, scalar(|s| &s.fort[1])?];
            let pair = fort(unit, leader(&commander, |c| &c.fort)?, level, base);
            Ok(pair.filter(|pair| *pair != [0, 0]).map(|pair| Row {
                kind: FORT,
                value: pair[0],
            }))
        })(),
    );
    push(
        &mut out,
        "snow",
        (|| {
            if !snow.clone()? {
                return Ok(None);
            }
            let value = mul(
                scalar(|s| &s.snow)?.wrapping_add(leader(&commander, |c| &c.snow)?),
                ONE,
            );
            Ok((value != 0).then_some(Row {
                kind: WEATHER,
                value,
            }))
        })(),
    );
    push(
        &mut out,
        "region weather",
        (|| {
            let Some(pair) = inputs.region_weather(to)? else {
                return Ok(None);
            };
            // Both products are zero for every factor.
            if pair == [0, 0] {
                return Ok(None);
            }
            let pair = weather(pair, inputs.weather_factor()?);
            Ok((pair != [0, 0]).then_some(Row {
                kind: WEATHER,
                value: pair[0],
            }))
        })(),
    );
    push(
        &mut out,
        "night",
        (|| {
            if inputs.night_factor(to)? <= NIGHT_THRESHOLD {
                return Ok(None);
            }
            Ok(Some(Row {
                kind: NIGHT,
                value: night(
                    scalar(|s| &s.night)?,
                    define(|d| &d.night)?,
                    inputs.night_modifier()?,
                    leader(&commander, |c| &c.night)?,
                ),
            }))
        })(),
    );
    push(
        &mut out,
        "enemy air",
        inputs.enemy_air().map(|positive| {
            positive.then_some(Row {
                kind: ENEMY_AIR,
                value: 0,
            })
        }),
    );
    Ok(Some(out))
}

/// The recorded content the frame's rows refer to.
pub trait Context {
    /// The province's terrain: its one-based database index and the terrain's own adjuster attack.
    fn terrain(&self, province: u32) -> Fact<Terrain>;
    /// The crossing kind of the neighbour `from -> to`, `None` when `to` is
    /// not one of `from`'s neighbours.
    fn crossing(&self, from: u32, to: u32) -> Fact<Option<u8>>;
    /// The province's strategic region id (through the province's strategic region); `None` for a
    /// province without one.
    fn region(&self, province: u32) -> Fact<Option<u32>>;
    /// The building type's `land_fort` for a lexer token.
    fn land_fort(&self, building_type: u32) -> Fact<i32>;
    /// The province's night factor at the frame's hour.
    fn night_factor(&self, province: u32) -> Fact<i64>;
    /// Whether `receiver`'s direct-war cell toward `target` (both raw
    /// country references) holds an active war.
    fn at_war(&self, receiver: u32, target: u32) -> Fact<bool>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Terrain {
    pub index: i32,
    pub attack: i64,
}

/// One division's operands read from a frame and its recorded content.
pub struct FrameInputs<'a, C: Context> {
    frame: &'a DecodedFrame,
    context: &'a C,
    division: u64,
}

fn complete<T: TableSchema>(frame: &DecodedFrame) -> Fact<&[T::Row]> {
    frame.complete::<T>().map_err(|error| error.to_string())
}

fn keyed<'a, T: TableSchema>(
    frame: &'a DecodedFrame,
    key: u64,
    key_of: impl Fn(&T::Row) -> u64,
    name: &str,
) -> Fact<&'a T::Row> {
    let rows = complete::<T>(frame)?;
    rows.binary_search_by_key(&key, key_of)
        .map(|at| &rows[at])
        .map_err(|_| format!("{name} has no row {key}"))
}

fn known<T: Clone>(value: Option<T>, name: &str) -> Fact<T> {
    value.ok_or_else(|| format!("{name} was refused when recorded"))
}

fn fresh(frame: &DecodedFrame, hour: rows::PacketHour, name: &str) -> Fact<()> {
    if hour.fresh_at(frame.hour) {
        Ok(())
    } else {
        Err(format!(
            "{name} sample hour {hour:?} differs from frame hour {}",
            frame.hour
        ))
    }
}

impl<'a, C: Context> FrameInputs<'a, C> {
    pub fn new(frame: &'a DecodedFrame, context: &'a C, division: u64) -> Self {
        Self {
            frame,
            context,
            division,
        }
    }

    fn division(&self) -> Fact<&'a rows::DivisionRow> {
        keyed::<tables::Divisions>(self.frame, self.division, |row| row.id, "Divisions")
    }

    fn status(&self) -> Fact<&'a rows::DivisionStatusRow> {
        keyed::<tables::DivisionStatus>(
            self.frame,
            self.division,
            |row| row.division,
            "DivisionStatus",
        )
    }

    /// Format 102's stat-block operands; `None` for a recording before them.
    fn stats(&self) -> Fact<Option<&'a rows::DivisionAttackStats>> {
        match &self.status()?.attack_stats {
            rows::Recorded::NotRecorded => Ok(None),
            rows::Recorded::Value(stats) => {
                known(stats.as_ref().map(Some), "DivisionStatus.attack_stats")
            }
        }
    }

    /// The operands a recording before format 102 captured.
    fn operands(&self) -> Fact<&'a rows::DivisionAttackInputs> {
        let packet = self
            .status()?
            .attack
            .require("DivisionStatus.attack")?
            .get()
            .ok_or("DivisionStatus.attack holds no operands")?;
        fresh(self.frame, packet.sample_hour, "DivisionStatus.attack")?;
        Ok(packet)
    }

    fn province(&self, id: u32) -> Fact<&'a rows::ProvinceRow> {
        keyed::<tables::Provinces>(
            self.frame,
            u64::from(id),
            |row| u64::from(row.id),
            "Provinces",
        )
    }

    /// CountryLinks' canonical slot of a raw reference.
    fn slot(&self, raw: u32) -> Fact<u32> {
        if raw == 0 {
            return Ok(0);
        }
        keyed::<tables::CountryLinks>(
            self.frame,
            u64::from(raw),
            |row| u64::from(row.id),
            "CountryLinks",
        )
        .map(|row| row.canonical_slot)
    }

    /// The Countries id that holds `slot`, the key of its counter inputs.
    fn country_id(&self, slot: u32) -> Fact<u32> {
        let countries = complete::<tables::Countries>(self.frame)?;
        let mut found = countries.iter().filter(|row| row.canonical_slot == slot);
        let id = found
            .next()
            .ok_or_else(|| format!("Countries lacks canonical slot {slot}"))?
            .id;
        if found.next().is_some() {
            return Err(format!("Countries duplicates canonical slot {slot}"));
        }
        Ok(id)
    }

    fn counter_inputs(&self, raw: u32) -> Fact<&'a rows::CountryCounterInputRow> {
        let slot = self.slot(raw)?;
        if slot == 0 {
            return Err(format!(
                "country {raw} is the reserved country, whose attack modifiers are not recorded"
            ));
        }
        let id = self.country_id(slot)?;
        keyed::<tables::CountryCounterInputs>(
            self.frame,
            u64::from(id),
            |row| u64::from(row.country),
            "CountryCounterInputs",
        )
    }

    /// Effective value of a raw modifier entry, with its recorded definition and the DLC
    /// feature mask.
    fn effective(
        &self,
        key: u32,
        raw: Option<i64>,
        family: crate::fixed_layout::Family,
    ) -> Fact<i64> {
        crate::fixed_layout::validate_recording(self.frame, family)?;
        if raw.is_none() {
            return Ok(0);
        }
        let definition = keyed::<tables::ModifierDefinitions>(
            self.frame,
            u64::from(key),
            |row| u64::from(row.index),
            "ModifierDefinitions",
        )?;
        let features = match complete::<tables::SimulationFeatures>(self.frame) {
            Ok([row]) => Some(row.dlc_mask),
            _ => None,
        };
        crate::naval_mission_derive::effective(definition, features, raw).map_err(str::to_owned)
    }

    /// A country's effective scalar: the format-102 raw entries, or the
    /// effective values recorded before them.
    fn country_scalar(
        &self,
        raw: u32,
        key: u32,
        family: crate::fixed_layout::Family,
        modern: fn(&rows::CountryAttackModifiers) -> Option<i64>,
        historical: fn(&rows::CountryAttackInputs) -> Option<i64>,
    ) -> Fact<i64> {
        let row = self.counter_inputs(raw)?;
        match &row.attack_modifiers {
            rows::Recorded::Value(modifiers) => {
                let modifiers = known(modifiers.as_ref(), "CountryCounterInputs.attack_modifiers")?;
                self.effective(key, modern(modifiers), family)
            }
            rows::Recorded::NotRecorded => {
                crate::fixed_layout::validate_recording(self.frame, family)?;
                let packet = row
                    .attack
                    .require("CountryCounterInputs.attack")?
                    .get()
                    .ok_or("CountryCounterInputs.attack holds no operands")?;
                fresh(
                    self.frame,
                    packet.sample_hour,
                    "CountryCounterInputs.attack",
                )?;
                known(historical(packet), "CountryCounterInputs.attack")
            }
        }
    }

    fn orders_group(&self) -> Fact<Option<&'a rows::OrdersGroupCounterInputRow>> {
        let group = self.division()?.orders_group;
        if group == 0 {
            return Ok(None);
        }
        keyed::<tables::OrdersGroupCounterInputs>(
            self.frame,
            group,
            |row| row.group,
            "OrdersGroupCounterInputs",
        )
        .map(Some)
    }

    fn weather_province(&self, id: u32) -> Fact<Option<&'a rows::WeatherProvinceRow>> {
        let rows = complete::<tables::WeatherProvinces>(self.frame)?;
        Ok(rows
            .binary_search_by_key(&id, |row| row.id)
            .ok()
            .map(|at| &rows[at]))
    }

    fn weather_region(&self, id: u32) -> Fact<Option<&'a rows::WeatherRegionRow>> {
        let rows = complete::<tables::WeatherRegions>(self.frame)?;
        Ok(rows
            .binary_search_by_key(&id, |row| row.id)
            .ok()
            .map(|at| &rows[at]))
    }
}

/// Entry `index` of a commander's recorded terrain vector, or the first entry outside it,
/// matching the commander's terrain-adjuster lookup.
fn commander_terrain(values: &[i64], index: i32) -> Fact<i64> {
    usize::try_from(index)
        .ok()
        .and_then(|index| values.get(index))
        .or_else(|| values.first())
        .copied()
        .ok_or_else(|| "the commander's terrain adjuster vector has no storage".into())
}

impl<C: Context> Inputs for FrameInputs<'_, C> {
    fn controller(&self, province: u32) -> Fact<u32> {
        self.province(province).map(|row| row.controller)
    }

    fn same_country(&self, first: u32, second: u32) -> Fact<bool> {
        Ok(self.slot(first)? == self.slot(second)?)
    }

    fn at_war(&self, target_controller: u32) -> Fact<bool> {
        self.context
            .at_war(self.division()?.logical_owner, target_controller)
    }

    fn terrain_attack(&self, target: u32) -> Fact<i64> {
        let terrain = self.context.terrain(target)?;
        match self.stats()? {
            Some(stats) => {
                // Read entry index - 1, then add the terrain's own adjuster.
                let entry = usize::try_from(terrain.index)
                    .ok()
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| stats.terrain.get(index))
                    .ok_or_else(|| {
                        format!(
                            "terrain {} is outside the stat block's {} terrain entries",
                            terrain.index,
                            stats.terrain.len()
                        )
                    })?;
                Ok(entry.wrapping_add(terrain.attack))
            }
            None => {
                let domain = self
                    .operands()?
                    .terrain
                    .as_ref()
                    .ok_or("DivisionStatus.attack terrain domain was refused when recorded")?;
                known(
                    rows::attack_terrain(domain, terrain.index)?,
                    "DivisionStatus.attack terrain value",
                )
            }
        }
    }

    fn commander(&self, target: u32) -> Fact<Option<Commander>> {
        let Some(group) = self.orders_group()? else {
            return Ok(None);
        };
        match &group.commander_attack {
            rows::Recorded::Value(commander) => {
                let Some(commander) = known(
                    commander.as_ref(),
                    "OrdersGroupCounterInputs.commander_attack",
                )?
                else {
                    return Ok(None);
                };
                let terrain = self
                    .context
                    .terrain(target)
                    .and_then(|terrain| commander_terrain(&commander.terrain, terrain.index));
                Ok(Some(Commander {
                    terrain,
                    river: Ok(commander.river),
                    fort: Ok(commander.fort),
                    snow: Ok(commander.snow),
                    night: Ok(commander.night),
                }))
            }
            rows::Recorded::NotRecorded => {
                let packet = group
                    .attack
                    .require("OrdersGroupCounterInputs.attack")?
                    .get()
                    .ok_or("OrdersGroupCounterInputs.attack holds no operands")?;
                fresh(
                    self.frame,
                    packet.sample_hour,
                    "OrdersGroupCounterInputs.attack",
                )?;
                let Some(commander) = known(
                    packet.commander.as_ref(),
                    "OrdersGroupCounterInputs.attack commander",
                )?
                else {
                    return Ok(None);
                };
                let terrain = self.context.terrain(target).and_then(|terrain| {
                    let domain = commander
                        .terrain
                        .as_ref()
                        .ok_or("the commander's terrain domain was refused when recorded")?;
                    known(
                        rows::attack_terrain(domain, terrain.index)?,
                        "commander terrain value",
                    )
                });
                Ok(Some(Commander {
                    terrain,
                    river: known(commander.river, "commander river attack"),
                    fort: known(commander.fort, "commander fort attack"),
                    snow: known(commander.snow, "commander snow attack"),
                    night: known(commander.night, "commander night attack"),
                }))
            }
        }
    }

    fn crossing(&self, from: u32, to: u32) -> Fact<Option<u8>> {
        self.context.crossing(from, to)
    }

    fn has_snow(&self, target: u32) -> Fact<bool> {
        // A province outside the weather records has no weather record.
        let Some(weather) = self.weather_province(target)? else {
            return Ok(false);
        };
        let ceiling = complete::<tables::WeatherRegions>(self.frame)?
            .first()
            .ok_or("WeatherRegions holds no snow ceiling")?
            .max_snow;
        Ok(has_snow(weather.snow, weather.permanent_snow, ceiling))
    }

    fn scalars(&self) -> Fact<Scalars> {
        match self.stats()? {
            Some(stats) => Ok(Scalars {
                river: stats.river.map(Ok),
                fort: stats.fort.map(Ok),
                snow: Ok(stats.snow),
                night: Ok(stats.night),
            }),
            None => {
                let stats = self.operands()?.stats;
                let field = |value: Option<i64>, name: &str| known(value, name);
                Ok(Scalars {
                    river: [
                        field(stats.river[0], "division river attack"),
                        field(stats.river[1], "division river breakthrough"),
                    ],
                    fort: [
                        field(stats.fort[0], "division fort attack"),
                        field(stats.fort[1], "division fort breakthrough"),
                    ],
                    snow: field(stats.snow, "division snow attack"),
                    night: field(stats.night, "division night attack"),
                })
            }
        }
    }

    fn defines(&self) -> Fact<Defines> {
        let [row] = complete::<tables::SimulationFeatures>(self.frame)? else {
            return Err("SimulationFeatures has no complete singleton".into());
        };
        let scalars = row
            .runtime_scalars
            .require("SimulationFeatures.runtime_scalars")?;
        fresh(
            self.frame,
            scalars.sample_hour,
            "SimulationFeatures.runtime_scalars",
        )?;
        Ok(Defines {
            river_small: known(scalars.river_small, "RIVER_CROSSING_PENALTY"),
            river_large: known(scalars.river_large, "RIVER_CROSSING_PENALTY_LARGE"),
            night: known(scalars.night, "BASE_NIGHT_ATTACK_PENALTY"),
            fort: known(scalars.fort, "BASE_FORT_PENALTY"),
        })
    }

    fn fort_level(&self, target: u32) -> Fact<i32> {
        let province = self.province(target)?;
        if let rows::Recorded::Value(_) = province.fort_bunker {
            // The province's active buildings, healthy level times land_fort.
            let rows = complete::<tables::ProvinceBuildings>(self.frame)?;
            let first = rows.partition_point(|row| row.location < target);
            let mut level = 0_i32;
            for building in rows[first..]
                .iter()
                .take_while(|row| row.location == target)
            {
                if building.active {
                    level = level.wrapping_add(
                        i32::from(building.healthy_level)
                            .wrapping_mul(self.context.land_fort(building.building_type)?),
                    );
                }
            }
            return Ok(level);
        }
        let packet = province
            .attack
            .require("Provinces.attack")?
            .get()
            .ok_or("Provinces.attack holds no operands")?;
        fresh(self.frame, packet.sample_hour, "Provinces.attack")?;
        known(packet.fort_sum, "Provinces.attack fort sum")
    }

    fn fort_base(&self, target: u32) -> Fact<i64> {
        let province = self.province(target)?;
        let define = self.defines()?.fort?;
        // The controller passed the outer gate, so its reference resolves a country.
        let country = self.country_scalar(
            province.controller,
            engine_executable::modifier_keys::LAND_BUNKER_EFFECTIVENESS_FACTOR,
            crate::fixed_layout::SELECTED_ATTACK_FORT,
            |modifiers| modifiers.land_bunker_effectiveness_factor,
            |packet| packet.land_bunker_effectiveness_factor,
        )?;
        let state = match &province.fort_bunker {
            rows::Recorded::Value(bunker) => known(*bunker, "Provinces.fort_bunker")?,
            rows::Recorded::NotRecorded => {
                let packet = province
                    .attack
                    .require("Provinces.attack")?
                    .get()
                    .ok_or("Provinces.attack holds no operands")?;
                known(
                    packet.land_bunker_effectiveness_factor,
                    "Provinces.attack state bunker factor",
                )?
            }
        };
        Ok(fort_base(Some(country), state, define))
    }

    fn region_weather(&self, target: u32) -> Fact<Option<[i64; 2]>> {
        let region = self
            .context
            .region(target)?
            .ok_or_else(|| format!("province {target} has no strategic region"))?;
        // An id outside the region array has no weather modifier.
        let Some(row) = self.weather_region(region)? else {
            return Ok(None);
        };
        match &row.combat {
            rows::Recorded::Value(combat) => {
                let combat = known(combat.as_ref(), "WeatherRegions.combat")?;
                Ok(Some([
                    self.effective(
                        engine_executable::modifier_keys::ARMY_ATTACK_FACTOR,
                        combat.army_attack_factor,
                        crate::fixed_layout::SELECTED_ATTACK_WEATHER_ATTACK,
                    )?,
                    self.effective(
                        engine_executable::modifier_keys::BREAKTHROUGH_FACTOR,
                        combat.breakthrough_factor,
                        crate::fixed_layout::SELECTED_ATTACK_WEATHER_BREAKTHROUGH,
                    )?,
                ]))
            }
            rows::Recorded::NotRecorded => {
                let packet = row
                    .attack
                    .require("WeatherRegions.attack")?
                    .get()
                    .ok_or("WeatherRegions.attack holds no operands")?;
                fresh(self.frame, packet.sample_hour, "WeatherRegions.attack")?;
                crate::fixed_layout::validate_recording(
                    self.frame,
                    crate::fixed_layout::SELECTED_ATTACK_WEATHER_ATTACK,
                )?;
                crate::fixed_layout::validate_recording(
                    self.frame,
                    crate::fixed_layout::SELECTED_ATTACK_WEATHER_BREAKTHROUGH,
                )?;
                Ok(Some([
                    known(packet.attack, "region weather attack")?,
                    known(packet.breakthrough, "region weather breakthrough")?,
                ]))
            }
        }
    }

    fn weather_factor(&self) -> Fact<i64> {
        match &self.status()?.weather_factor {
            rows::Recorded::Value(factor) => known(*factor, "DivisionStatus.weather_factor"),
            rows::Recorded::NotRecorded => known(
                self.operands()?.acclimatization,
                "DivisionStatus.attack acclimatization factor",
            ),
        }
    }

    fn night_factor(&self, target: u32) -> Fact<i64> {
        self.context.night_factor(target)
    }

    fn night_modifier(&self) -> Fact<i64> {
        let division = self.division()?;
        let owner = if (division.expeditionary_owner as i32) > 0 {
            division.expeditionary_owner
        } else {
            division.roster_owner
        };
        self.country_scalar(
            owner,
            engine_executable::modifier_keys::LAND_NIGHT_ATTACK,
            crate::fixed_layout::SELECTED_ATTACK_NIGHT,
            |modifiers| modifiers.land_night_attack,
            |packet| packet.land_night_attack,
        )
    }

    fn enemy_air(&self) -> Fact<bool> {
        let division = self.division()?;
        // A location without a strategic region adds no entry.
        let Some(region) = self.context.region(division.province)? else {
            return Ok(false);
        };
        let owner = self.slot(division.logical_owner)?;
        let mut present = false;
        let mut enemy = 0;
        for row in complete::<tables::RaidAirScores>(self.frame)? {
            if self.slot(row.country).ok() != Some(owner) {
                continue;
            }
            present = true;
            if row.region == region {
                enemy = row.enemy;
            }
        }
        // An absent owner has no strategic-air object; an absent pair is zero.
        Ok(present && enemy > 0)
    }
}

#[cfg(test)]
#[path = "attack_derive/tests.rs"]
mod tests;
