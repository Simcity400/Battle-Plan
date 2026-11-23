
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native::{
    carmy,
    carmy_leader,
    ccountry,
    cgame_state,
    corders_group,
    cprovince,
    crelation,
    cstate,
    cweather_manager::{self, province_weather as sweather_per_province},
};
use engine_native_politics::cdiplomacy_status;
use engine_native_parts::{cterrain_type, modifier_accumulator};
use crate::modifier_keys;
use engine_sites::Rva;

pub type Field<T> = Result<T, &'static str>;
pub const ONE: i64 = 100_000;
use modifier_keys::{
    ATTRITION, ATTRITION_FOR_CONTROLLER, ENEMY_ATTRITION, HEAT_ATTRITION, HEAT_ATTRITION_FACTOR,
    WINTER_ATTRITION, WINTER_ATTRITION_FACTOR,
};

pub const KEYS: [u32; 10] = [
    ATTRITION,
    WINTER_ATTRITION_FACTOR,
    WINTER_ATTRITION,
    HEAT_ATTRITION,
    HEAT_ATTRITION_FACTOR,
    modifier_keys::UNIT_UPKEEP_ATTRITION_FACTOR,
    modifier_keys::OUT_OF_SUPPLY_FACTOR,
    modifier_keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR,
    ATTRITION_FOR_CONTROLLER,
    ENEMY_ATTRITION,
];

