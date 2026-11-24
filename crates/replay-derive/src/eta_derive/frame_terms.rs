
use super::{
    AirOperands, CommonModifiers, Defines, EdgeTerm, EdgeTerms, RailDefines, SpeedTerms,
    SupplyFactors, SupplyOperands,
};
use crate::{
    directed_war_derive::ReceiverWars,
    frame::DecodedFrame,
    rows::{
        CoreSupplyAccessRow, CountryCounterInputRow, CountryLinkRow, CountryRow,
        CountryUnitSpeedEntryRow, DivisionMovementRow, DivisionPanelStatsRow, DivisionRow,
        DivisionStatusRow,
        DivisionTemplateIconRow, LexicalTokenRow, ModifierDefinitionRow,
        MovementDefines as RecordedDefines, MovementStateTermRow, OrdersGroupCounterInputRow,
        OwnerTransportSpeedRow, ProvinceRow, RaidAirScoreRow, RailCooldownRow, RailEdgeRow,
        RiverRelationRow, StateMovementTerms, StateRow, StrategicAction, TableId, TableSchema,
        WeatherProvinceRow, WeatherRegionRow, tables,
    },
};
use engine_railways::crailway_manager::fresh_inputs;
use std::{cell::RefCell, collections::BTreeMap};

pub trait SubUnitContent {
    fn special_forces(&self, name: &[u8]) -> Result<bool, String>;
    fn sub_unit_types(&self, name: &[u8]) -> Result<u64, String>;
}

pub const ARMOUR_TYPE: u64 = 4;

pub trait Geography: SubUnitContent {
    fn terrain_index(&self, province: u32) -> Option<i32>;
    fn terrain_flags(&self, province: u32) -> Option<u8>;
    fn land(&self, province: u32) -> Option<bool> {
        self.terrain_flags(province)
            .map(|flags| flags & engine_native::cprovince::definition::LAND_FLAG != 0)
    }
    fn terrain_movement(&self, index: i32) -> Option<i64>;
    fn crossing_kind(&self, from: u32, to: u32) -> Option<u8>;
    fn region(&self, province: u32) -> Option<u32>;
}

pub const TABLES: [TableId; 24] = [
    TableId::Provinces,
    TableId::States,
    TableId::DivisionStatus,
    TableId::DivisionPanelStats,
    TableId::CountryLinks,
    TableId::Countries,
    TableId::CountryCounterInputs,
    TableId::OrdersGroupCounterInputs,
    TableId::WeatherProvinces,
    TableId::WeatherRegions,
    TableId::ModifierDefinitions,
    TableId::LexicalTokens,
    TableId::SimulationFeatures,
    TableId::WarRelations,
    TableId::Clock,
    TableId::RaidAirScores,
    TableId::RailCooldowns,
    TableId::RailEdges,
    TableId::DivisionTemplateIcons,
    TableId::MovementStateTerms,
    TableId::CoreSupplyAccess,
    TableId::CountryUnitSpeedEntries,
    TableId::RiverRelations,
    TableId::DivisionTransportSpeeds,
];

pub struct FrameTerms<'a> {
    frame: &'a DecodedFrame,
    provinces: Result<&'a [ProvinceRow], String>,
    states: Result<&'a [StateRow], String>,
    status: Result<&'a [DivisionStatusRow], String>,
    panels: Result<&'a [DivisionPanelStatsRow], String>,
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
    state_terms: Result<&'a [MovementStateTermRow], String>,
    core_access: Result<&'a [CoreSupplyAccessRow], String>,
    unit_entries: Result<&'a [CountryUnitSpeedEntryRow], String>,
    river_relations: Result<&'a [RiverRelationRow], String>,
    transport_speeds: Result<&'a [OwnerTransportSpeedRow], String>,
    air_index: std::cell::OnceCell<Result<AirIndex, String>>,
    country_ids: std::cell::OnceCell<Result<BTreeMap<u32, u32>, String>>,
    layout: std::cell::OnceCell<Result<(), String>>,
}

type AirIndex = BTreeMap<(u32, u32), [i64; 3]>;

