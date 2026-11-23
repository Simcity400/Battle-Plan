use crate::naval_missions::MissionIcons;
use content_reader_layers::ResolvedLayers;
use content_reader_map::map::ProvinceKind;
use crate::naval_missions;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::io;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use viewer_counter_kit::CounterMap;
use viewer_unit_reads::counter_inputs::NativeInputs;
use engine_executable::modifier_keys as keys;
use replay_data::rows::*;
use replay_derive::naval_mission_derive as derive;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};
use dashboard_text::table_access::TokenError;
use dashboard_text::table_access::TokenNames;

type Fact<T> = Result<T, String>;
pub mod hover;
pub const UNKNOWN: &str = "naval mission operands are undecided (see named input gaps)";

type RegionTopology = (
    BTreeMap<u32, Vec<content_reader_naval_path::naval_path::RegionEdge>>,
    BTreeSet<u32>,
);

pub struct Content {
    terrain: content_reader_key_names::terrain::TerrainDefinitions,
    pub allegiance_group_cutoff: f32,
    naval_terrain: BTreeMap<u32, Vec<u8>>,
    efficiencies: derive::Defines,
    spotting: hover::patrol::Defines,
    patrol_air: patrol_air::Defines,
    convoy: hover::convoy::Defines,
    dominance: replay_derive::naval_dominance_derive::Defines,
    fuel_factor: i64,
    underway_factor: i64,
    distance_factor: i64,
    edges: OnceLock<RegionTopology>,
    distances: OnceLock<Mutex<content_reader_naval_path::naval_path::Distances>>,
}

impl Content {
    pub fn read(
        terrain: content_reader_key_names::terrain::TerrainDefinitions,
        layers: &ResolvedLayers,
        regions: &BTreeMap<u32, content_reader_regions::regions::StrategicRegion>,
        defines: &content_reader_defines::defines::Defines,
    ) -> io::Result<Self> {
        Ok(Self {
            allegiance_group_cutoff: defines.number(
                "NGraphics",
                "NAVAL_MISSION_TASK_FORCES_GROUP_BY_ALLEGIANCE_CUTOFF",
            )? as f32,
            terrain,
            naval_terrain: regions
                .iter()
                .filter_map(|(&id, region)| {
                    region
                        .naval_terrain
                        .as_ref()
                        .map(|name| (id, name.as_bytes().to_vec()))
                })
                .collect(),
            efficiencies: derive::Defines::read(|section, name| defines.fixed5(section, name))?,
            spotting: hover::patrol::Defines::read(defines)?,
            patrol_air: patrol_air::Defines::read(defines, layers)?,
            convoy: hover::convoy::Defines::read(defines)?,
            dominance: replay_derive::naval_dominance_derive::Defines::read(|section, name| {
                defines.fixed5(section, name)
            })?,
            fuel_factor: defines.fixed5("NNavy", "OUT_OF_FUEL_RANGE_FACTOR")?,
            underway_factor: defines.fixed5("NNavy", "UNDERWAY_REPLENISHMENT_RANGE_FACTOR")?,
            distance_factor: defines.fixed5("NNavy", "NAVAL_RANGE_TO_INGAME_DISTANCE")?,
            edges: OnceLock::new(),
            distances: OnceLock::new(),
        })
    }

    fn terrain(&self, region: Option<u32>) -> Fact<&content_reader_key_names::terrain::TerrainCategory> {
        let at = region
            .and_then(|id| self.naval_terrain.get(&id))
            .and_then(|name| self.terrain.category(name))
            .or_else(|| self.terrain.category(b"ocean"));
        at.map(|at| &self.terrain.categories[at])
            .ok_or_else(|| "naval terrain and native ocean fallback are undefined".into())
    }
}

fn data<T: TableSchema>(frame: &DecodedFrame) -> Fact<&[T::Row]> {
    frame.complete::<T>().map_err(|error| error.to_string())
}

type Topology<'a> = Fact<(
    &'a content_reader_map::province_graph::ProvinceGraph,
    &'a content_reader_map::province_geometry::ProvinceGeometry,
)>;

