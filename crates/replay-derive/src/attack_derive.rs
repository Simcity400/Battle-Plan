
use crate::{
    frame::DecodedFrame,
    rows::{self, TableSchema, tables},
};

pub type Fact<T> = Result<T, String>;

pub const ONE: i64 = 100_000;
pub const TERRAIN: u32 = 0x1b;
pub const RIVER: u32 = 0x0a;
pub const FORT: u32 = 0x09;
pub const WEATHER: u32 = 0x1c;
pub const NIGHT: u32 = 0x1d;
pub const ENEMY_AIR: u32 = 0x0e;
pub const NIGHT_THRESHOLD: i64 = 80_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub kind: u32,
    pub value: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    pub rows: Vec<Row>,
    pub refusals: Vec<(&'static str, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scalars {
    pub river: [Fact<i64>; 2],
    pub fort: [Fact<i64>; 2],
    pub snow: Fact<i64>,
    pub night: Fact<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commander {
    pub terrain: Fact<i64>,
    pub river: Fact<i64>,
    pub fort: Fact<i64>,
    pub snow: Fact<i64>,
    pub night: Fact<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Defines {
    pub river_small: Fact<i64>,
    pub river_large: Fact<i64>,
    pub night: Fact<i64>,
    pub fort: Fact<i64>,
}

pub trait Inputs {
    fn controller(&self, province: u32) -> Fact<u32>;
    fn same_country(&self, first: u32, second: u32) -> Fact<bool>;
    fn at_war(&self, target_controller: u32) -> Fact<bool>;
    fn terrain_attack(&self, target: u32) -> Fact<i64>;
    fn commander(&self, target: u32) -> Fact<Option<Commander>>;
    fn crossing(&self, from: u32, to: u32) -> Fact<Option<u8>>;
    fn has_snow(&self, target: u32) -> Fact<bool>;
    fn scalars(&self) -> Fact<Scalars>;
    fn defines(&self) -> Fact<Defines>;
    fn fort_level(&self, target: u32) -> Fact<i32>;
    fn fort_base(&self, target: u32) -> Fact<i64>;
    fn region_weather(&self, target: u32) -> Fact<Option<[i64; 2]>>;
    fn weather_factor(&self) -> Fact<i64>;
    fn night_factor(&self, target: u32) -> Fact<i64>;
    fn night_modifier(&self) -> Fact<i64>;
    fn enemy_air(&self) -> Fact<bool>;
}

fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}

pub fn river(unit: [i64; 2], leader: i64, define: i64) -> [i64; 2] {
    let adjustment = mul(define.wrapping_add(leader), ONE);
    unit.map(|value| value.wrapping_add(adjustment))
}

pub fn fort_base(country: Option<i64>, state: i64, define: i64) -> i64 {
    match country {
        None => define,
        Some(country) => mul(country.wrapping_add(ONE).wrapping_add(state), define),
    }
}

pub fn fort(unit: [i64; 2], leader: i64, level: i32, base: i64) -> Option<[i64; 2]> {
    if i64::from(level) * ONE <= 0 {
        return None;
    }
    let levels = (i64::from(level) * ONE).max(ONE);
    let factor = mul(mul(levels, base), ONE).wrapping_add(leader);
    Some(unit.map(|value| mul(value.wrapping_add(factor).min(0), ONE)))
}

pub fn weather(pair: [i64; 2], factor: i64) -> [i64; 2] {
    pair.map(|value| value.wrapping_mul(factor) / ONE)
}

pub fn night(stat: i64, define: i64, country: i64, leader: i64) -> i64 {
    stat.wrapping_add(define)
        .wrapping_add(country)
        .wrapping_add(leader)
}

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

pub trait Context {
    fn terrain(&self, province: u32) -> Fact<Terrain>;
    fn crossing(&self, from: u32, to: u32) -> Fact<Option<u8>>;
    fn region(&self, province: u32) -> Fact<Option<u32>>;
    fn land_fort(&self, building_type: u32) -> Fact<i32>;
    fn night_factor(&self, province: u32) -> Fact<i64>;
    fn at_war(&self, receiver: u32, target: u32) -> Fact<bool>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Terrain {
    pub index: i32,
    pub attack: i64,
}

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

    fn stats(&self) -> Fact<Option<&'a rows::DivisionAttackStats>> {
        match &self.status()?.attack_stats {
            rows::Recorded::NotRecorded => Ok(None),
            rows::Recorded::Value(stats) => {
                known(stats.as_ref().map(Some), "DivisionStatus.attack_stats")
            }
        }
    }

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
        Ok(present && enemy > 0)
    }
}

#[cfg(test)]
#[path = "attack_derive/tests.rs"]
mod tests;