pub const UNIT_UPKEEP_ATTRITION_RVA: Rva = Rva(0x033e_0008);
pub const OUT_OF_SUPPLY_ATTRITION_RVA: Rva = Rva(0x033e_3908);
pub const SUPPLY_THRESHOLD_RVA: Rva = Rva(0x033e_43a8);
pub const MAX_OUT_OF_SUPPLY_DAYS_RVA: Rva = Rva(0x033e_3844);
pub const SUPPLY_GRACE_RVA: Rva = Rva(0x033e_3678);
pub const TRAINING_ATTRITION_RVA: Rva = Rva(0x033e_0750);
pub const BORDER_WAR_ATTRITION_FACTOR_RVA: Rva = Rva(0x033e_4650);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Terrain {
    pub attrition: Field<i64>,
    pub equipment: Field<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Supply {
    pub penalty: Field<i64>,
    pub attrition_factor: Field<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thermal {
    pub base: i64,
    pub country_factor: Field<i64>,
    pub leader_factor: Field<Option<i64>>,
}

impl Thermal {
    pub fn evaluate(self) -> Field<i64> {
        Ok(product(
            self.base,
            ONE.wrapping_add(self.country_factor?)
                .wrapping_add(self.leader_factor?.unwrap_or(0)),
        )
        .max(0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Weather {
    pub acclimatization: Field<i64>,
    pub attrition: Field<i64>,
    pub heat: Field<Option<Thermal>>,
    pub winter: Field<Option<Thermal>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Border {
    pub state: Field<u32>,
    pub country: Field<u32>,
    pub factor: Field<i64>,
}

pub const SOURCE_COUNT: usize = 10;
pub const UPKEEP_SOURCE: usize = 0;
pub const STATE_SOURCE: usize = 1;
pub const WEATHER_SOURCE: usize = 2;
pub const HEAT_SOURCE: usize = 3;
#[cfg(test)]
pub const HEAT_COUNTRY_SOURCE: usize = 4;
#[cfg(test)]
pub const HEAT_LEADER_SOURCE: usize = 5;
pub const WINTER_SOURCE: usize = 6;
pub const WINTER_LEADER_SOURCE: usize = 8;
pub const COUNTRY_SOURCE: usize = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRequest {
    pub accumulator: usize,
    pub key: u32,
    pub depth: i32,
    pub scale: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inputs {
    pub province: Field<Option<u32>>,
    pub upkeep: Field<i64>,
    pub terrain: Field<Option<Terrain>>,
    pub state: Field<Option<i64>>,
    pub supply: Field<Supply>,
    pub country_upkeep_modifier: Field<i64>,
    pub moving_state: Field<Option<i64>>,
    pub weather: Field<Option<Weather>>,
    pub training: Field<bool>,
    pub training_attrition: Field<Option<i64>>,
    pub border: Field<Option<Border>>,
    pub country_attrition_modifier: Field<i64>,
    pub sources: [Field<Option<SourceRequest>>; SOURCE_COUNT],
}

impl Inputs {
    fn skipped() -> Self {
        Self {
            province: Ok(None),
            upkeep: Ok(0),
            terrain: Ok(None),
            state: Ok(None),
            supply: Ok(Supply {
                penalty: Ok(0),
                attrition_factor: Ok(0),
            }),
            country_upkeep_modifier: Ok(0),
            moving_state: Ok(None),
            weather: Ok(None),
            training: Ok(false),
            training_attrition: Ok(None),
            border: Ok(None),
            country_attrition_modifier: Ok(0),
            sources: [Ok(None); SOURCE_COUNT],
        }
    }

    fn failed(reason: &'static str) -> Self {
        Self {
            province: Err(reason),
            upkeep: Err(reason),
            terrain: Err(reason),
            state: Err(reason),
            supply: Err(reason),
            country_upkeep_modifier: Err(reason),
            moving_state: Err(reason),
            weather: Err(reason),
            training: Err(reason),
            training_attrition: Err(reason),
            border: Err(reason),
            country_attrition_modifier: Err(reason),
            sources: [Err(reason); SOURCE_COUNT],
        }
    }

    pub fn evaluate(self, border_ratio: Option<i64>) -> Field<i64> {
        if self.province?.is_none() {
            return Ok(0);
        }
        let mut total = self.upkeep?;
        if let Some(terrain) = self.terrain? {
            total = total.wrapping_add(terrain.attrition?);
            let source_equipment = terrain.equipment?;
            let floor = total.wrapping_neg();
            let equipment = if total < source_equipment.wrapping_neg() {
                floor
            } else {
                source_equipment
            };
            total = total.wrapping_add(equipment);
        }
        total = total.wrapping_add(self.state?.unwrap_or(0));
        let supply = self.supply?;
        let penalty = supply.penalty?;
        if penalty != 0 {
            total = total.wrapping_add(product(penalty, supply.attrition_factor?));
        }
        total = total.wrapping_add(self.moving_state?.unwrap_or(0));
        if let Some(weather) = self.weather? {
            let weather_total = weather
                .attrition?
                .wrapping_add(
                    weather
                        .heat?
                        .map(Thermal::evaluate)
                        .transpose()?
                        .unwrap_or(0),
                )
                .wrapping_add(
                    weather
                        .winter?
                        .map(Thermal::evaluate)
                        .transpose()?
                        .unwrap_or(0),
                );
            total = total.wrapping_add(weather_total);
        }
        total = total.wrapping_add(self.training_attrition?.unwrap_or(0));
        if let Some(border) = self.border? {
            border.state?;
            border.country?;
            total = total.wrapping_add(product(
                border_ratio.ok_or("attrition border ratio unavailable")?,
                border.factor?,
            ));
        }
        Ok(product(
            ONE.wrapping_add(self.country_attrition_modifier?).max(0),
            total,
        ))
    }
}

fn product(left: i64, right: i64) -> i64 {
    left.wrapping_mul(right) / ONE
}

fn fraction(numerator: i64, denominator: i64) -> Field<i64> {
    if denominator == 0
        || (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
    {
        return Ok(0xffff_ffff);
    }
    numerator
        .wrapping_mul(ONE)
        .checked_div(denominator)
        .ok_or("native attrition fraction traps")
}

fn at(pointer: usize, offset: usize) -> Field<usize> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .ok_or("attrition pointer invalid or overflow")
}

fn field<T: engine_native_memory::memory::Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Field<T> {
    read(memory, at(pointer, offset)?).ok_or("attrition field unreadable")
}

fn global<T: engine_native_memory::memory::Raw>(memory: &impl Memory, base: usize, key: Rva) -> Field<T> {
    let address = engine_sites::at(base, key).ok_or("attrition pointer invalid or overflow")?;
    read(memory, address).ok_or("attrition field unreadable")
}

fn identity(memory: &impl Memory, base: usize, pointer: usize, vtable: Rva) -> Field<()> {
    let expected = engine_sites::at(base, vtable).ok_or("attrition module address overflow")?;
    if field::<u64>(memory, pointer, 0)? != expected as u64 {
        return Err("attrition object identity mismatch");
    }
    Ok(())
}

fn modifier(memory: &impl Memory, base: usize, accumulator: usize, key: u32) -> Field<i64> {
    modifier_accumulator::requested_effective_at(memory, base, accumulator, key)
}

fn request(accumulator: usize, key: u32, depth: i32, scale: i64) -> SourceRequest {
    SourceRequest {
        accumulator,
        key,
        depth,
        scale,
    }
}

#[derive(Default)]
pub struct Memo {
    enabled: bool,
    modifiers: engine_native_parts::keyed_map::Map<(usize, u32), Field<i64>>,
    countries: engine_native_parts::keyed_map::Map<u32, Field<engine_native_memory::country_identity::Country>>,
    at_war: engine_native_parts::keyed_map::Map<usize, Field<bool>>,
    states: engine_native_parts::keyed_map::Map<(usize, u32), Field<i64>>,
    state_roots: engine_native_parts::keyed_map::Map<(usize, u32), Field<usize>>,
    weather: engine_native_parts::keyed_map::Map<u32, Field<Option<usize>>>,
    templates: engine_native_parts::keyed_map::Map<usize, Field<i64>>,
    requests: u64,
    hits: u64,
}

impl Memo {
    pub fn new() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }

    pub fn clear(&mut self) {
        self.modifiers.clear();
        self.countries.clear();
        self.at_war.clear();
        self.states.clear();
        self.state_roots.clear();
        self.weather.clear();
        self.templates.clear();
        self.requests = 0;
        self.hits = 0;
    }

    pub fn counts(&self) -> (u64, u64) {
        (self.requests, self.hits)
    }

    pub fn allocated_bytes(&self) -> usize {
        fn bytes<K, V>(map: &engine_native_parts::keyed_map::Map<K, V>) -> usize {
            map.capacity() * (std::mem::size_of::<(K, V)>() + 1)
        }
        bytes(&self.modifiers)
            + bytes(&self.countries)
            + bytes(&self.at_war)
            + bytes(&self.states)
            + bytes(&self.state_roots)
            + bytes(&self.weather)
            + bytes(&self.templates)
    }
}

fn remember<K: Copy + Eq + std::hash::Hash, V: Copy>(
    enabled: bool,
    counts: (&mut u64, &mut u64),
    map: &mut engine_native_parts::keyed_map::Map<K, Field<V>>,
    key: K,
    read: impl FnOnce() -> Field<V>,
) -> Field<V> {
    if !enabled {
        return read();
    }
    *counts.0 += 1;
    if let Some(&value) = map.get(&key) {
        *counts.1 += 1;
        return value;
    }
    let value = read();
    if crate::allocation::reserve_map(map, 1).is_ok() {
        map.insert(key, value);
    }
    value
}

impl Memo {
    fn modifier(
        &mut self,
        memory: &impl Memory,
        base: usize,
        accumulator: usize,
        key: u32,
    ) -> Field<i64> {
        remember(
            self.enabled,
            (&mut self.requests, &mut self.hits),
            &mut self.modifiers,
            (accumulator, key),
            || modifier(memory, base, accumulator, key),
        )
    }

    fn country(
        &mut self,
        raw: u32,
        read: impl FnOnce() -> Field<engine_native_memory::country_identity::Country>,
    ) -> Field<engine_native_memory::country_identity::Country> {
        remember(
            self.enabled,
            (&mut self.requests, &mut self.hits),
            &mut self.countries,
            raw,
            read,
        )
    }

    fn at_war(&mut self, memory: &impl Memory, base: usize, country: usize) -> Field<bool> {
        remember(
            self.enabled,
            (&mut self.requests, &mut self.hits),
            &mut self.at_war,
            country,
            || at_war(memory, base, country),
        )
    }

    fn state_root(
        &mut self,
        memory: &impl Memory,
        game_state: usize,
        state: usize,
        owner: u32,
    ) -> Field<usize> {
        remember(
            self.enabled,
            (&mut self.requests, &mut self.hits),
            &mut self.state_roots,
            (state, owner),
            || cstate::country_modifier_accumulator(memory, game_state, state, owner),
        )
    }

    fn weather(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        id: u32,
    ) -> Field<Option<usize>> {
        remember(
            self.enabled,
            (&mut self.requests, &mut self.hits),
            &mut self.weather,
            id,
            || weather_accumulator(memory, base, game_state, id),
        )
    }

    fn template_ratio(&mut self, memory: &impl Memory, base: usize, template: usize) -> Field<i64> {
        remember(
            self.enabled,
            (&mut self.requests, &mut self.hits),
            &mut self.templates,
            template,
            || template_ratio(memory, base, template),
        )
    }
}

pub fn capture(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    seed: &carmy::DivisionSeed,
    exercising: Field<u8>,
) -> Inputs {
    capture_with_acclimatization(memory, base, game_state, seed, exercising, None)
}

pub fn capture_with_acclimatization(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    seed: &carmy::DivisionSeed,
    exercising: Field<u8>,
    fresh: Option<Field<i64>>,
) -> Inputs {
    capture_memoized(
        memory,
        base,
        game_state,
        seed,
        exercising,
        fresh,
        &mut Memo::default(),
    )
}

pub fn capture_memoized(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    seed: &carmy::DivisionSeed,
    exercising: Field<u8>,
    fresh: Option<Field<i64>>,
    memo: &mut Memo,
) -> Inputs {
    let primary = engine_sites::at(base, carmy::VTABLE_RVA).map(|p| p as u64);
    let secondary = engine_sites::at(base, carmy::SECONDARY_VTABLE_RVA).map(|p| p as u64);
    if primary.is_none()
        || secondary.is_none()
        || seed.primary_vtable != primary
        || seed.secondary_vtable != secondary
        || seed.handle != Some(seed.division)
        || seed.division == 0
        || seed.class != Some(0)
    {
        return Inputs::failed("attrition army seed identity unavailable");
    }
    let route = route(seed);
    let selected = selected_province(memory, base, game_state, seed, route);
    let province = match selected {
        Ok(0) => return Inputs::skipped(),
        Ok(province) => province,
        Err(reason) => return Inputs::failed(reason),
    };
    let mut output = Inputs::skipped();
    output.province = field(memory, province, cprovince::ID).map(Some);
    let countries = ccountry::Reader::new(memory, base, game_state);
    let owner = seed
        .expeditionary_owner
        .ok_or("attrition expeditionary owner unavailable")
        .and_then(|raw| {
            if (raw as i32) > 0 {
                Ok(raw)
            } else {
                seed.roster_owner
                    .ok_or("attrition roster owner unavailable")
            }
        });
    let country = owner.and_then(|raw| {
        let countries = countries.as_ref().map_err(|reason| *reason)?;
        memo.country(raw, || countries.resolve(raw))
    });
    let country_accumulator =
        country.and_then(|country| at(country.pointer, engine_native_memory::country_identity::MODIFIER_ACCUMULATOR));
    let upkeep_modifier = country_accumulator.and_then(|root| {
        memo.modifier(
            memory,
            base,
            root,
            modifier_keys::UNIT_UPKEEP_ATTRITION_FACTOR,
        )
    });
    output.country_upkeep_modifier = upkeep_modifier;
    output.upkeep = upkeep_modifier.and_then(|value| {
        Ok(product(
            ONE.wrapping_add(value).max(0),
            global(memory, base, UNIT_UPKEEP_ATTRITION_RVA)?,
        ))
    });
    output.sources[UPKEEP_SOURCE] = upkeep_modifier.and_then(|value| {
        if value == 0 {
            Ok(None)
        } else {
            Ok(Some(request(
                country_accumulator?,
                modifier_keys::UNIT_UPKEEP_ATTRITION_FACTOR,
                -1,
                ONE,
            )))
        }
    });
    let moving = route.and_then(|(count, _)| {
        if count <= 0 {
            return Ok(false);
        }
        seed.strategic_redeployment
            .map(|value| value == 0)
            .ok_or("attrition strategic redeployment unavailable")
    });
    output.terrain = moving.and_then(|moving| {
        if !moving {
            return Ok(None);
        }
        terrain(memory, base, seed.pointer, province, memo)
    });
    let state = (|| {
        let state = field::<u64>(memory, province, cprovince::STATE)? as usize;
        if state == 0 {
            return Ok(None);
        }
        identity(memory, base, state, cstate::VTABLE_RVA)?;
        Ok(Some(state))
    })();
    output.state = state.and_then(|state| {
        let Some(state) = state else {
            return Ok(None);
        };
        let roster = seed
            .roster_owner
            .ok_or("attrition roster owner unavailable")?;
        remember(
            memo.enabled,
            (&mut memo.requests, &mut memo.hits),
            &mut memo.states,
            (state, roster),
            || {
                let root = cstate::country_modifier_accumulator(memory, game_state, state, roster)?;
                let mut value = modifier(memory, base, root, ATTRITION)?;
                let controller = field(memory, state, cstate::CONTROLLER)?;
                if ccountry::movement_friendly(memory, base, game_state, roster, controller)? {
                    value =
                        value.wrapping_add(modifier(memory, base, root, ATTRITION_FOR_CONTROLLER)?);
                } else if ccountry::movement_enemy(memory, base, game_state, roster, controller)? {
                    value = value.wrapping_add(modifier(memory, base, root, ENEMY_ATTRITION)?);
                }
                Ok(value)
            },
        )
        .map(Some)
    });
    output.supply = Ok(Supply {
        penalty: supply_penalty(memory, base, seed, country_accumulator, memo),
        attrition_factor: global(memory, base, OUT_OF_SUPPLY_ATTRITION_RVA),
    });

    let weather_gate = moving.and_then(|moving| {
        if !moving {
            return Ok(false);
        }
        let raw = seed
            .logical_owner
            .ok_or("attrition logical owner unavailable")?;
        let countries = countries.as_ref().map_err(|reason| *reason)?;
        let country = memo.country(raw, || countries.resolve(raw))?;
        memo.at_war(memory, base, country.pointer)
    });
    let moving_root = weather_gate.and_then(|gate| {
        if !gate {
            return Ok(None);
        }
        let Some(state) = state? else {
            return Ok(None);
        };
        let raw = seed
            .logical_owner
            .ok_or("attrition logical owner unavailable")?;
        memo.state_root(memory, game_state, state, raw).map(Some)
    });
    output.moving_state = moving_root.and_then(|root| {
        root.map(|root| memo.modifier(memory, base, root, ATTRITION))
            .transpose()
    });
    output.sources[STATE_SOURCE] = output.moving_state.and_then(|value| {
        if value.is_none_or(|value| value == 0) {
            return Ok(None);
        }
        Ok(Some(request(
            moving_root?.ok_or("attrition moving state root unavailable")?,
            ATTRITION,
            -1,
            ONE,
        )))
    });
    let weather_root = weather_gate.and_then(|gate| {
        if !gate {
            return Ok(None);
        }
        memo.weather(
            memory,
            base,
            game_state,
            output.province?.ok_or("attrition province absent")?,
        )
    });
    output.weather = weather_root.map(|root| {
        let root = root?;
        let acclimatization = fresh.unwrap_or_else(|| fresh_acclimatization(memory, base, seed));
        let attrition = memo.modifier(memory, base, root, ATTRITION);
        output.sources[WEATHER_SOURCE] =
            attrition.map(|value| (value != 0).then(|| request(root, ATTRITION, -1, ONE)));
        let heat = thermal(
            memory,
            base,
            seed,
            root,
            country_accumulator,
            acclimatization,
            HEAT_ATTRITION,
            HEAT_ATTRITION_FACTOR,
            HEAT_SOURCE,
            &mut output.sources,
            memo,
        );
        let winter = thermal(
            memory,
            base,
            seed,
            root,
            country_accumulator,
            acclimatization,
            WINTER_ATTRITION,
            WINTER_ATTRITION_FACTOR,
            WINTER_SOURCE,
            &mut output.sources,
            memo,
        );
        Some(Weather {
            acclimatization,
            attrition,
            heat,
            winter,
        })
    });
    if let Err(reason) = weather_root {
        for source in &mut output.sources[WEATHER_SOURCE..=WINTER_LEADER_SOURCE] {
            *source = Err(reason);
        }
    }
    output.training = exercising.and_then(|exercising| {
        if exercising == 0 {
            return Ok(false);
        }
        route.map(|(count, _)| count < 1)
    });
    output.training_attrition = output.training.and_then(|training| {
        if training {
            global(memory, base, TRAINING_ATTRITION_RVA).map(Some)
        } else {
            Ok(None)
        }
    });
    output.border = state.and_then(|state| {
        let Some(state) = state else {
            return Ok(None);
        };
        if field::<u8>(memory, state, 0x865)? == 0 {
            return Ok(None);
        }
        Ok(Some(Border {
            state: field(memory, state, cstate::ID),
            country: seed
                .logical_owner
                .ok_or("attrition logical owner unavailable"),
            factor: global(memory, base, BORDER_WAR_ATTRITION_FACTOR_RVA),
        }))
    });
    output.country_attrition_modifier =
        country_accumulator.and_then(|root| memo.modifier(memory, base, root, ATTRITION));
    output.sources[COUNTRY_SOURCE] =
        country_accumulator.map(|root| Some(request(root, ATTRITION, 2, ONE)));
    output
}

fn route(seed: &carmy::DivisionSeed) -> Field<(i32, [u8; 16])> {
    let header = seed.ordinary_route.ok_or("attrition route unavailable")?;
    Ok((
        i32::from_le_bytes(header[12..16].try_into().unwrap()),
        header,
    ))
}

fn selected_province(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    seed: &carmy::DivisionSeed,
    route: Field<(i32, [u8; 16])>,
) -> Field<usize> {
    let (count, header) = route?;
    let mut province = 0;
    if count > 0 {
        let route = Array::from_header(header, 4).ok_or("attrition route invalid")?;
        let id = read::<u32>(memory, route.data).ok_or("attrition first route id unreadable")?;
        let provinces = Array::read(memory, at(game_state, cgame_state::PROVINCES)?, 8)
            .ok_or("attrition province roster invalid")?;
        province = read::<u64>(
            memory,
            provinces
                .element(id, 8)
                .ok_or("attrition route province outside roster")?,
        )
        .ok_or("attrition route province unreadable")? as usize;
        if province != 0 && field::<u32>(memory, province, cprovince::ID)? != id {
            return Err("attrition route province identity mismatch");
        }
    }
    if province == 0 {
        province = seed.location.ok_or("attrition location unavailable")?;
    }
    if province != 0 {
        identity(memory, base, province, cprovince::VTABLE_RVA)?;
    }
    Ok(province)
}

fn terrain(
    memory: &impl Memory,
    base: usize,
    army: usize,
    province: usize,
    memo: &mut Memo,
) -> Field<Option<Terrain>> {
    let definition = carmy::movement_definition(memory, base, province)?;
    let terrain = field::<u64>(memory, definition, engine_native::cprovince::definition::TERRAIN)? as usize;
    let valid = field::<u8>(memory, terrain, cterrain_type::VALID)?;
    if valid == 0 {
        return Ok(None);
    }
    identity(memory, base, terrain, cterrain_type::VTABLE_RVA)?;
    let equipment = (|| {
        let index = field::<i32>(memory, terrain, cterrain_type::DATABASE_INDEX)?;
        if index < 0 {
            return Err("attrition terrain index negative");
        }
        let entries = Array::read(memory, at(army, carmy::EQUIPMENT_DATA)?, 16)
            .ok_or("attrition equipment bank invalid")?;
        let mut equipment = 0_i64;
        for ordinal in 0..entries.count {
            let entry = entries
                .element(ordinal, 16)
                .ok_or("attrition equipment index invalid")?;
            if field::<i64>(memory, entry, 8)? == 0 {
                continue;
            }
            let variant = field::<u64>(memory, entry, 0)? as usize;
            identity(memory, base, variant, engine_native_parts::equipment_variant_identity::VTABLE_RVA)?;
            let values = field::<u64>(memory, variant, 0x3a0)? as usize;
            equipment = equipment.wrapping_add(field(memory, values, index as usize * 8)?);
        }
        Ok(equipment)
    })();
    Ok(Some(Terrain {
        attrition: at(terrain, 0xb8).and_then(|root| memo.modifier(memory, base, root, ATTRITION)),
        equipment,
    }))
}

fn at_war(memory: &impl Memory, base: usize, country: usize) -> Field<bool> {
    let diplomacy = field::<u64>(memory, country, ccountry::DIPLOMACY_STATUS)? as usize;
    identity(memory, base, diplomacy, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA)?;
    if field::<i32>(memory, diplomacy, cdiplomacy_status::ENEMY_COUNTRY_COUNT)? != 0 {
        return Ok(true);
    }
    let relations = Array::read(
        memory,
        at(diplomacy, engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS)?,
        8,
    )
    .ok_or("attrition owned relations invalid")?;
    for ordinal in 0..relations.count {
        let relation = field::<u64>(
            memory,
            relations
                .element(ordinal, 8)
                .ok_or("attrition relation index invalid")?,
            0,
        )? as usize;
        if field::<u32>(memory, relation, crelation::TYPE_ID)? == crelation::TENSION_CAP_TYPE {
            return Ok(true);
        }
    }
    Ok(false)
}

fn weather_accumulator(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    id: u32,
) -> Field<Option<usize>> {
    let manager = field::<u64>(memory, game_state, cgame_state::WEATHER_MANAGER)? as usize;
    identity(memory, base, manager, cweather_manager::VTABLE_RVA)?;
    let count = field::<i32>(memory, manager, cweather_manager::PROVINCES_COUNT)?;
    if (id as i32) < 0 || (id as i32) >= count {
        return Ok(None);
    }
    let data = field::<u64>(memory, manager, cweather_manager::PROVINCES)? as usize;
    let record = at(data, id as usize * sweather_per_province::STRIDE)?;
    identity(memory, base, record, sweather_per_province::VTABLE_RVA)?;
    if field::<u32>(memory, record, sweather_per_province::ID)? != id {
        return Err("attrition weather identity mismatch");
    }
    at(record, sweather_per_province::MODIFIERS).map(Some)
}

pub fn fresh_acclimatization(
    memory: &impl Memory,
    base: usize,
    seed: &carmy::DivisionSeed,
) -> Field<i64> {
    let header = seed
        .acclimatization
        .ok_or("attrition acclimatization unavailable")?;
    if u64::from_le_bytes(header[..8].try_into().unwrap())
        != engine_sites::at(base, engine_native_parts::acclimatization_data::VTABLE_RVA)
            .ok_or("attrition module address overflow")? as u64
    {
        return Err("attrition acclimatization identity mismatch");
    }
    let active = u64::from_le_bytes(header[0x20..0x28].try_into().unwrap()) as usize;
    if active == 0 {
        return Ok(ONE);
    }
    let entries = Array::from_header(header[8..0x18].try_into().unwrap(), 16)
        .ok_or("attrition acclimatization array invalid")?;
    let mut chosen = 0;
    let mut weight = 0_i64;
    for ordinal in 0..entries.count {
        let entry = entries
            .element(ordinal, 16)
            .ok_or("attrition acclimatization index invalid")?;
        let value = field::<i64>(memory, entry, 8)?;
        if value > weight {
            weight = value;
            chosen = field::<u64>(memory, entry, 0)? as usize;
        }
    }
    if chosen != active {
        return Ok(ONE);
    }
    let maximum = i64::from_le_bytes(header[0x38..0x40].try_into().unwrap());
    let scaled = product(fraction(weight, maximum)?, field(memory, active, 0x30)?);
    Ok(if scaled < 0 {
        ONE
    } else {
        ONE - scaled.min(ONE)
    })
}

#[allow(clippy::too_many_arguments)]
fn thermal(
    memory: &impl Memory,
    base: usize,
    seed: &carmy::DivisionSeed,
    weather: usize,
    country: Field<usize>,
    acclimatization: Field<i64>,
    key: u32,
    factor_key: u32,
    source: usize,
    sources: &mut [Field<Option<SourceRequest>>; SOURCE_COUNT],
    memo: &mut Memo,
) -> Field<Option<Thermal>> {
    let observed = acclimatization.and_then(|scale| {
        memo.modifier(memory, base, weather, key)
            .map(|value| (product(value, scale), scale))
    });
    let (value, scale) = match observed {
        Ok(observed) => observed,
        Err(reason) => {
            sources[source..source + 3].fill(Err(reason));
            return Err(reason);
        }
    };
    if value <= 0 {
        return Ok(None);
    }
    sources[source] = Ok(Some(request(weather, key, -1, scale)));
    sources[source + 1] = country.map(|country| Some(request(country, factor_key, -1, ONE)));
    let country_factor =
        country.and_then(|country| memo.modifier(memory, base, country, factor_key));
    let leader_root = (|| {
        let group = seed
            .orders_group
            .ok_or("attrition orders group unavailable")?;
        let leader = if group == 0 {
            0
        } else {
            let vtable = field::<u64>(memory, group, 0)?;
            let groups = [corders_group::VTABLE_RVA, engine_native_parts::carmy_group::VTABLE_RVA];
            if engine_sites::which(base, vtable as usize, &groups).is_none()
                || field::<u32>(memory, group, corders_group::HANDLE)?
                    != corders_group::REFERENCE_TYPE
            {
                return Err("attrition leader group identity mismatch");
            }
            field::<u64>(memory, group, corders_group::LEADER)? as usize
        };
        if leader == 0 {
            return Ok(None);
        }
        identity(memory, base, leader, carmy_leader::VTABLE_RVA)?;
        if field::<u32>(memory, leader, engine_native::cunit_leader::TYPE)?
            != engine_native::cunit_leader::REFERENCE_TYPE
        {
            return Err("attrition army leader reference type mismatch");
        }
        at(leader, 0x298).map(Some)
    })();
    sources[source + 2] =
        leader_root.map(|root| root.map(|root| request(root, factor_key, -1, ONE)));
    let leader_factor = leader_root.and_then(|root| {
        root.map(|root| memo.modifier(memory, base, root, factor_key))
            .transpose()
    });
    Ok(Some(Thermal {
        base: value,
        country_factor,
        leader_factor,
    }))
}

fn supply_penalty(
    memory: &impl Memory,
    base: usize,
    seed: &carmy::DivisionSeed,
    country: Field<usize>,
    memo: &mut Memo,
) -> Field<i64> {
    if seed
        .naval_transfer_membership
        .ok_or("attrition naval transfer unavailable")?
        != 0
    {
        return Ok(0);
    }
    let available = if field::<i64>(memory, seed.pointer, 0x600)? <= 0 {
        0
    } else {
        let grace = seed
            .supply_grace_remaining
            .ok_or("attrition remaining supply grace unavailable")?;
        let hours = i64::from(global::<i32>(memory, base, SUPPLY_GRACE_RVA)?).wrapping_mul(ONE);
        fraction(grace, hours)?
            .max(
                seed.counter_supply_status
                    .ok_or("attrition received supply unavailable")?,
            )
            .clamp(0, ONE)
    };
    let threshold = global::<i64>(memory, base, SUPPLY_THRESHOLD_RVA)?;
    let normalized = if threshold == 0 {
        ONE
    } else {
        fraction(available, threshold)?
    }
    .min(ONE);
    let days = i64::from(field::<i32>(memory, seed.pointer, 0x498)?).wrapping_mul(ONE);
    let maximum =
        i64::from(global::<i32>(memory, base, MAX_OUT_OF_SUPPLY_DAYS_RVA)?).wrapping_mul(ONE);
    let penalty = product(
        ONE.wrapping_sub(normalized).clamp(0, ONE),
        fraction(days, maximum)?,
    );
    if penalty == 0 {
        return Ok(penalty);
    }
    let ratio = memo.template_ratio(
        memory,
        base,
        seed.standard_template
            .ok_or("attrition standard template unavailable")?,
    )?;
    let country = country?;
    let mut multiplier = ONE.wrapping_add(memo.modifier(
        memory,
        base,
        country,
        modifier_keys::OUT_OF_SUPPLY_FACTOR,
    )?);
    if ratio > 0 {
        multiplier = multiplier.wrapping_add(product(
            ratio,
            memo.modifier(
                memory,
                base,
                country,
                modifier_keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR,
            )?,
        ));
    }
    if let Some(leader) = carmy_leader::movement_accumulator_seeded(memory, base, seed)? {
        multiplier = multiplier.wrapping_add(memo.modifier(
            memory,
            base,
            leader,
            modifier_keys::OUT_OF_SUPPLY_FACTOR,
        )?);
        if ratio > 0 {
            multiplier = multiplier.wrapping_add(product(
                ratio,
                memo.modifier(
                    memory,
                    base,
                    leader,
                    modifier_keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR,
                )?,
            ));
        }
    }
    Ok(product(multiplier.max(0), penalty))
}

fn template_ratio(memory: &impl Memory, base: usize, template: usize) -> Field<i64> {
    if template == 0 {
        return Ok(0);
    }
    identity(
        memory,
        base,
        template,
        engine_native::creferenced_division_template::VTABLE_RVA,
    )?;
    let entries = Array::read(memory, at(template, 0xc0)?, 16)
        .ok_or("attrition supply composition invalid")?;
    let mut total = 0_i32;
    let mut special = 0_i32;
    for ordinal in 0..entries.count {
        let entry = entries
            .element(ordinal, 16)
            .ok_or("attrition supply composition index invalid")?;
        let unit = field::<u64>(memory, entry, 0)? as usize;
        identity(memory, base, unit, engine_native_parts::csub_unit_definition::VTABLE_RVA)?;
        let count = field::<i32>(memory, entry, 8)?;
        total = total.wrapping_add(count);
        if field::<u8>(memory, unit, 0x5b7)? != 0 {
            special = special.wrapping_add(count);
        }
    }
    if total == 0 {
        Ok(0)
    } else {
        fraction(
            i64::from(special).wrapping_mul(ONE),
            i64::from(total).wrapping_mul(ONE),
        )
    }
}

#[cfg(test)]
mod tests;