pub struct Hour<'a> {
    frame: &'a DecodedFrame,
    content: &'a Content,
    anchors: &'a CounterMap,
    native: &'a NativeInputs,
    links: &'a BTreeMap<u32, u32>,
    ids: &'a BTreeMap<u32, u32>,
    forces: HashMap<u64, &'a TaskForceRow>,
    inputs: HashMap<u64, &'a NavalMissionTaskForceRow>,
    countries: HashMap<u32, &'a NavalMissionCountryRow>,
    commanders: HashMap<(u32, u64, u8), &'a NavalMissionCommanderRow>,
    fleet_forces: HashMap<u64, Vec<&'a TaskForceRow>>,
    ships: OnceLock<Fact<HashMap<u64, Vec<&'a ShipRow>>>>,
    fleets: Fact<HashMap<u64, &'a FleetRow>>,
    flags: OnceLock<Fact<HashMap<u64, u64>>>,
    own_distances: Mutex<Option<content_reader_naval_path::naval_path::Distances>>,
    definitions: Fact<HashMap<u32, &'a ModifierDefinitionRow>>,
    names: Fact<TokenNames<'a>>,
    features: Option<u32>,
    combined: HashMap<(u64, u32), Fact<i64>>,
    patrol_air: OnceLock<Fact<patrol_air::Prepared<'a>>>,
    patrol_navy: HashMap<(u64, usize), Fact<i64>>,
    patrol_radar: OnceLock<Fact<BTreeMap<u32, i32>>>,
    topology: Topology<'a>,
    ports: Fact<BTreeMap<u32, u32>>,
    bases: Fact<BTreeSet<u32>>,
    moves: Fact<viewer_unit_reads::movement_reads::Moves<'a, 'a>>,
    ours: u32,
}

impl<'a> Hour<'a> {
    pub fn read(
        frame: &'a DecodedFrame,
        anchors: &'a CounterMap,
        mission_content: &'a Result<Content, String>,
        native: &'a NativeInputs,
        countries: (&'a BTreeMap<u32, u32>, &'a BTreeMap<u32, u32>),
        forces: HashMap<u64, &'a TaskForceRow>,
        (topology, moves): (
            Topology<'a>,
            Fact<viewer_unit_reads::movement_reads::Moves<'a, 'a>>,
        ),
        ours: u32,
    ) -> Fact<Self> {
        let content = mission_content.as_ref().map_err(Clone::clone)?;
        let mut fleet_forces: HashMap<_, Vec<_>> = HashMap::new();
        for &force in forces.values() {
            fleet_forces.entry(force.fleet).or_default().push(force);
        }
        Ok(Self {
            frame,
            content,
            anchors,
            native,
            links: countries.0,
            ids: countries.1,
            forces,
            inputs: data::<tables::NavalMissionTaskForces>(frame)?
                .iter()
                .map(|row| (row.task_force, row))
                .collect(),
            countries: data::<tables::NavalMissionCountries>(frame)?
                .iter()
                .map(|row| (row.country, row))
                .collect(),
            commanders: data::<tables::NavalMissionCommanders>(frame)?
                .iter()
                .map(|row| ((row.country, row.reference, row.source), row))
                .collect(),
            fleet_forces,
            ships: OnceLock::new(),
            fleets: data::<tables::Fleets>(frame)
                .map(|rows| rows.iter().map(|row| (row.id, row)).collect()),
            flags: OnceLock::new(),
            own_distances: Mutex::new(None),
            definitions: data::<tables::ModifierDefinitions>(frame)
                .map(|rows| rows.iter().map(|row| (row.index, row)).collect()),
            names: TokenNames::new(frame).map_err(|error| error.to_string()),
            features: data::<tables::SimulationFeatures>(frame)
                .ok()
                .and_then(|rows| rows.first())
                .map(|row| row.dlc_mask),
            combined: HashMap::new(),
            patrol_air: OnceLock::new(),
            patrol_navy: HashMap::new(),
            patrol_radar: OnceLock::new(),
            topology,
            ports: data::<tables::Provinces>(frame).map(|rows| {
                rows.iter()
                    .map(|row| (row.id, row.port_sea as u32))
                    .collect()
            }),
            bases: data::<tables::NavalBases>(frame)
                .map(|rows| rows.iter().map(|row| row.province).collect()),
            moves,
            ours,
        })
    }

