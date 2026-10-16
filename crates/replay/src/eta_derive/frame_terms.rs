//! The hour's recorded operands of the movement speed, read from a frame: the common
//! modifier, each edge's terms and the organisation and fuel factors ([`SpeedTerms`]),
//! with the content each reads given by a [`Geography`].

use super::{
    AirOperands, CommonModifiers, Defines, EdgeTerm, EdgeTerms, RailDefines, SpeedTerms,
    SupplyFactors, SupplyOperands,
};
use crate::{
    directed_war_derive::ReceiverWars,
    frame::DecodedFrame,
    rows::{
        CountryCounterInputRow, CountryLinkRow, CountryRow, DivisionMovementRow, DivisionRow,
        DivisionStatusRow, DivisionTemplateIconRow, LexicalTokenRow, ModifierDefinitionRow,
        MovementDefines as RecordedDefines, OrdersGroupCounterInputRow, ProvinceRow,
        RaidAirScoreRow, RailCooldownRow, RailEdgeRow, StateRow, StrategicAction, TableSchema,
        WeatherProvinceRow, WeatherRegionRow, tables,
    },
};
use engine::crailway_manager::fresh_inputs;
use std::{cell::RefCell, collections::BTreeMap};

/// The content's sub-unit facts the movement speed and the attrition hover read.
pub trait SubUnitContent {
    /// Whether the sub-unit definition `name` is special forces; an error names why it
    /// is unknown.
    fn special_forces(&self, name: &[u8]) -> Result<bool, String>;
    /// The sub-unit definition `name`'s type mask; an error names why it is unknown.
    fn sub_unit_types(&self, name: &[u8]) -> Result<u64, String>;
}

/// The sub-unit type mask's armour bit: a priority sub-unit with it takes the country's
/// and the commander block's armour speed.
pub const ARMOUR_TYPE: u64 = 4;

/// The content the land movement speed reads beyond the recording.
pub trait Geography: SubUnitContent {
    /// The province's one-based terrain index; `None` for a province the content lacks.
    fn terrain_index(&self, province: u32) -> Option<i32>;
    /// Whether the province is land (its definition's land bit); `None` for a province
    /// the content lacks.
    fn land(&self, province: u32) -> Option<bool>;
    /// The terrain's own unit adjuster movement for a one-based terrain index.
    fn terrain_movement(&self, index: i32) -> Option<i64>;
    /// The crossing kind of the move `from -> to` (1 sea, 2 river, 3 large river, 4
    /// impassable, otherwise 0); `None` for an unknown province.
    fn crossing_kind(&self, from: u32, to: u32) -> Option<u8>;
    /// The province's strategic region id; `None` for a province without one.
    fn region(&self, province: u32) -> Option<u32>;
}

/// The hour's recorded operands of the speed terms ([`SpeedTerms`]) and the attrition
/// hover's supply line.
pub struct FrameTerms<'a> {
    frame: &'a DecodedFrame,
    provinces: Result<&'a [ProvinceRow], String>,
    states: Result<&'a [StateRow], String>,
    status: Result<&'a [DivisionStatusRow], String>,
    links: Result<&'a [CountryLinkRow], String>,
    countries: Result<&'a [CountryRow], String>,
    counters: Result<&'a [CountryCounterInputRow], String>,
    groups: Result<&'a [OrdersGroupCounterInputRow], String>,
    weather_provinces: Result<&'a [WeatherProvinceRow], String>,
    weather_regions: Result<&'a [WeatherRegionRow], String>,
    definitions: Result<&'a [ModifierDefinitionRow], String>,
    wars: RefCell<BTreeMap<u32, Result<ReceiverWars, String>>>,
    air_scores: Result<&'a [RaidAirScoreRow], String>,
    rail_cooldowns: Result<&'a [RailCooldownRow], String>,
    templates: Result<&'a [DivisionTemplateIconRow], String>,
    tokens: Result<&'a [LexicalTokenRow], String>,
    rail_edges: Result<&'a [RailEdgeRow], String>,
    /// Each canonical owner slot's regional scores, built once.
    air_index: std::cell::OnceCell<Result<AirIndex, String>>,
    /// Each canonical slot's country id, built once: a frame derives every mover
    /// against it.
    country_ids: std::cell::OnceCell<Result<BTreeMap<u32, u32>, String>>,
    /// The movement family's recorded layout check, made once per frame.
    layout: std::cell::OnceCell<Result<(), String>>,
}

/// `[friendly, enemy, impact]` by canonical owner slot and strategic region id.
type AirIndex = BTreeMap<(u32, u32), [i64; 3]>;

/// Why an arrival time refuses a division in combat.
const IN_COMBAT: &str =
    "the division is in combat, and the combat's movement factor is not recorded";
/// Why an arrival time refuses a move to or from the sea.
pub const SEA_MOVE: &str =
    "the route moves to or from the sea, at the owner's transport speed, which is not recorded";