pub const TRANSPORT_NOT_RECORDED: &str = "the division moves by sea at its owner's transport speed, which DivisionTransportSpeeds does not hold for the edge's strategic region";
pub const SUPPLY_DAYS_ZERO: &str = "the supply-days define is zero, so the out-of-supply penalty reads the received supply at zero days out of supply, which is recorded only at its display step";
pub const UNREADABLE_COMPOSITION: &str = "the template's composition was unreadable when recorded, so its special-forces share is unknown";

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
            panels: complete::<tables::DivisionPanelStats>(frame),
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
            state_terms: complete::<tables::MovementStateTerms>(frame),
            core_access: complete::<tables::CoreSupplyAccess>(frame),
            unit_entries: complete::<tables::CountryUnitSpeedEntries>(frame),
            river_relations: complete::<tables::RiverRelations>(frame),
            transport_speeds: complete::<tables::DivisionTransportSpeeds>(frame),
            air_index: std::cell::OnceCell::new(),
            country_ids: std::cell::OnceCell::new(),
            layout: std::cell::OnceCell::new(),
        }
    }

    fn base_speed(&self, movement: &DivisionMovementRow) -> Option<i64> {
        let panel = self.panels.as_ref().ok().and_then(|rows| {
            rows.binary_search_by_key(&movement.division, |row| row.division)
                .ok()
                .map(|at| &rows[at])
        });
        movement.base_speed(panel)
    }

    fn province(&self, id: u32) -> Result<&'a ProvinceRow, String> {
        let rows = self.provinces.as_ref().map_err(Clone::clone)?;
        rows.binary_search_by_key(&id, |row| row.id)
            .map(|at| &rows[at])
            .map_err(|_| format!("Provinces has no row for province {id}"))
    }

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

    fn armoured(
        &self,
        division: &DivisionRow,
        sub_units: &impl SubUnitContent,
    ) -> Result<bool, String> {
        let template = self.template(division)?;
        Ok(sub_units.sub_unit_types(&template.priority_unit_name)? & ARMOUR_TYPE != 0)
    }

    fn transport_speed(&self, owner: u32, region: u32) -> Result<i64, String> {
        let rows = self.transport_speeds.as_ref().map_err(Clone::clone)?;
        let at = rows
            .binary_search_by_key(&(owner, region), |row| (row.owner, row.region))
            .map_err(|_| TRANSPORT_NOT_RECORDED.to_owned())?;
        rows[at]
            .speed
            .ok_or_else(|| "the owner's transport speed was refused when recorded".to_owned())
    }

    fn state_terms(
        &self,
        state: u32,
        division: &DivisionRow,
    ) -> Result<&'a StateMovementTerms, String> {
        let rows = self.state_terms.as_ref().map_err(Clone::clone)?;
        let key = (state, division.roster_owner, Self::owner(division));
        let at = rows
            .binary_search_by_key(&key, |row| (row.state, row.roster, row.owner))
            .map_err(|_| {
                "MovementStateTerms has no row for the division's location state, roster owner and owner"
                    .to_owned()
            })?;
        rows[at].terms.as_ref().ok_or_else(|| {
            "the location state's movement speed entries were refused when recorded".to_owned()
        })
    }

    fn state_speed(&self, division: &DivisionRow, hostile: bool) -> Result<[i64; 2], String> {
        use engine_executable::modifier_keys as keys;
        let state = self.province(division.province)?.state;
        if state == 0 {
            return Ok([0; 2]);
        }
        let terms = self.state_terms(state, division)?;
        let branch = match terms.branch & StateMovementTerms::RELATION_MASK {
            StateMovementTerms::NEITHER => 0,
            StateMovementTerms::FRIENDLY => {
                self.effective(keys::ARMY_SPEED_FACTOR_FOR_CONTROLLER, terms.relation_speed)?
            }
            StateMovementTerms::ENEMY => {
                self.effective(keys::ENEMY_ARMY_SPEED_FACTOR, terms.relation_speed)?
            }
            other => {
                return Err(format!(
                    "MovementStateTerms holds an unknown controller branch {other}"
                ));
            }
        };
        Ok([
            self.effective(keys::ARMY_SPEED_FACTOR, terms.army_speed)?
                .wrapping_add(branch),
            if hostile {
                if terms.branch & StateMovementTerms::ATTACK_REFUSED != 0 {
                    return Err("the location state's hostile-destination attack speed was refused when recorded".into());
                }
                self.effective(keys::ARMY_ATTACK_SPEED_FACTOR, terms.attack_speed)?
            } else {
                0
            },
        ])
    }

    fn unit_speed(&self, division: &DivisionRow) -> Result<i64, String> {
        let keys = self
            .template(division)?
            .speed_keys
            .require("DivisionTemplateIcons.speed_keys")?
            .as_ref()
            .ok_or(
                "the template's movement priority sub-unit speed keys were refused when recorded",
            )?;
        let owner = Self::owner(division);
        let mut sum = 0_i64;
        for &key in keys {
            let rows = self.unit_entries.as_ref().map_err(Clone::clone)?;
            let at = rows
                .binary_search_by_key(&(owner, key), |row| (row.country, row.key))
                .map_err(|_| {
                    "CountryUnitSpeedEntries has no row for the owner's sub-unit speed key"
                        .to_owned()
                })?;
            let entry = rows[at]
                .entry
                .ok_or("the owner's sub-unit speed entry was refused when recorded")?;
            sum = sum.wrapping_add(self.effective(key, entry)?);
        }
        Ok(sum)
    }

    fn core_access(&self, division: &DivisionRow) -> Result<bool, String> {
        let rows = self.core_access.as_ref().map_err(Clone::clone)?;
        let key = (Self::owner(division), division.province);
        let at = rows
            .binary_search_by_key(&key, |row| (row.owner, row.province))
            .map_err(|_| {
                "CoreSupplyAccess has no row for the division's owner and location".to_owned()
            })?;
        rows[at]
            .access
            .ok_or_else(|| "the owner's core supply access was refused when recorded".to_owned())
    }

    fn noncore(
        &self,
        division: &DivisionRow,
        status: &DivisionStatusRow,
        defines: &Defines,
    ) -> Result<i64, String> {
        if status.move_flags & DivisionStatusRow::MOVE_NONCORE_SUPPLY_BYPASS != 0
            || self.core_access(division)?
        {
            return Ok(0);
        }
        if status.move_flags & DivisionStatusRow::MOVE_NONCORE_AIR_REFUSED != 0 {
            return Err(
                "the division's noncore movement air-supply sign was refused when recorded".into(),
            );
        }
        let air = status.move_flags & DivisionStatusRow::MOVE_NONCORE_AIR != 0;
        defines.noncore_supply[usize::from(air)].ok_or_else(|| {
            if air {
                "the content's air noncore supply speed define failed to load"
            } else {
                "the content's ground noncore supply speed define failed to load"
            }
            .to_owned()
        })
    }

    fn river_relation(&self, owner: u32, controller: u32) -> Result<Option<i64>, String> {
        let rows = self.river_relations.as_ref().map_err(Clone::clone)?;
        let at = rows
            .binary_search_by_key(&(owner, controller), |row| (row.owner, row.controller))
            .map_err(|_| {
                "RiverRelations has no row for the owner and the crossing's destination controller"
                    .to_owned()
            })?;
        rows[at].entry.ok_or_else(|| {
            "the owner's relation river crossing factor was refused when recorded".to_owned()
        })
    }

    fn combat_base(&self, status: &DivisionStatusRow, defines: &Defines) -> Result<i64, String> {
        let entry = status
            .combat_movement
            .require("DivisionStatus.combat_movement")?
            .ok_or("the division's combat movement operand was refused when recorded")?;
        if entry.is_none() {
            return Ok(0);
        }
        let define = defines
            .combat_movement
            .ok_or("the content's combat movement speed define failed to load")?;
        Ok(super::combat_base(entry, define))
    }

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

    pub fn organisation(
        &self,
        division: &DivisionRow,
        movement: &DivisionMovementRow,
        defines: &Defines,
    ) -> Result<Option<i64>, String> {
        let row = self.status_row(division.id)?;
        let current = row
            .organisation_exact
            .require("DivisionStatus.organisation_exact")?;
        let maximum = row
            .maximum_organisation_exact
            .require("DivisionStatus.maximum_organisation_exact")?;
        if row.move_flags & DivisionStatusRow::MOVE_ORGANISATION_REFUSED != 0 {
            return Err(
                "the division's exact movement organisation operands were refused when recorded"
                    .into(),
            );
        }
        if row.move_flags & DivisionStatusRow::MOVE_LOW_ORGANISATION == 0 {
            return if current.is_none() && maximum.is_none() {
                Ok(None)
            } else {
                Err(
                    "exact organisation operands were recorded outside their movement branch"
                        .into(),
                )
            };
        }
        let operands = current.zip(*maximum).ok_or("the division's low-organisation movement factor needs its missing exact current and maximum organisation")?;
        super::organisation_factor(Ok(operands), || self.attacking(division, movement), defines)
    }

    fn status_row(&self, division: u64) -> Result<&'a DivisionStatusRow, String> {
        let rows = self.status.as_ref().map_err(Clone::clone)?;
        rows.binary_search_by_key(&division, |row| row.division)
            .map(|at| &rows[at])
            .map_err(|_| format!("DivisionStatus has no row for division {division:#x}"))
    }

    fn country_counter(&self, raw: u32) -> Result<&'a crate::rows::CountryCounterInputRow, String> {
        let slot = self.slot(raw)?;
        let ids = self
            .country_ids
            .get_or_init(|| {
                let countries = self.countries.as_ref().map_err(Clone::clone)?;
                let mut ids = BTreeMap::new();
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
        Ok(row)
    }

    fn country_modifiers(
        &self,
        raw: u32,
    ) -> Result<&'a crate::rows::CountryMovementModifiers, String> {
        self.country_counter(raw)?
            .movement_modifiers
            .require("CountryCounterInputs.movement_modifiers")?
            .as_ref()
            .ok_or_else(|| {
                "the country's movement modifier entries were refused when recorded".into()
            })
    }

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

    fn weather(&self, location: u32, geography: &impl Geography) -> Result<[i64; 4], String> {
        use engine_executable::{
            modifier_keys::ARMY_ATTACK_SPEED_FACTOR, modifier_keys::ARMY_SPEED_FACTOR,
        };
        let mut values = [0; 4];
        if let Some(row) = self.weather_province(location)? {
            let modifiers = row
                .modifiers
                .require("WeatherProvinces.modifiers")?
                .as_ref()
                .ok_or("the province's weather modifier entries were refused when recorded")?;
            values[0] = self.effective(ARMY_SPEED_FACTOR, modifiers.get(ARMY_SPEED_FACTOR))?;
            values[1] = self.effective(
                ARMY_ATTACK_SPEED_FACTOR,
                modifiers.get(ARMY_ATTACK_SPEED_FACTOR),
            )?;
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

    fn owner(division: &DivisionRow) -> u32 {
        if division.expeditionary_owner as i32 > 0 {
            division.expeditionary_owner
        } else {
            division.roster_owner
        }
    }

    fn leader_block(&self, status: &DivisionStatusRow) -> Result<[Option<i64>; 6], String> {
        status
            .commander_modifiers
            .require("DivisionStatus.commander_modifiers")?
            .map(|entries| {
                [
                    entries[0], entries[1], entries[2], None, entries[3], entries[4],
                ]
            })
            .ok_or_else(|| {
                "the division's commander modifier entries were refused when recorded".to_owned()
            })
    }

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
            engine_native_parts::csub_unit_definition::movement_air_defense_value(
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

    fn supply_operands(
        &self,
        status: &DivisionStatusRow,
        attrition_threshold: Option<i64>,
    ) -> Result<SupplyOperands, String> {
        let recorded = self.recorded_defines()?;
        let define = |value: Option<i64>, name: &str| {
            value.ok_or_else(|| format!("the {name} define was refused when recorded"))
        };
        let days = status
            .out_of_supply_days
            .require("DivisionStatus.out_of_supply_days")?
            .ok_or("the division's days out of supply were refused when recorded")?;
        let supply_days = define(recorded.supply_days, "supply days")?;
        let grace_maximum = *status
            .supply_grace_maximum
            .require("DivisionStatus.supply_grace_maximum")?;
        let supply_grace = define(recorded.supply_grace, "supply grace")? as i32;
        let penalty = define(recorded.out_of_supply_speed, "out-of-supply speed")?;
        let [ratio, grace_remaining] = if let Some(exact) = status.supply_exact() {
            exact
        } else if days != 0 {
            status
                .supply_ratio_exact
                .require("DivisionStatus.supply_ratio_exact")?;
            return Err(
                "the division's exact supply was not recorded with its days out of supply".into(),
            );
        } else if supply_days != 0 {
            [
                status
                    .supply_ratio
                    .ok_or("the division's supply ratio is not recorded")?,
                *status
                    .supply_grace_remaining
                    .require("DivisionStatus.supply_grace_remaining")?,
            ]
        } else if grace_maximum < 1 || attrition_threshold == Some(0) {
            [0, 0]
        } else {
            let ratio_bounds = status
                .recorded_supply_ratio_bounds()
                .ok_or("the division's supply ratio is not recorded")?;
            let grace_bounds = status
                .recorded_supply_grace_bounds(supply_grace)
                .ok_or("DivisionStatus.supply_grace_remaining is not recorded")?;
            let [eff_low, eff_high] = DivisionStatusRow::effective_supply_bounds(
                ratio_bounds,
                grace_maximum,
                grace_bounds,
                supply_grace,
            );
            let proven = eff_low == eff_high
                || attrition_threshold
                    .is_some_and(|threshold| threshold > 0 && eff_low >= threshold);
            if !proven {
                return Err(SUPPLY_DAYS_ZERO.into());
            }
            [ratio_bounds[0], grace_bounds[0]]
        };
        Ok(SupplyOperands {
            grace_maximum,
            grace_remaining,
            ratio,
            days,
            supply_days: supply_days as i32,
            supply_grace,
            penalty,
        })
    }

    pub fn attrition_supply(
        &self,
        division: &DivisionRow,
        movement: &DivisionMovementRow,
        threshold: i64,
        sub_units: &impl SubUnitContent,
    ) -> Result<Option<i64>, String> {
        use engine_executable::modifier_keys as keys;
        if movement.naval_transfer != 0 {
            return Ok(None);
        }
        self.validate_layout()?;
        let status = self.status_row(division.id)?;
        let operands = self.supply_operands(status, Some(threshold))?;
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
                leader_value(5, keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR)?,
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

    pub fn panel_width(&self, division: &DivisionRow) -> Result<i64, String> {
        use engine_executable::modifier_keys::COMBAT_WIDTH_FACTOR;
        if division.template == 0 {
            return Ok(0);
        }
        let base = self
            .template(division)?
            .base_width
            .require("DivisionTemplateIcons.base_width")?
            .ok_or("deployed template width base unavailable")?;
        self.country_modifiers(Self::owner(division))?;
        let status = self.status_row(division.id)?;
        self.leader_block(status)?;
        let leader_width = *status
            .commander_width_factor
            .require("DivisionStatus.commander_width_factor")?;
        let country_width = *self
            .country_counter(Self::owner(division))?
            .combat_width_factor
            .require("CountryCounterInputs.combat_width_factor")?;
        let factor = 100_000_i64
            .wrapping_add(self.effective(COMBAT_WIDTH_FACTOR, country_width)?)
            .wrapping_add(self.effective(COMBAT_WIDTH_FACTOR, leader_width)?);
        Ok(base.wrapping_mul(factor) / 100_000)
    }

    fn panel_stat(&self, division: u64, index: usize) -> Option<i64> {
        let rows = self.frame.complete::<tables::DivisionPanelStats>().ok()?;
        rows.binary_search_by_key(&division, |row| row.division)
            .ok()
            .and_then(|at| rows[at].panel_stats[index])
    }

    pub fn panel_speed(
        &self,
        division: &DivisionRow,
        movement: &DivisionMovementRow,
        geography: &impl Geography,
        defines: &Defines,
        combat: Result<i64, String>,
    ) -> Result<i64, String> {
        if movement.naval_transfer != 0 || geography.land(division.province) == Some(false) {
            let speed = movement
                .transport_speed
                .require("DivisionMovement.transport_speed")?
                .ok_or("deployed regional transport speed unavailable")?;
            return super::movement_fraction(
                speed.wrapping_mul(defines.transport_scale) / 100_000,
                defines.outer_scale,
            );
        }
        let status = self.status_row(division.id)?;
        let base = self
            .panel_stat(division.id, 21)
            .ok_or("deployed maximum speed cache unavailable")?;
        let to = if movement.path_total as i32 > 0 {
            *movement
                .path
                .first()
                .ok_or("deployed speed first route province unavailable")?
        } else {
            0
        };
        if to != 0 || movement.naval_transfer != 0 {
            let combat = if movement.combat_count == 0 {
                100_000
            } else {
                combat?
            };
            return self.with_speed_terms(
                division,
                movement,
                geography,
                defines,
                Some(combat),
                |terms| {
                    super::movement_fraction(
                        super::edge_speed(base, division.province, to, terms, defines)?,
                        defines.outer_scale,
                    )
                },
            );
        }
        let maximum = self
            .panel_stat(division.id, 1)
            .ok_or("deployed maximum organisation cache unavailable")?
            .max(100);
        let org = super::organisation_factor(
            Ok((division.organisation, maximum)),
            || self.attacking(division, movement),
            defines,
        )?;
        let recorded = self.recorded_defines()?;
        let stats = status
            .movement_stats
            .require("DivisionStatus.movement_stats")?
            .as_ref()
            .ok_or("deployed fuel capacity cache unavailable")?;
        let capacity = recorded
            .fuel_capacity_scale
            .ok_or("fuel capacity scale unavailable")?;
        let fuel = if stats.fuel_rate.wrapping_mul(capacity) / 100_000 <= 0 {
            None
        } else {
            super::fuel_factor(
                movement
                    .fuel
                    .require("DivisionMovement.fuel")?
                    .ok_or("deployed current fuel unavailable")?,
                stats.fuel_rate,
                capacity,
                recorded
                    .out_of_fuel_speed
                    .ok_or("out of fuel speed unavailable")?,
            )?
        };
        let mut speed = base;
        for factor in [org, fuel].into_iter().flatten() {
            speed = speed.wrapping_mul(factor) / 100_000;
        }
        super::movement_fraction(
            speed.max(100_000).wrapping_mul(defines.outer_scale) / 100_000,
            defines.outer_scale,
        )
    }

    pub fn with_terms<T>(
        &self,
        division: &DivisionRow,
        movement: &DivisionMovementRow,
        geography: &impl Geography,
        defines: &Defines,
        estimate: impl FnOnce(&SpeedTerms<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_route_terms(
            division,
            movement,
            &movement.path,
            geography,
            defines,
            estimate,
        )
    }

    fn with_speed_terms<T>(
        &self,
        division: &DivisionRow,
        movement: &DivisionMovementRow,
        geography: &impl Geography,
        defines: &Defines,
        panel_combat: Option<i64>,
        estimate: impl FnOnce(&SpeedTerms<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_route_terms_inner(
            division,
            movement,
            &movement.path,
            geography,
            defines,
            panel_combat,
            estimate,
        )
    }

    pub fn with_route_terms<T>(
        &self,
        division: &DivisionRow,
        movement: &DivisionMovementRow,
        route: &[u32],
        geography: &impl Geography,
        defines: &Defines,
        estimate: impl FnOnce(&SpeedTerms<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_route_terms_inner(
            division, movement, route, geography, defines, None, estimate,
        )
    }

    fn with_route_terms_inner<T>(
        &self,
        division: &DivisionRow,
        movement: &DivisionMovementRow,
        route: &[u32],
        geography: &impl Geography,
        defines: &Defines,
        panel_combat: Option<i64>,
        estimate: impl FnOnce(&SpeedTerms<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        use engine_executable::modifier_keys as keys;
        let transport = |from: u32| -> Result<EdgeTerm, String> {
            let region = geography
                .region(from)
                .ok_or_else(|| format!("province {from} has no strategic region"))?;
            Ok(EdgeTerm::Scaled {
                speed: self.transport_speed(division.logical_owner, region)?,
                scale: defines.transport_scale,
            })
        };
        if movement.naval_transfer != 0 {
            let edge = |from: u32, _: u32| transport(from);
            return estimate(&SpeedTerms {
                modifier: 0,
                edge: &edge,
                organisation: None,
                fuel: None,
                base_speed: self.base_speed(movement),
            });
        }
        let invasion = geography
            .terrain_flags(division.province)
            .ok_or_else(|| format!("province {} has no content terrain", division.province))?
            & (engine_native::cprovince::definition::LAND_FLAG
                | engine_native::cprovince::definition::LAKE_FLAG)
            == 0
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
                    return transport(from).map(Some);
                }
            }
            Ok(invasion.then_some(EdgeTerm::Fixed(defines.invasion_speed)))
        };
        let mut from = division.province;
        let mut normal = movement
            .path
            .first()
            .map(|&first| special_edge(division.province, first))
            .transpose()?
            .is_some_and(|term| term.is_none());
        for &to in route {
            if from != to {
                normal |= special_edge(from, to)?.is_none();
            }
            from = to;
        }
        if !normal {
            let edge = |from, to| {
                special_edge(from, to)?.ok_or_else(|| {
                    "the requested edge is outside the division's sea or invasion itinerary"
                        .to_owned()
                })
            };
            return estimate(&SpeedTerms {
                modifier: 0,
                edge: &edge,
                organisation: None,
                fuel: None,
                base_speed: self.base_speed(movement),
            });
        }
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
                        Ok(EdgeTerm::Scaled {
                            speed: self.rail_speed(from, to, &rail)?,
                            scale: defines.outer_scale,
                        })
                    };
                    return estimate(&SpeedTerms {
                        modifier: 0,
                        edge: &edge,
                        organisation: None,
                        fuel: None,
                        base_speed: self.base_speed(movement),
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
        let withdrawing = *movement
            .withdrawal
            .require("DivisionMovement.withdrawal")?
            .as_ref()
            .ok_or("the division's withdrawal byte was refused when recorded")?
            != 0;
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
        let leader_value =
            |index: usize, key: u32| -> Result<i64, String> { self.effective(key, leader[index]) };
        let attacking = std::cell::OnceCell::new();
        let attacking = || -> Result<bool, String> {
            attacking
                .get_or_init(|| self.attacking(division, movement))
                .clone()
        };
        let combat_base = if movement.combat_count != 0 && !withdrawing {
            Some(self.combat_base(status, defines)?)
        } else {
            None
        };
        let hostile = attacking()?;
        let [state_speed, state_attack_speed] = self.state_speed(division, hostile)?;
        let exiled = status.move_flags & DivisionStatusRow::MOVE_EXILED != 0;
        let mut modifiers = CommonModifiers {
            combat_base,
            country_speed: self.effective(keys::ARMY_SPEED_FACTOR, country.army_speed_factor)?,
            leader_speed: leader_value(0, keys::ARMY_SPEED_FACTOR)?,
            country_attack_speed: self.effective(
                keys::ARMY_ATTACK_SPEED_FACTOR,
                country.army_attack_speed_factor,
            )?,
            state_speed,
            state_attack_speed,
            unit_speed: self.unit_speed(division)?,
            noncore: if exiled {
                0
            } else {
                self.noncore(division, status, defines)?
            },
            weather: self.weather(division.province, geography)?,
            armour_speed: [0; 2],
        };
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
            modifiers.state_attack_speed = 0;
        }
        let supply = if exiled {
            0
        } else {
            let operands = self.supply_operands(status, None)?;
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
                    leader_value(5, keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR)?,
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
            super::common_modifier(&modifiers, hostile, acclimatization, supply, air, retreat)
                .wrapping_add(panel_combat.unwrap_or(100_000).wrapping_sub(100_000));
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
        let organisation = if panel_combat.is_some() {
            let maximum = self
                .panel_stat(division.id, 1)
                .map(|maximum| maximum.max(100))
                .ok_or_else(|| "deployed maximum organisation cache unavailable".to_owned());
            super::organisation_factor(
                maximum.map(|maximum| (division.organisation, maximum)),
                attacking,
                defines,
            )?
        } else {
            self.organisation(division, movement, defines)?
        };
        let river_leader = leader_value(4, keys::RIVER_CROSSING_FACTOR)?;
        let owner = Self::owner(division);
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
                    river_leader.wrapping_add(self.effective(
                        keys::RIVER_CROSSING_FACTOR_AGAINST,
                        self.river_relation(owner, self.province(to)?.controller)?,
                    )?),
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
            base_speed: self.base_speed(movement),
        })
    }
}