    fn force(&self, handle: u64) -> Fact<&'a TaskForceRow> {
        self.forces
            .get(&handle)
            .copied()
            .ok_or_else(|| format!("naval mission task force {handle:#x} is absent"))
    }
    fn input(&self, handle: u64) -> Fact<&'a NavalMissionTaskForceRow> {
        self.inputs
            .get(&handle)
            .copied()
            .map(Ok)
            .unwrap_or_else(|| {
                data::<tables::NavalDominanceTaskForceRanges>(self.frame)?
                    .iter()
                    .find(|row| row.task_force == handle)
                    .ok_or_else(|| {
                        format!("task force {handle:#x} has no naval dominance range operands")
                    })
            })
    }
    fn country_id(&self, raw: u32) -> Fact<u32> {
        self.links
            .get(&raw)
            .and_then(|slot| self.ids.get(slot))
            .copied()
            .ok_or_else(|| format!("naval mission country {raw} is not linked"))
    }
    fn country(&self, raw: u32) -> Fact<&'a NavalMissionCountryRow> {
        let id = self.country_id(raw)?;
        self.countries
            .get(&id)
            .copied()
            .ok_or_else(|| format!("country {id} has no NavalMissionCountries row"))
    }
    fn effective(&self, key: u32, raw: Option<i64>) -> Fact<i64> {
        if raw.is_none() {
            return Ok(0);
        }
        let definition = self
            .definitions
            .as_ref()
            .map_err(Clone::clone)?
            .get(&key)
            .ok_or_else(|| format!("naval mission modifier {key:#x} has no recorded definition"))?;
        derive::effective(definition, self.features, raw).map_err(str::to_owned)
    }
    fn country_value(&self, raw: u32, key: u32) -> Fact<i64> {
        let row = self.country(raw)?;
        self.effective(key, row.value(key)?)
    }
    fn commander_value(&self, country: u32, reference: u64, source: u8, key: u32) -> Fact<i64> {
        if reference == 0 {
            return Ok(0);
        }
        if replay_data::sliced::read_hour(
            data::<tables::SliceReads>(self.frame)?,
            TableId::NavalMissionCommanders,
            country,
        )
        .is_none()
        {
            return Err(format!(
                "country {country} has no NavalMissionCommanders slice witness"
            ));
        }
        let row = self
            .commanders
            .get(&(country, reference, source))
            .ok_or_else(|| {
                if source != 0
                    && (1..5)
                        .any(|source| self.commanders.contains_key(&(country, reference, source)))
                {
                    format!(
                        "headquarters commander {reference:#x} has an incomplete source quartet"
                    )
                } else {
                    format!(
                        "commander {reference:#x} source {source} has no NavalMissionCommanders row"
                    )
                }
            })?;
        self.effective(key, row.value(key)?)
    }

    fn ships(&self, handle: u64) -> Fact<&[&'a ShipRow]> {
        Ok(self
            .ships
            .get_or_init(|| {
                data::<tables::Ships>(self.frame).map(|rows| {
                    let mut ships: HashMap<_, Vec<_>> = HashMap::new();
                    for ship in rows {
                        ships.entry(ship.task_force).or_default().push(ship);
                    }
                    ships
                })
            })
            .as_ref()
            .map_err(Clone::clone)?
            .get(&handle)
            .map_or(&[], Vec::as_slice))
    }
    fn terrain_value(&self, force: &TaskForceRow, key: u32) -> Fact<i64> {
        let ships = self.ships(force.id)?;
        if ships.is_empty() {
            return Ok(0);
        }
        let terrain = self
            .content
            .terrain(self.anchors.region_of.get(&force.province).copied())?;
        let definition = self
            .definitions
            .as_ref()
            .map_err(Clone::clone)?
            .get(&key)
            .ok_or_else(|| format!("naval terrain modifier {key:#x} has no recorded definition"))?;
        let names = self.names.as_ref().map_err(Clone::clone)?;
        let key_name = names
            .name(definition.token)
            .map_err(|error| error.to_string())?;
        let raw = |block: &content_reader_script::modifiers::Modifiers| -> Fact<Option<i64>> {
            if let Some(reason) = block.unheld.cause() {
                return Err(format!(
                    "terrain {} modifier {key_name}: {reason}",
                    String::from_utf8_lossy(&terrain.name)
                ));
            }
            Ok(block
                .values
                .iter()
                .find(|value| value.name.eq_ignore_ascii_case(key_name))
                .map(|value| value.value))
        };
        let common = raw(&terrain.modifiers)?;
        let mut classes: HashMap<i32, i32> = HashMap::new();
        for ship in ships {
            *classes.entry(ship.class_id).or_default() += 1;
        }
        let mut terms = Vec::with_capacity(classes.len());
        for (class, count) in classes {
            let own = match names.name(class as u32) {
                Ok(class_name) => {
                    let subunits = self.anchors.subunit_tokens.as_ref().map_err(Clone::clone)?;
                    if subunits.resolve(class_name)?.is_none() {
                        None
                    } else {
                        terrain
                            .classes
                            .iter()
                            .find(|(name, _)| name.eq_ignore_ascii_case(class_name.as_bytes()))
                            .map(|(_, block)| raw(block))
                            .transpose()?
                            .flatten()
                    }
                }
                Err(TokenError::Rejected | TokenError::Absent(_)) => None,
                Err(error) => return Err(error.to_string()),
            };
            let value = match (common, own) {
                (None, None) => None,
                (a, b) => Some(a.unwrap_or(0).wrapping_add(b.unwrap_or(0))),
            };
            terms.push((count, self.effective(key, value)?));
        }
        Ok(derive::terrain_mean(terms))
    }
    fn combined(&mut self, handle: u64, key: u32) -> Fact<i64> {
        if let Some(value) = self.combined.get(&(handle, key)) {
            return value.clone();
        }
        let value = self.combine(handle, key);
        self.combined.insert((handle, key), value.clone());
        value
    }
    fn combine(&self, handle: u64, key: u32) -> Fact<i64> {
        let force = self.force(handle)?;
        let row = self.input(handle)?;
        let country = self.country_id(force.roster_owner)?;
        let location = if force.province == 0 {
            None
        } else {
            let (local, weather) = row.value(key);
            Some(derive::Location {
                terrain: self.terrain_value(force, key)?,
                weather: self.effective(key, weather)?,
                weather_reduction: derive::weather_reduction(
                    self.country_value(force.roster_owner, keys::NAVY_WEATHER_PENALTY)?,
                ),
                local: self.effective(key, local)?,
            })
        };
        let mut headquarters = Vec::with_capacity(row.headquarters.len());
        for &reference in row.headquarters.iter() {
            let mut values = [0; 4];
            for (at, value) in values.iter_mut().enumerate() {
                *value = self.commander_value(country, reference, at as u8 + 1, key)?;
            }
            headquarters.push(derive::headquarters_sum(values));
        }
        Ok(derive::combined(
            self.country_value(force.roster_owner, key)?,
            location,
            self.commander_value(country, row.leader, 0, key)?,
            headquarters,
        ))
    }
    fn siblings(&self, handle: u64, kind: u32) -> Fact<Vec<&'a TaskForceRow>> {
        let force = self.force(handle)?;
        let row = self.input(handle)?;
        let mut result = Vec::new();
        for sibling in self.fleet_forces.get(&force.fleet).into_iter().flatten() {
            if sibling.mission != kind {
                continue;
            }
            let counter = self.native.task_force(sibling.id).ok_or_else(|| {
                format!(
                    "task force {:#x} has no TaskForceCounterInputs is_in_regions",
                    sibling.id
                )
            })?;
            if counter.is_in_regions == 0 {
                continue;
            }
            let input = self.input(sibling.id)?;
            if (input.range_fleet, input.range_begin, input.range_end)
                == (row.range_fleet, row.range_begin, row.range_end)
            {
                result.push(*sibling);
            }
        }
        Ok(result)
    }
    fn raiding(&mut self, handle: u64) -> Fact<i64> {
        let force = self.force(handle)?;
        if force.fleet == 0 {
            return Ok(100_000);
        }
        let row = self.input(handle)?;
        let range = [row.range_begin, row.range_end];
        if range[1].wrapping_sub(range[0]) == 0 {
            return Ok(100_000);
        }
        let count = self.siblings(handle, 3)?.len() as i32;
        derive::raiding_efficiency(
            true,
            range,
            count,
            derive::raid_region_ratio(
                self.combined(handle, keys::NAVAL_COORDINATION)?,
                &self.content.efficiencies,
            ),
            row.raiding_factor,
            self.combined(handle, keys::CONVOY_RAIDING_EFFICIENCY_FACTOR)?,
            &self.content.efficiencies,
        )
        .map_err(str::to_owned)
    }
    fn escort(&mut self, handle: u64) -> Fact<i64> {
        let row = self.input(handle)?;
        let mut shares = Vec::new();
        for sibling in self.siblings(handle, 4)? {
            shares.push(derive::EscortShare {
                ships: self.ships(sibling.id)?.len() as u32,
                capacity: derive::escort_capacity(
                    self.combined(sibling.id, keys::NAVAL_COORDINATION)?,
                    &self.content.efficiencies,
                ),
            });
        }
        derive::escort_efficiency(
            shares,
            row.convoys,
            self.country_value(row.mission_owner, keys::CONVOY_ESCORT_EFFICIENCY)?,
            derive::escort_region_ratio(
                self.combined(handle, keys::NAVAL_COORDINATION)?,
                &self.content.efficiencies,
            ),
            [row.range_begin, row.range_end],
            &self.content.efficiencies,
        )
        .map_err(str::to_owned)
    }

    fn dominance_design_power(&self, handle: u64) -> Fact<i64> {
        let models = data::<tables::ShipModels>(self.frame)?;
        let mut power = 0_i64;
        for ship in self.ships(handle)? {
            let model = models
                .binary_search_by_key(&ship.id, |row| row.ship)
                .ok()
                .map(|index| &models[index])
                .ok_or_else(|| {
                    format!("naval dominance ship {:#x} has no ShipModels row", ship.id)
                })?;
            if model.variant == 0 {
                continue;
            }
            let raw = model
                .naval_power
                .require("ShipModels.naval_power")?
                .ok_or_else(|| {
                    format!(
                        "naval dominance ship {:#x} selected design capture refused",
                        ship.id
                    )
                })?;
            power = power.wrapping_add(replay_derive::naval_dominance_derive::ship_power(
                raw,
                self.content.dominance,
            ));
        }
        Ok(power)
    }
    fn dominance_mission_power(&mut self, handle: u64, region: u32) -> Fact<i64> {
        let force = self.force(handle)?;
        if self.ships(handle)?.is_empty() {
            return Ok(0);
        }
        let counter = self.native.task_force(handle).ok_or_else(|| {
            format!("naval dominance task force {handle:#x} active state unavailable")
        })?;
        if counter.is_in_regions == 0 {
            return Ok(0);
        }
        let missions = data::<tables::TaskForceMissions>(self.frame)?;
        let Some(mission) = missions
            .binary_search_by_key(&handle, |row| row.task_force)
            .ok()
            .map(|index| &missions[index])
        else {
            return Ok(0);
        };
        if !mission.regions.contains(&region) {
            return Ok(0);
        }
        let factor = match force.mission {
            1 if mission.current_region == region => 100_000,
            1 => return Ok(0),
            2 => {
                if force.fleet == 0 {
                    100_000
                } else {
                    let siblings = self
                        .fleet_forces
                        .get(&force.fleet)
                        .cloned()
                        .unwrap_or_default();
                    let total = siblings.len() as i64 * 100_000;
                    let mut reached = 0_i64;
                    for sibling in siblings {
                        if self.active(sibling.id, region)? {
                            reached = reached.wrapping_add(100_000);
                        }
                    }
                    if total == 0 {
                        0
                    } else {
                        if (reached as u64).wrapping_add(0x7fff_ffff_ffff_fffe)
                            < 0xffff_ffff_ffff_fffd
                        {
                            reached.wrapping_mul(100_000) / total
                        } else {
                            0xffff_ffff
                        }
                    }
                }
            }
            3 => self.raiding(handle)?,
            4 => self.escort(handle)?,
            _ => return Ok(0),
        };
        Ok(self.dominance_design_power(handle)?.wrapping_mul(factor) / 100_000)
    }
    fn dominance_region(
        &mut self,
        region: u32,
        owner: u32,
        enemies: &[u32],
    ) -> Fact<viewer_unit_reads::naval_dominance_sources::RegionalDominanceSources> {
        let ledgers = data::<tables::NavalMissionLedgers>(self.frame)?;
        let mut patrol = 0_i64;
        let mut friendly_strike = 0_i64;
        let mut enemy_raiding = 0_i64;
        let mut enemy_strike = 0_i64;
        if let Some(ledger) = ledgers
            .iter()
            .find(|row| row.owner == owner && row.region == region)
        {
            for &handle in &ledger.task_forces {
                match self.force(handle)?.mission {
                    1 => {
                        patrol = patrol.wrapping_add(self.dominance_mission_power(handle, region)?)
                    }
                    2 => {
                        friendly_strike =
                            friendly_strike.wrapping_add(self.dominance_strike(handle, region)?)
                    }
                    _ => {}
                }
            }
        }
        for &enemy in enemies {
            let id = self.country_id(enemy)?;
            if let Some(ledger) = ledgers
                .iter()
                .find(|row| row.owner == id && row.region == region)
            {
                for &handle in &ledger.task_forces {
                    let force = self.force(handle)?;
                    if force.mission == 3
                        && self.anchors.region_of.get(&force.province) == Some(&region)
                    {
                        enemy_raiding = enemy_raiding
                            .wrapping_add(self.dominance_mission_power(handle, region)?);
                    } else if force.mission == 2 {
                        enemy_strike =
                            enemy_strike.wrapping_add(self.dominance_strike(handle, region)?);
                    }
                }
            }
        }
        Ok(
            viewer_unit_reads::naval_dominance_sources::RegionalDominanceSources {
                patrol,
                strike_multiplier: 100_000_i64.wrapping_add(friendly_strike),
                enemy_raiding: 100_000_i64
                    .wrapping_add(enemy_strike)
                    .wrapping_mul(enemy_raiding)
                    / 100_000,
            },
        )
    }
    fn dominance_strike(&mut self, handle: u64, region: u32) -> Fact<i64> {
        let missions = data::<tables::TaskForceMissions>(self.frame)?;
        let count = missions
            .binary_search_by_key(&handle, |row| row.task_force)
            .ok()
            .map_or(0, |index| missions[index].regions.len() as i32);
        Ok(replay_derive::naval_dominance_derive::strike_effect(
            self.dominance_mission_power(handle, region)?,
            count,
            self.content.dominance,
        ))
    }
    pub fn dominance_sources(
        &mut self,
    ) -> viewer_unit_reads::naval_dominance_sources::RegionalDominanceSourcesMap {
        self.dominance_sources_in(None)
    }

    pub fn dominance_sources_in(
        &mut self,
        only: Option<u32>,
    ) -> viewer_unit_reads::naval_dominance_sources::RegionalDominanceSourcesMap {
        let owner = self
            .ids
            .get(&self.ours)
            .copied()
            .ok_or_else(|| "naval dominance source viewer country unavailable".to_owned())?;
        let enemies = replay_derive::naval_region_control::raw_roster(
            self.frame,
            self.ours,
            AirCountryRelationRow::ENEMY,
        )?;
        let mut owners = BTreeSet::from([owner]);
        for &enemy in &enemies {
            owners.insert(self.country_id(enemy)?);
        }
        let regions: BTreeSet<_> = data::<tables::NavalMissionLedgers>(self.frame)?
            .iter()
            .filter(|row| owners.contains(&row.owner) && only.is_none_or(|only| only == row.region))
            .map(|row| row.region)
            .collect();
        Ok(regions
            .into_iter()
            .map(|region| (region, self.dominance_region(region, owner, &enemies)))
            .collect())
    }
    fn decorate(
        &mut self,
        entry: &mut naval_missions::MissionEntry,
        ledger: &NavalMissionLedgerRow,
    ) -> Fact<()> {
        match entry.kind {
            2 | 9 => {
                let mut active = 0;
                for &handle in &ledger.task_forces {
                    if self.force(handle)?.mission == entry.kind {
                        active += u32::from(self.active(handle, ledger.region)?);
                    }
                }
                entry.active = Ok(Some(active));
                entry.alert = if active < entry.total {
                    Ok(true)
                } else {
                    entry.engagement
                };
            }
            3 => {
                let mut values = Vec::new();
                for &handle in &ledger.task_forces {
                    if self.force(handle)?.mission == 3 {
                        values.push(self.raiding(handle)?);
                    }
                }
                let value = derive::raiding_ledger(values);
                entry.efficiency = Some(Ok(value));
                entry.alert = if value < 100_000 {
                    Ok(true)
                } else {
                    entry.engagement
                };
            }
            4 => {
                let registrations = ledger
                    .task_forces
                    .iter()
                    .map(|&handle| {
                        let force = self.force(handle)?;
                        Ok(derive::Registration {
                            kind: force.mission,
                            fleet: force.fleet,
                            convoys: if force.mission == 4 {
                                self.input(handle)?.convoys
                            } else {
                                0
                            },
                        })
                    })
                    .collect::<Fact<Vec<_>>>()?;
                let mut failure = None;
                let escort = derive::escort_ledger(&registrations, |at| {
                    self.escort(ledger.task_forces[at]).map_err(|reason| {
                        failure = Some(reason);
                        UNKNOWN
                    })
                })
                .map_err(|reason| failure.unwrap_or_else(|| reason.into()))?;
                let presence = derive::presence(ledger.ring, ledger.positive, ledger.total);
                entry.efficiency = Some(Ok(derive::escort_shown(escort, presence)));
                entry.alert = Ok(derive::escort_alert(
                    escort,
                    presence,
                    &self.content.efficiencies,
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn admits(&self, name: &str, owner: u32, submarine: bool) -> Fact<bool> {
        let (graph, _) = self.topology.as_ref().map_err(Clone::clone)?;
        let rule = content_reader_map::adjacency_rules::find(&graph.rules, name)
            .ok_or_else(|| format!("naval adjacency rule {name} is undefined"))?;
        self.moves
            .as_ref()
            .map_err(Clone::clone)?
            .naval_rule(rule, owner, submarine)
    }

    fn active(&mut self, handle: u64, region: u32) -> Fact<bool> {
        let (graph, geometry) = *self.topology.as_ref().map_err(Clone::clone)?;
        self.ports.as_ref().map_err(Clone::clone)?;
        self.bases.as_ref().map_err(Clone::clone)?;
        let members = self
            .anchors
            .region_provinces
            .get(&region)
            .ok_or_else(|| format!("mission region {region} is undefined"))?;
        let sea: Vec<_> = members
            .iter()
            .copied()
            .filter(|&province| geometry.kind_flags(province) == Some(0))
            .collect();
        if sea.is_empty() {
            return Ok(false);
        }
        let force = self.force(handle)?;
        let row = self.input(handle)?;
        let underway = if row.underway == 0 {
            0
        } else {
            let owner = if (force.expeditionary_owner as i32) > 0 {
                force.expeditionary_owner
            } else {
                force.roster_owner
            };
            derive::underway_bonus(
                row.replenishment,
                row.replenishment_limit,
                self.country_value(owner, keys::UNDERWAY_REPLENISHMENT_RANGE_FACTOR)?,
                self.content.underway_factor,
            )
        };
        let range = derive::mission_range(
            row.range_base,
            self.combined(handle, keys::NAVAL_RANGE)?,
            self.combined(handle, keys::NAVAL_RANGE_FACTOR)?,
            row.fuel,
            row.fuel_requested,
            self.content.fuel_factor,
            underway,
            self.content.distance_factor,
        );
        if range < 0 {
            return Ok(false);
        }
        let ports = self.ports.as_ref().map_err(Clone::clone)?;
        let bases = self.bases.as_ref().map_err(Clone::clone)?;
        let source = match geometry.kind_flags(force.province) {
            None if force.province == 0 => return Ok(false),
            None => {
                return Err(format!(
                    "task force province {} is undefined",
                    force.province
                ));
            }
            Some(flags) if flags & 1 != 0 => ports.get(&force.province).copied().unwrap_or(0),
            Some(_) => force.province,
        };
        if source == 0 {
            return Ok(false);
        }
        let from = self
            .anchors
            .region_of
            .get(&source)
            .copied()
            .ok_or_else(|| format!("naval source province {source} has no strategic region"))?;
        let mut all_flag_seven = true;
        for ship in self.ships(handle)? {
            all_flag_seven &= self
                .flags
                .get_or_init(|| {
                    data::<tables::ShipModels>(self.frame).map(|rows| {
                        rows.iter()
                            .map(|row| (row.ship, row.formation_flags))
                            .collect()
                    })
                })
                .as_ref()
                .map_err(Clone::clone)?
                .get(&ship.id)
                .ok_or_else(|| format!("ship {:#x} has no ShipModels formation flags", ship.id))?
                & 0x80
                != 0;
        }
        let (edges, sea_regions) = self.content.edges.get_or_init(|| {
            (
                content_reader_naval_path::naval_path::region_edges(
                    graph,
                    geometry,
                    &self.anchors.region_provinces,
                    &self.anchors.region_of,
                ),
                self.anchors
                    .region_provinces
                    .iter()
                    .filter(|(_, provinces)| {
                        provinces
                            .iter()
                            .any(|&id| geometry.kind_flags(id) == Some(0))
                    })
                    .map(|(&id, _)| id)
                    .collect(),
            )
        });
        if !content_reader_naval_path::naval_path::region_reaches(
            edges,
            sea_regions,
            from,
            region,
            |name| self.admits(name, row.mission_owner, all_flag_seven),
        )? {
            return Ok(false);
        }
        if force.fleet == 0 {
            return Ok(false);
        }
        let fleet = self
            .fleets
            .as_ref()
            .map_err(Clone::clone)?
            .get(&force.fleet)
            .ok_or_else(|| format!("fleet {:#x} has no Fleets row", force.fleet))?;
        if fleet.home_base == 0 {
            return Ok(false);
        }
        let hubs = &self.country(row.mission_owner)?.hubs;
        let shared = self
            .content
            .distances
            .get_or_init(|| {
                Mutex::new(content_reader_naval_path::naval_path::Distances::new(
                    geometry, ports,
                ))
            });
        let mut shared = match shared.try_lock() {
            Ok(held) => Some(held),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        };
        let mut own = self.own_distances.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let distances = match shared.as_deref_mut() {
            Some(shared) => shared,
            None => own.get_or_insert_with(|| content_reader_naval_path::naval_path::Distances::new(geometry, ports)),
        };
        for province in sea {
            for &base in std::iter::once(&fleet.home_base).chain(hubs.iter()) {
                if bases.contains(&base)
                    && distances.distance(graph, geometry, base, province)? <= range
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

pub fn decorate_profiled<'a>(
    icons: &mut MissionIcons,
    spans: &mut viewer_runtime::profiling::BuilderSpans,
    hour: impl FnOnce() -> Fact<Hour<'a>>,
) {
    icons.naval_dominance_sources = None;
    let result = spans
        .measure("overlay.tables.counters.missions.build.inputs", |_| hour())
        .and_then(|mut hour| {
            icons.naval_dominance_sources = Some(std::sync::Arc::new(spans.measure(
                "overlay.tables.counters.missions.build.dominance",
                |_| hour.dominance_sources(),
            )));
            spans.measure(
                "overlay.tables.counters.missions.build.controls_hovers",
                |_| decorate_inputs(icons, &mut hour),
            )
        });
    if let Err(reason) = result {
        if icons.naval_dominance_sources.is_none() {
            icons.naval_dominance_sources = Some(std::sync::Arc::new(Err(reason.clone())));
        }
        icons.input_gaps.insert(reason, 1);
    }
}

fn decorate_inputs(icons: &mut MissionIcons, hour: &mut Hour<'_>) -> Fact<()> {
    let owner = hour.ids.get(&hour.ours).copied();
    for ledger in data::<tables::NavalMissionLedgers>(hour.frame)?
        .iter()
        .filter(|row| owner == Some(row.owner))
    {
        let Some(entries) = icons.regions.get_mut(&ledger.region) else {
            continue;
        };
        for entry in entries.iter_mut() {
            let result = hour.decorate(entry, ledger);
            if let Err(reason) = result {
                *icons
                    .input_gaps
                    .entry(format!(
                        "region {} mission {}: {reason}",
                        ledger.region, entry.kind
                    ))
                    .or_default() += 1;
            }
        }
        hover::prepare(hour, ledger, entries, Arc::make_mut(&mut icons.hovers));
        for &handle in &ledger.task_forces {
            if icons.convoy_hovers.contains_key(&handle) {
                continue;
            }
            let admitted = (|| -> Fact<bool> {
                if hour.force(handle)?.mission != 3 {
                    return Ok(false);
                }
                let row = hour.input(handle)?;
                Ok(hour
                    .native
                    .task_force(handle)
                    .is_some_and(|row| row.is_in_regions != 0)
                    && row.raiding_region == Some(ledger.region)
                    && row
                        .raiding_targets
                        .is_some_and(|targets| targets.contains(&Some(true))))
            })();
            if admitted == Ok(true) {
                let text = hover::convoy::prepare(hour, ledger, handle);
                Arc::make_mut(&mut icons.convoy_hovers).insert(handle, text);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "naval_mission_inputs_tests.rs"]
mod tests;

mod patrol_air;