/// Why a division with no days out of supply refuses its out-of-supply term under a
/// supply-days define of zero.
pub const SUPPLY_DAYS_ZERO: &str = "the supply-days define is zero, so the out-of-supply penalty reads the received supply at zero days out of supply, which is recorded only at its display step";
/// Why the special-forces share refuses: the template's composition was unreadable when
/// recorded, so the hour holds none of its entries.
pub const UNREADABLE_COMPOSITION: &str = "the template's composition was unreadable when recorded, so its special-forces share is unknown";

/// The tables the speed terms and the attrition hover's supply line read, beyond the
/// division's own Divisions and DivisionMovement rows.
pub const TABLES: [crate::rows::TableId; 18] = {
    use crate::rows::TableId::*;
    [
        Provinces,
        States,
        DivisionStatus,
        CountryLinks,
        Countries,
        CountryCounterInputs,
        OrdersGroupCounterInputs,
        WeatherProvinces,
        WeatherRegions,
        ModifierDefinitions,
        LexicalTokens,
        SimulationFeatures,
        WarRelations,
        Clock,
        RaidAirScores,
        RailCooldowns,
        RailEdges,
        DivisionTemplateIcons,
    ]
};

/// Why an arrival time refuses a strategically redeploying division whose redeployment
/// inputs were not read.
pub const STRATEGIC_REDEPLOYMENT: &str =
    "the division is strategically redeploying, and its redeployment inputs were not recorded";

fn complete<T: TableSchema>(frame: &DecodedFrame) -> Result<&[T::Row], String> {
    frame.complete::<T>().map_err(|error| error.to_string())
}

impl<'a> FrameTerms<'a> {
    pub fn new(frame: &'a DecodedFrame) -> Self {
        Self {
            frame,
            provinces: complete::<tables::Provinces>(frame),
            states: complete::<tables::States>(frame),
            status: complete::<tables::DivisionStatus>(frame),
            links: complete::<tables::CountryLinks>(frame),
            countries: complete::<tables::Countries>(frame),
            counters: complete::<tables::CountryCounterInputs>(frame),
            groups: complete::<tables::OrdersGroupCounterInputs>(frame),
            weather_provinces: complete::<tables::WeatherProvinces>(frame),
            weather_regions: complete::<tables::WeatherRegions>(frame),
            definitions: complete::<tables::ModifierDefinitions>(frame),
            wars: RefCell::default(),
            air_scores: complete::<tables::RaidAirScores>(frame),
            rail_cooldowns: complete::<tables::RailCooldowns>(frame),
            templates: complete::<tables::DivisionTemplateIcons>(frame),
            tokens: complete::<tables::LexicalTokens>(frame),
            rail_edges: complete::<tables::RailEdges>(frame),
            air_index: std::cell::OnceCell::new(),
            country_ids: std::cell::OnceCell::new(),
            layout: std::cell::OnceCell::new(),
        }
    }

    fn province(&self, id: u32) -> Result<&'a ProvinceRow, String> {
        let rows = self.provinces.as_ref().map_err(Clone::clone)?;
        rows.binary_search_by_key(&id, |row| row.id)
            .map(|at| &rows[at])
            .map_err(|_| format!("Provinces has no row for province {id}"))
    }

    /// The movement infrastructure term of `province`'s state
    /// (`States.movement_infrastructure` of its `Provinces.state`), zero without a state.
    pub fn infrastructure(&self, province: u32) -> Result<i64, String> {
        let state = self.province(province)?.state;
        if state == 0 {
            return Ok(0);
        }
        let rows = self.states.as_ref().map_err(Clone::clone)?;
        rows.binary_search_by_key(&state, |row| row.id)
            .map(|at| rows[at].movement_infrastructure)
            .map_err(|_| format!("States has no row for state {state}"))
    }

    /// The division's standard template's recorded composition.
    fn template(&self, division: &DivisionRow) -> Result<&'a DivisionTemplateIconRow, String> {
        let rows = self.templates.as_ref().map_err(Clone::clone)?;
        rows.binary_search_by_key(&division.template, |row| row.template)
            .map(|at| &rows[at])
            .map_err(|_| {
                format!(
                    "DivisionTemplateIcons has no row for template {:#x}",
                    division.template
                )
            })
    }

    /// The template's special-forces share: over its composition's (definition, count)
    /// entries, the summed count of special-forces definitions over the summed count of
    /// all, fixed5, zero for an empty composition. A null-object entry (token zero)
    /// counts and is not special forces. A composition unreadable when recorded refuses
    /// by name ([`UNREADABLE_COMPOSITION`]).
    fn special_share(
        &self,
        division: &DivisionRow,
        sub_units: &impl SubUnitContent,
    ) -> Result<i64, String> {
        let template = self.template(division)?;
        if template.model_unreadable != 0 {
            return Err(UNREADABLE_COMPOSITION.into());
        }
        let tokens = self.tokens.as_ref().map_err(Clone::clone)?;
        let (mut total, mut special) = (0_i32, 0_i32);
        for unit in &template.subunits {
            total = total.wrapping_add(unit.count);
            if unit.token == 0 {
                continue;
            }
            let name = tokens
                .binary_search_by_key(&unit.token, |row| row.token)
                .map(|at| tokens[at].literal.as_slice())
                .map_err(|_| format!("LexicalTokens has no token {}", unit.token))?;
            if sub_units.special_forces(name)? {
                special = special.wrapping_add(unit.count);
            }
        }
        let denominator = i64::from(total).wrapping_mul(100_000);
        if denominator == 0 {
            return Ok(0);
        }
        i64::from(special)
            .wrapping_mul(100_000)
            .wrapping_mul(100_000)
            .checked_div(denominator)
            .ok_or_else(|| "the special-forces share overflows the native division".to_owned())
    }

    /// Whether the template's priority sub-unit (`DivisionTemplateIcons.priority_unit_name`)
    /// has the armour type bit ([`ARMOUR_TYPE`]).
    fn armoured(
        &self,
        division: &DivisionRow,
        sub_units: &impl SubUnitContent,
    ) -> Result<bool, String> {
        let template = self.template(division)?;
        Ok(sub_units.sub_unit_types(&template.priority_unit_name)? & ARMOUR_TYPE != 0)
    }

    /// The strategic redeployment speed of the move `from -> to` before the outer
    /// movement scale: the larger of the road speed over the destination state's railway
    /// infrastructure fraction (`States.railway_infrastructure`, zero without a state)
    /// and the rail speed over the departure province's directed railway percentage
    /// (`RailCooldowns` and `RailEdges`; a province without rail info has none).
    fn rail_speed(&self, from: u32, to: u32, rail: &RailDefines) -> Result<i64, String> {
        let state = self.province(to)?.state;
        let infrastructure = if state == 0 {
            0
        } else {
            let rows = self.states.as_ref().map_err(Clone::clone)?;
            let row = rows
                .binary_search_by_key(&state, |row| row.id)
                .map(|at| &rows[at])
                .map_err(|_| format!("States has no row for state {state}"))?;
            row.railway_infrastructure
                .require("States.railway_infrastructure")?
                .ok_or_else(|| {
                    format!("state {state}'s railway infrastructure was refused when recorded")
                })?
        };
        let cooldowns = self.rail_cooldowns.as_ref().map_err(Clone::clone)?;
        let percentage = match cooldowns.binary_search_by_key(&from, |row| row.province) {
            Err(_) => 0,
            Ok(at) => {
                let row = &cooldowns[at];
                let edge = if row.cooldown_ticks <= 0 {
                    let edges = self.rail_edges.as_ref().map_err(Clone::clone)?;
                    edges
                        .binary_search_by_key(&(from, to), |edge| (edge.from, edge.to))
                        .ok()
                        .map(|at| edges[at].level)
                } else {
                    None
                };
                let health = if edge.is_some() {
                    row.building
                        .require("RailCooldowns.building")?
                        .map(|state| fresh_inputs::RailwayBuildingState {
                            building: state.building.map(|health| {
                                fresh_inputs::RailwayBuildingHealth {
                                    level: health.level,
                                    healthy: health.healthy,
                                    partial: health.partial,
                                }
                            }),
                        })
                } else {
                    None
                };
                fresh_inputs::directed_percentage(
                    row.cooldown_ticks,
                    edge,
                    health,
                    rail.maximum_level,
                )
                .map_err(str::to_owned)?
            }
        };
        Ok(fresh_inputs::road_or_rail_speed(
            infrastructure,
            percentage,
            rail.speed,
        ))
    }

    /// The movement family's recorded layout check, once per frame.
    fn validate_layout(&self) -> Result<(), String> {
        self.layout
            .get_or_init(|| {
                crate::fixed_layout::validate_recording(self.frame, crate::fixed_layout::MOVEMENT)
            })
            .clone()
    }

    fn slot(&self, raw: u32) -> Result<u32, String> {
        let links = self.links.as_ref().map_err(Clone::clone)?;
        links
            .binary_search_by_key(&raw, |row| row.id)
            .map(|at| links[at].canonical_slot)
            .map_err(|_| format!("CountryLinks has no country {raw}"))
    }

    /// Whether the division's recorded route ends at a province whose controller its
    /// owner is at war with: the controller's cell in the owner's direct-war matrix,
    /// never the owner itself or a null controller.
    fn attacking(
        &self,
        division: &DivisionRow,
        movement: &DivisionMovementRow,
    ) -> Result<bool, String> {
        if movement.path.len() != movement.path_total as usize {
            return Err(
                "the division's recorded route is truncated, so its destination is unknown".into(),
            );
        }
        let Some(&destination) = movement.path.last() else {
            return Ok(false);
        };
        let controller = self.province(destination)?.controller;
        if controller as i32 <= 0 {
            return Ok(false);
        }
        let owner = self.slot(division.logical_owner)?;
        let target = self.slot(controller)?;
        if owner as i32 <= 0 || owner == target {
            return Ok(false);
        }
        let mut wars = self.wars.borrow_mut();
        let wars = wars
            .entry(owner)
            .or_insert_with(|| ReceiverWars::read(self.frame, owner));
        Ok(wars.as_ref().map_err(Clone::clone)?.pointer_active(target))
    }

    /// The division's organisation factor ([`super::organisation_factor`]) from its
    /// recorded organisation (`Divisions.organisation`) and maximum
    /// (`DivisionStatus.maximum_organisation`), both at their display steps.
    pub fn organisation(
        &self,
        division: &DivisionRow,
        movement: &DivisionMovementRow,
        defines: &Defines,
    ) -> Result<Option<i64>, String> {
        let maximum = self.status.as_ref().map_err(Clone::clone).and_then(|rows| {
            rows.binary_search_by_key(&division.id, |row| row.division)
                .map(|at| rows[at].maximum_organisation)
                .map_err(|_| format!("DivisionStatus has no row for division {:#x}", division.id))
        });
        super::organisation_factor(
            maximum.map(|maximum| (division.organisation, maximum)),
            || self.attacking(division, movement),
            defines,
        )
    }

    fn status_row(&self, division: u64) -> Result<&'a DivisionStatusRow, String> {
        let rows = self.status.as_ref().map_err(Clone::clone)?;
        rows.binary_search_by_key(&division, |row| row.division)
            .map(|at| &rows[at])
            .map_err(|_| format!("DivisionStatus has no row for division {division:#x}"))
    }

    /// The country's movement and attrition entries ([`crate::rows::CountryMovementModifiers`]):
    /// a raw country reference's canonical slot's CountryCounterInputs row.
    fn country_modifiers(
        &self,
        raw: u32,
    ) -> Result<&'a crate::rows::CountryMovementModifiers, String> {
        let slot = self.slot(raw)?;
        let ids = self
            .country_ids
            .get_or_init(|| {
                let countries = self.countries.as_ref().map_err(Clone::clone)?;
                let mut ids = BTreeMap::new();
                // The first row of a slot, as a scan in row order finds it.
                for row in countries.iter().rev() {
                    ids.insert(row.canonical_slot, row.id);
                }
                Ok(ids)
            })
            .as_ref()
            .map_err(Clone::clone)?;
        let id = *ids
            .get(&slot)
            .ok_or_else(|| format!("Countries lacks canonical slot {slot}"))?;
        let rows = self.counters.as_ref().map_err(Clone::clone)?;
        let row = rows
            .binary_search_by_key(&id, |row| row.country)
            .map(|at| &rows[at])
            .map_err(|_| format!("CountryCounterInputs has no row for country {id}"))?;
        row.movement_modifiers
            .require("CountryCounterInputs.movement_modifiers")?
            .as_ref()
            .ok_or_else(|| {
                "the country's movement modifier entries were refused when recorded".into()
            })
    }

    /// The direct commander's movement operands; `None` without a commander.
    fn commander(
        &self,
        group: u64,
    ) -> Result<Option<&'a crate::rows::CommanderMovementStats>, String> {
        if group == 0 {
            return Ok(None);
        }
        let rows = self.groups.as_ref().map_err(Clone::clone)?;
        let row = rows
            .binary_search_by_key(&group, |row| row.group)
            .map(|at| &rows[at])
            .map_err(|_| format!("OrdersGroupCounterInputs has no row for group {group:#x}"))?;
        row.commander_movement
            .require("OrdersGroupCounterInputs.commander_movement")?
            .as_ref()
            .map(Option::as_ref)
            .ok_or_else(|| "the commander's movement operands were refused when recorded".into())
    }

    /// The effective value of a raw entry under its recorded definition and the DLC
    /// feature mask, as the game's modifier lookup returns it.
    fn effective(&self, key: u32, raw: Option<i64>) -> Result<i64, String> {
        if raw.is_none() {
            return Ok(0);
        }
        let definitions = self.definitions.as_ref().map_err(Clone::clone)?;
        let definition = definitions
            .binary_search_by_key(&key, |row| row.index)
            .map(|at| &definitions[at])
            .map_err(|_| format!("ModifierDefinitions has no row {key:#x}"))?;
        let features = match self.frame.complete::<tables::SimulationFeatures>() {
            Ok([row]) => Some(row.dlc_mask),
            _ => None,
        };
        crate::naval_mission_derive::effective(definition, features, raw).map_err(str::to_owned)
    }

    fn recorded_defines(&self) -> Result<RecordedDefines, String> {
        let rows = complete::<tables::SimulationFeatures>(self.frame)?;
        let [row] = rows else {
            return Err("SimulationFeatures has no singleton row".into());
        };
        Ok(*row
            .movement_defines
            .require("SimulationFeatures.movement_defines")?)
    }

    fn weather_province(&self, id: u32) -> Result<Option<&'a WeatherProvinceRow>, String> {
        let rows = self.weather_provinces.as_ref().map_err(Clone::clone)?;
        Ok(rows
            .binary_search_by_key(&id, |row| row.id)
            .ok()
            .map(|at| &rows[at]))
    }

    /// Whether `province` has positive effective snow (`attack_derive::has_snow` over the
    /// recorded weather and the manager's snow ceiling).
    fn snow(&self, province: u32) -> Result<bool, String> {
        let Some(weather) = self.weather_province(province)? else {
            return Ok(false);
        };
        let ceiling = self
            .weather_regions
            .as_ref()
            .map_err(Clone::clone)?
            .first()
            .ok_or("WeatherRegions holds no snow ceiling")?
            .max_snow;
        Ok(crate::attack_derive::has_snow(
            weather.snow,
            weather.permanent_snow,
            ceiling,
        ))
    }

    /// The location's province and region weather speed and attack speed entries,
    /// effective; zero where the province or region has no weather record.
    fn weather(&self, location: u32, geography: &impl Geography) -> Result<[i64; 4], String> {
        use engine::modifier_keys::{ARMY_ATTACK_SPEED_FACTOR, ARMY_SPEED_FACTOR};
        let mut values = [0; 4];
        if let Some(row) = self.weather_province(location)? {
            let modifiers = row
                .modifiers
                .require("WeatherProvinces.modifiers")?
                .as_ref()
                .ok_or("the province's weather modifier entries were refused when recorded")?;
            values[0] = self.effective(ARMY_SPEED_FACTOR, modifiers.army_speed_factor)?;
            values[1] =
                self.effective(ARMY_ATTACK_SPEED_FACTOR, modifiers.army_attack_speed_factor)?;
        }
        if let Some(region) = geography.region(location) {
            let rows = self.weather_regions.as_ref().map_err(Clone::clone)?;
            if let Ok(at) = rows.binary_search_by_key(&region, |row| row.id) {
                let combat = rows[at]
                    .combat
                    .require("WeatherRegions.combat")?
                    .as_ref()
                    .ok_or("the region's weather modifier entries were refused when recorded")?;
                values[2] = self.effective(ARMY_SPEED_FACTOR, combat.army_speed_factor)?;
                values[3] =
                    self.effective(ARMY_ATTACK_SPEED_FACTOR, combat.army_attack_speed_factor)?;
            }
        }
        Ok(values)
    }

    /// The owner whose country terms a division reads: its expeditionary owner when
    /// positive, else its roster owner.
    fn owner(division: &DivisionRow) -> u32 {
        if division.expeditionary_owner as i32 > 0 {
            division.expeditionary_owner
        } else {
            division.roster_owner
        }
    }

    /// The division's selected commander unit block's raw entries
    /// ([`DivisionStatusRow::COMMANDER_KEYS`]), all absent without a commander.
    fn leader_block(&self, status: &DivisionStatusRow) -> Result<[Option<i64>; 5], String> {
        status
            .commander_modifiers
            .require("DivisionStatus.commander_modifiers")?
            .ok_or_else(|| {
                "the division's commander modifier entries were refused when recorded".to_owned()
            })
    }

    /// The enemy air term's operands over the division's location ([`AirOperands`]):
    /// `None` where the game reads none, for an owner in no war relation or a
    /// location outside any strategic region. The owner's war state is its recorded
    /// war relations, which every war installs in both endpoints' relations; its
    /// region pair is the owner's `RaidAirScores` row, an absent row a zero pair.
    fn air(
        &self,
        division: &DivisionRow,
        stats: &crate::rows::DivisionMovementStats,
        geography: &impl Geography,
        recorded: &RecordedDefines,
    ) -> Result<Option<AirOperands>, String> {
        let Some(region) = geography.region(division.province) else {
            return Ok(None);
        };
        let owner = self.slot(division.logical_owner)?;
        if owner as i32 <= 0 {
            return Err(format!(
                "the division's owner {} has no positive country slot",
                division.logical_owner
            ));
        }
        let at_war = {
            let mut wars = self.wars.borrow_mut();
            let wars = wars
                .entry(owner)
                .or_insert_with(|| ReceiverWars::read(self.frame, owner));
            !wars.as_ref().map_err(Clone::clone)?.targets().is_empty()
        };
        if !at_war {
            return Ok(None);
        }
        let index = self
            .air_index
            .get_or_init(|| {
                let rows = self.air_scores.as_ref().map_err(Clone::clone)?;
                let mut index = BTreeMap::new();
                for row in *rows {
                    let impact = *row.impact.require("RaidAirScores.impact")?;
                    index.insert(
                        (self.slot(row.country)?, row.region),
                        [row.friendly, row.enemy, impact],
                    );
                }
                Ok(index)
            })
            .as_ref()
            .map_err(Clone::clone)?;
        if index.range((owner, 0)..=(owner, u32::MAX)).next().is_none() {
            return Err("RaidAirScores holds no row for the division's owner".into());
        }
        let [friendly, enemy, impact] = index.get(&(owner, region)).copied().unwrap_or([0; 3]);
        let define = |value: Option<i64>, name: &str| {
            value.ok_or_else(|| format!("the {name} define was refused when recorded"))
        };
        let defense = if enemy > 0 {
            engine::csub_unit_definition::movement_air_defense_value(
                stats.air_defense,
                define(recorded.air_defense_denominator, "air defense denominator")?,
                define(recorded.air_defense_reduction, "air defense reduction")?,
            )
            .map_err(str::to_owned)?
        } else {
            0
        };
        Ok(Some(AirOperands {
            friendly,
            enemy,
            impact,
            scale: if enemy > 0 {
                define(recorded.air_speed, "air speed")?
            } else {
                0
            },
            defense,
        }))
    }

    /// The division's out-of-supply operands under the recorded defines. With days out
    /// of supply the received ratio and remaining grace are the exact recorded values
    /// ([`DivisionStatusRow::supply_ratio_exact`]); without, the severity is zero whatever
    /// they hold, so the displayed ones stand in, except under a supply-days define of
    /// zero, which refuses by name ([`SUPPLY_DAYS_ZERO`]).
    fn supply_operands(&self, status: &DivisionStatusRow) -> Result<SupplyOperands, String> {
        let recorded = self.recorded_defines()?;
        let define = |value: Option<i64>, name: &str| {
            value.ok_or_else(|| format!("the {name} define was refused when recorded"))
        };
        let days = status
            .out_of_supply_days
            .require("DivisionStatus.out_of_supply_days")?
            .ok_or("the division's days out of supply were refused when recorded")?;
        let supply_days = define(recorded.supply_days, "supply days")?;
        let [ratio, grace_remaining] = if days == 0 {
            // With a supply-days define of zero the severity divides zero by zero, which
            // the game's fraction answers with its sentinel: the penalty then reads the
            // ratio and grace, which are recorded exactly only with days out of supply.
            if supply_days == 0 {
                return Err(SUPPLY_DAYS_ZERO.into());
            }
            [
                status.supply_ratio,
                *status
                    .supply_grace_remaining
                    .require("DivisionStatus.supply_grace_remaining")?,
            ]
        } else {
            status
                .supply_ratio_exact
                .require("DivisionStatus.supply_ratio_exact")?;
            status
                .supply_exact()
                .ok_or("the division's exact supply was not recorded with its days out of supply")?
        };
        Ok(SupplyOperands {
            grace_maximum: *status
                .supply_grace_maximum
                .require("DivisionStatus.supply_grace_maximum")?,
            grace_remaining,
            ratio,
            days,
            supply_days: supply_days as i32,
            supply_grace: define(recorded.supply_grace, "supply grace")? as i32,
            penalty: define(recorded.out_of_supply_speed, "out-of-supply speed")?,
        })
    }

    /// The attrition hover's out-of-supply line value: the attrition severity
    /// ([`super::attrition_supply_severity`] against `threshold`) times the
    /// out-of-supply attrition define; `None` where the severity is zero and the game
    /// writes no line. A division in naval transfer has none.
    pub fn attrition_supply(
        &self,
        division: &DivisionRow,
        movement: &DivisionMovementRow,
        threshold: i64,
        sub_units: &impl SubUnitContent,
    ) -> Result<Option<i64>, String> {
        use engine::modifier_keys as keys;
        if movement.naval_transfer != 0 {
            return Ok(None);
        }
        self.validate_layout()?;
        let status = self.status_row(division.id)?;
        let operands = self.supply_operands(status)?;
        let country = self.country_modifiers(Self::owner(division))?;
        let leader = self.leader_block(status)?;
        let leader_value =
            |index: usize, key: u32| -> Result<i64, String> { self.effective(key, leader[index]) };
        let share = || self.special_share(division, sub_units);
        let factors = SupplyFactors {
            country: self.effective(keys::OUT_OF_SUPPLY_FACTOR, country.out_of_supply_factor)?,
            leader: leader_value(2, keys::OUT_OF_SUPPLY_FACTOR)?,
            special_forces: [
                self.effective(
                    keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR,
                    country.special_forces_out_of_supply_factor,
                )?,
                leader_value(4, keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR)?,
            ],
            share: &share,
        };
        let severity = super::attrition_supply_severity(&operands, threshold, &factors)?;
        if severity == 0 {
            return Ok(None);
        }
        let factor = self
            .recorded_defines()?
            .out_of_supply_attrition
            .ok_or("the out-of-supply attrition define was refused when recorded")?;
        Ok(Some(severity.wrapping_mul(factor) / 100_000))
    }

    /// Runs `estimate` with the division's speed terms ([`SpeedTerms`]): the common
    /// modifier of its country's and selected commander block's terms and armour speed,
    /// its location's weather, its out-of-supply term (at the template's special-forces
    /// share) and retreat terms; each edge's terrain, fort, snow, commander terrain,
    /// river and infrastructure terms; and its organisation and fuel factors. A strategic
    /// redeployment that no modifier blocks moves at the road or rail speed
    /// ([`EdgeTerm::Scaled`]) and land edges past a landing at the invasion speed
    /// ([`EdgeTerm::Fixed`]). A division in combat that is not withdrawing refuses by
    /// name: the combat's movement factor replaces the modifier's one and is not
    /// recorded. A missing operand of a term the game applies refuses by name.
    pub fn with_terms<T>(
        &self,
        division: &DivisionRow,
        movement: &DivisionMovementRow,
        geography: &impl Geography,
        defines: &Defines,
        estimate: impl FnOnce(&SpeedTerms<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        use engine::modifier_keys as keys;
        // Without a base speed no term matters.
        super::base_speed(movement)?;
        let withdrawing = *movement
            .withdrawal
            .require("DivisionMovement.withdrawal")?
            .as_ref()
            .ok_or("the division's withdrawal byte was refused when recorded")?
            != 0;
        if movement.combat_count != 0 && !withdrawing {
            return Err(IN_COMBAT.into());
        }
        // A naval transfer moves at the transport speed.
        if movement.naval_transfer != 0 {
            return Err(SEA_MOVE.into());
        }
        // A move to or from the sea moves at the transport speed; past a landing (the
        // location at sea, the first route province on land) the land edges move at the
        // invasion speed. Otherwise None: the edge's own speed applies.
        let invasion = !geography
            .land(division.province)
            .ok_or_else(|| format!("province {} has no content terrain", division.province))?
            && movement
                .path
                .first()
                .and_then(|&first| geography.land(first))
                .unwrap_or(false);
        let special_edge = |from: u32, to: u32| -> Result<Option<EdgeTerm>, String> {
            for province in [from, to] {
                if !geography
                    .land(province)
                    .ok_or_else(|| format!("province {province} has no content terrain"))?
                {
                    return Err(SEA_MOVE.into());
                }
            }
            Ok(invasion.then_some(EdgeTerm::Fixed(defines.invasion_speed)))
        };
        // A strategic redeployment that no disable-redeployment modifier blocks moves at
        // the road or rail speed; a blocked one at the ordinary speed.
        if movement.strategic_redeployment != 0 {
            match movement
                .strategic_action
                .require("DivisionMovement.strategic_action")?
            {
                StrategicAction::Available(inputs) if !inputs.blocked() => {
                    let rail = defines.rail.ok_or(
                        "the content's strategic redeployment speed defines failed to load",
                    )?;
                    let edge = |from: u32, to: u32| -> Result<EdgeTerm, String> {
                        if let Some(term) = special_edge(from, to)? {
                            return Ok(term);
                        }
                        Ok(EdgeTerm::Scaled(self.rail_speed(from, to, &rail)?))
                    };
                    return estimate(&SpeedTerms {
                        modifier: 0,
                        edge: &edge,
                        organisation: None,
                        fuel: None,
                    });
                }
                StrategicAction::Available(_) => {}
                StrategicAction::Withheld(failure) => {
                    return Err(format!(
                        "the division is strategically redeploying, and its {}",
                        failure.reason().trim_start_matches("strategic action ")
                    ));
                }
                StrategicAction::Unreached => return Err(STRATEGIC_REDEPLOYMENT.into()),
            }
        }
        self.validate_layout()?;
        let status = self.status_row(division.id)?;
        let recorded = self.recorded_defines()?;
        let define = |value: Option<i64>, name: &str| {
            value.ok_or_else(|| format!("the {name} define was refused when recorded"))
        };
        let stats = status
            .movement_stats
            .require("DivisionStatus.movement_stats")?
            .as_ref()
            .ok_or("the division's stat-block movement operands were refused when recorded")?;
        let country = self.country_modifiers(Self::owner(division))?;
        let commander = self.commander(division.orders_group)?;
        let leader = self.leader_block(status)?;
        // Commander block entries: army speed, armour speed, out of supply, river
        // crossing and special-forces out of supply.
        let leader_value =
            |index: usize, key: u32| -> Result<i64, String> { self.effective(key, leader[index]) };
        let attacking = std::cell::OnceCell::new();
        let attacking = || -> Result<bool, String> {
            attacking
                .get_or_init(|| self.attacking(division, movement))
                .clone()
        };
        let mut modifiers = CommonModifiers {
            country_speed: self.effective(keys::ARMY_SPEED_FACTOR, country.army_speed_factor)?,
            leader_speed: leader_value(0, keys::ARMY_SPEED_FACTOR)?,
            country_attack_speed: self.effective(
                keys::ARMY_ATTACK_SPEED_FACTOR,
                country.army_attack_speed_factor,
            )?,
            weather: self.weather(division.province, geography)?,
            armour_speed: [0; 2],
        };
        // A division with a commander whose priority sub-unit is armour takes the
        // country's and the commander block's armour speed.
        let armour_speed = [
            self.effective(
                keys::ARMY_ARMOR_SPEED_FACTOR,
                country.army_armor_speed_factor,
            )?,
            leader_value(1, keys::ARMY_ARMOR_SPEED_FACTOR)?,
        ];
        if armour_speed != [0, 0] && commander.is_some() && self.armoured(division, geography)? {
            modifiers.armour_speed = armour_speed;
        }
        // A hostile move is decided only where an attack speed term is nonzero.
        let hostile = if modifiers.country_attack_speed != 0
            || modifiers.weather[1] != 0
            || modifiers.weather[3] != 0
        {
            attacking()?
        } else {
            false
        };
        let acclimatization = if modifiers.weather == [0; 4] {
            0
        } else {
            status
                .weather_factor
                .require("DivisionStatus.weather_factor")?
                .ok_or("the division's acclimatization factor was refused when recorded")?
        };
        if !hostile {
            modifiers.country_attack_speed = 0;
        }
        let exiled = status.move_flags & DivisionStatusRow::MOVE_EXILED != 0;
        let supply = if exiled || movement.naval_transfer != 0 {
            0
        } else {
            let operands = self.supply_operands(status)?;
            let share = || self.special_share(division, geography);
            let factors = SupplyFactors {
                country: self
                    .effective(keys::OUT_OF_SUPPLY_FACTOR, country.out_of_supply_factor)?,
                leader: leader_value(2, keys::OUT_OF_SUPPLY_FACTOR)?,
                special_forces: [
                    self.effective(
                        keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR,
                        country.special_forces_out_of_supply_factor,
                    )?,
                    leader_value(4, keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR)?,
                ],
                share: &share,
            };
            super::supply_term(false, &operands, &factors)?
        };
        let retreating = movement.retreat != 0;
        let retreat = if retreating || withdrawing {
            super::retreat_term(
                retreating,
                withdrawing,
                define(recorded.retreat_speed, "retreat speed")?,
                self.effective(
                    keys::ARMY_RETREAT_SPEED_FACTOR,
                    country.army_retreat_speed_factor,
                )?,
                define(recorded.withdrawal_speed, "withdrawal speed")?,
            )
        } else {
            0
        };
        let air = super::air_term(self.air(division, stats, geography, &recorded)?.as_ref())?;
        let modifier =
            super::common_modifier(&modifiers, hostile, acclimatization, supply, air, retreat);
        // A stat block without fuel capacity takes no fuel factor whatever its fuel.
        let capacity_scale = define(recorded.fuel_capacity_scale, "fuel capacity scale")?;
        let fuel = if stats.fuel_rate.wrapping_mul(capacity_scale) / 100_000 <= 0 {
            None
        } else {
            let fuel = movement
                .fuel
                .require("DivisionMovement.fuel")?
                .ok_or("the division's fuel was refused when recorded")?;
            super::fuel_factor(
                fuel,
                stats.fuel_rate,
                capacity_scale,
                define(recorded.out_of_fuel_speed, "out-of-fuel speed")?,
            )?
        };
        let maximum = self
            .status_row(division.id)
            .map(|row| row.maximum_organisation);
        let organisation = super::organisation_factor(
            maximum.map(|maximum| (division.organisation, maximum)),
            attacking,
            defines,
        )?;
        let river_leader = leader_value(3, keys::RIVER_CROSSING_FACTOR)?;
        let edge = |from: u32, to: u32| -> Result<EdgeTerm, String> {
            if let Some(term) = special_edge(from, to)? {
                return Ok(term);
            }
            let index = geography
                .terrain_index(to)
                .ok_or_else(|| format!("province {to} has no content terrain"))?;
            let entry = usize::try_from(index)
                .ok()
                .and_then(|index| index.checked_sub(1));
            let unit = entry.and_then(|at| stats.terrain.get(at)).ok_or_else(|| {
                format!(
                    "terrain {index} is outside the stat block's {} terrain entries",
                    stats.terrain.len()
                )
            })?;
            let own = geography.terrain_movement(index).ok_or_else(|| {
                format!("terrain {index} is outside the content's terrain categories")
            })?;
            let scalar = |value: Option<i64>| {
                value.ok_or_else(|| {
                    "the stat block's fort, river and snow movement were refused when recorded"
                        .to_owned()
                })
            };
            let leader_terrain = match commander {
                None => 0,
                // Out of range, the commander's lookup takes the first entry.
                Some(commander) => usize::try_from(index)
                    .ok()
                    .and_then(|index| commander.terrain.get(index))
                    .or_else(|| commander.terrain.first())
                    .copied()
                    .ok_or("the commander's terrain movement vector has no storage")?,
            };
            let crossing = geography
                .crossing_kind(from, to)
                .ok_or_else(|| format!("route edge {from}->{to} has no content crossing"))?;
            let river = match crossing {
                1..=3 => Some((
                    river_leader,
                    scalar(stats.river())?,
                    match crossing {
                        2 => defines.river_crossing_two,
                        3 => defines.river_crossing_three,
                        _ => 0,
                    },
                )),
                _ => None,
            };
            Ok(EdgeTerm::Normal(super::edge_operands(&EdgeTerms {
                terrain: unit.wrapping_add(own),
                fort: if self.province(to)?.fort_positive != 0 {
                    Some(scalar(stats.fort())?)
                } else {
                    None
                },
                snow: if self.snow(to)? {
                    Some(scalar(stats.snow())?)
                } else {
                    None
                },
                leader_terrain,
                infrastructure: self.infrastructure(to)?,
                river,
            })))
        };
        estimate(&SpeedTerms {
            modifier,
            edge: &edge,
            organisation,
            fuel,
        })
    }
}
