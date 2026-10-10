//! Fresh counter queries over hourly simulation inputs, never the client's
//! hover/path cache. Route admission is the shared native `unit_path` search;
//! edge speed and duration use the engine's pure movement evaluators.

use super::MovementContent;
use crate::map::raids::reach::Moves;
use engine::{
    carmy::{MovementSpeedInputs, NormalEdgeOperands, PreparedNormalSpeed},
    crailway_manager::fresh_inputs,
    cunit::eta_ratio,
};
use installed_content::unit_path::{CounterRequest, Fact, MoveWorld};
use replay::{
    frame::DecodedFrame,
    rows::{
        CountryCounterInputRow, DivisionMovementRow, DivisionRow, FreshCounterEta, ProvinceRow,
        RailCooldownRow, StateRow, TableSchema, WeatherProvinceRow, tables,
    },
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

/// Immutable row indexes for one hour. Each domain retains its own refusal;
/// constructing the adapter cannot make a failed rail or normal domain deny
/// a route which takes only transport edges.
pub(super) struct Inputs<'a> {
    pub(super) frame: &'a DecodedFrame,
    pub(super) hour: i32,
    pub(super) content: &'a MovementContent,
    naval: OnceLock<super::naval::NavalInputs<'a>>,
    railway: OnceLock<Fact<BTreeMap<u64, &'a replay::rows::RailwayGunCounterInputRow>>>,
    movement: OnceLock<Fact<BTreeMap<u64, &'a DivisionMovementRow>>>,
    divisions: Fact<BTreeMap<u64, &'a DivisionRow>>,
    provinces: Fact<BTreeMap<u32, &'a ProvinceRow>>,
    states: Fact<BTreeMap<u32, &'a StateRow>>,
    links: Fact<BTreeMap<u32, u32>>,
    countries: Fact<BTreeMap<u32, u32>>,
    owners: Fact<BTreeMap<u32, &'a CountryCounterInputRow>>,
    cooldowns: Fact<BTreeMap<u32, &'a RailCooldownRow>>,
    edges: Fact<BTreeMap<(u32, u32), i32>>,
    weather: Fact<BTreeMap<u32, &'a WeatherProvinceRow>>,
    snow_ceiling: Fact<i64>,
}

#[derive(Clone, Copy)]
pub(super) enum Endpoint {
    Next,
    Destination,
}

pub(super) fn index<T: TableSchema, K: Ord>(
    frame: &DecodedFrame,
    key: impl Fn(&T::Row) -> K,
) -> Fact<BTreeMap<K, &T::Row>> {
    let rows = frame.complete::<T>().map_err(|error| error.to_string())?;
    let mut indexed = BTreeMap::new();
    for row in rows {
        if indexed.insert(key(row), row).is_some() {
            return Err(format!("fresh ETA {:?} contains duplicate keys", T::ID));
        }
    }
    Ok(indexed)
}

pub(super) fn available<T>(domain: &Fact<T>) -> Fact<&T> {
    domain.as_ref().map_err(Clone::clone)
}

pub(super) fn owner_entry<'a, T, K: Ord>(
    rows: &'a [T],
    key: impl Fn(&T) -> K,
    wanted: K,
    domain: &str,
) -> Fact<&'a T> {
    // Owner acquisition sorts and deduplicates its complete key demand. Keep
    // that contract explicit even for frames which bypass normal decoding.
    if let Some(pair) = rows.windows(2).find(|pair| key(&pair[0]) >= key(&pair[1])) {
        let reason = if key(&pair[0]) == key(&pair[1]) {
            "contains duplicate keys"
        } else {
            "keys are not increasing"
        };
        return Err(format!("fresh ETA {domain} {reason}"));
    }
    rows.binary_search_by(|row| key(row).cmp(&wanted))
        .map(|position| &rows[position])
        .map_err(|_| format!("fresh ETA {domain} key is outside its domain"))
}

impl<'a> Inputs<'a> {
    pub(super) fn frame(&self) -> &'a DecodedFrame {
        self.frame
    }
    pub(super) fn new(frame: &'a DecodedFrame, content: &'a MovementContent) -> Self {
        let links = index::<tables::CountryLinks, _>(frame, |row| row.id).map(|rows| {
            rows.into_iter()
                .map(|(id, row)| (id, row.canonical_slot))
                .collect()
        });
        let countries = index::<tables::Countries, _>(frame, |row| row.id).and_then(|rows| {
            let mut slots = BTreeMap::new();
            for (id, row) in rows {
                if slots.insert(row.canonical_slot, id).is_some() {
                    return Err("fresh ETA Countries contains duplicate keys".into());
                }
            }
            Ok(slots)
        });
        let edges = index::<tables::RailEdges, _>(frame, |row| (row.from, row.to)).map(|rows| {
            rows.into_iter()
                .map(|(edge, row)| (edge, row.level))
                .collect()
        });
        let snow_ceiling = (|| {
            let regions = frame
                .complete::<tables::WeatherRegions>()
                .map_err(|error| error.to_string())?;
            Ok(regions
                .first()
                .ok_or("fresh ETA weather snow ceiling is unavailable")?
                .max_snow)
        })();
        Self {
            frame,
            hour: frame.hour,
            content,
            naval: OnceLock::new(),
            railway: OnceLock::new(),
            movement: OnceLock::new(),
            divisions: index::<tables::Divisions, _>(frame, |row| row.id),
            provinces: index::<tables::Provinces, _>(frame, |row| row.id),
            states: index::<tables::States, _>(frame, |row| row.id),
            links,
            countries,
            owners: index::<tables::CountryCounterInputs, _>(frame, |row| row.country),
            cooldowns: index::<tables::RailCooldowns, _>(frame, |row| row.province),
            edges,
            weather: index::<tables::WeatherProvinces, _>(frame, |row| row.id),
            snow_ceiling,
        }
    }

    /// One native endpoint query. A first-query refusal cannot suppress a
    /// known destination query, or vice versa. Native foreign and own counter
    /// callers choose Next and Destination respectively.
    pub(super) fn derive(
        &self,
        moves: &Moves<'_, '_>,
        movement: &DivisionMovementRow,
        endpoint: Endpoint,
        local_option: bool,
    ) -> Fact<i64> {
        let movement = self.movement(movement.division)?;
        let target = endpoint.target(movement)?;
        self.derive_to(moves, movement.division, target, local_option)
            .and_then(|preview| preview.duration)
    }

    pub(super) fn route_identities(&self) -> Fact<()> {
        available(&self.provinces)?;
        available(&self.links)?;
        available(&self.countries)?;
        Ok(())
    }

    fn province(&self, id: u32) -> Fact<&ProvinceRow> {
        available(&self.provinces)?
            .get(&id)
            .copied()
            .ok_or_else(|| format!("fresh ETA province {id} is not recorded"))
    }

    fn state(&self, id: u32) -> Fact<&StateRow> {
        available(&self.states)?
            .get(&id)
            .copied()
            .ok_or_else(|| format!("fresh ETA state {id} is not recorded"))
    }

    fn snow(&self, id: u32) -> Fact<bool> {
        let Some(row) = available(&self.weather)?.get(&id) else {
            // movement_snow returns before reading MAX_SNOW on an absent
            // native per-province record. Completeness certifies that absence.
            return Ok(false);
        };
        Ok(row
            .permanent_snow
            .wrapping_add(row.snow)
            .min(*available(&self.snow_ceiling)?)
            > 0)
    }

    fn slot(&self, raw: u32) -> Fact<u32> {
        if raw == 0 {
            return Ok(0);
        }
        available(&self.links)?
            .get(&raw)
            .copied()
            .ok_or_else(|| format!("fresh ETA country reference {raw} has no link"))
    }

    fn owner(&self, raw: u32) -> Fact<&replay::rows::CountryMovementInputs> {
        // The passive current-hour owner reader explicitly refuses zero and
        // signed-nonpositive logical/land owners. Never join them to slot zero.
        if raw as i32 <= 0 {
            return Err("fresh ETA owner is not a supported positive country reference".into());
        }
        let slot = self.slot(raw)?;
        if slot == 0 {
            return Err("fresh ETA owner links to no country".into());
        }
        let id = available(&self.countries)?
            .get(&slot)
            .ok_or_else(|| format!("fresh ETA country slot {slot} is not recorded"))?;
        let movement = available(&self.owners)?
            .get(id)
            .ok_or_else(|| format!("fresh ETA owner {raw} has no CountryCounterInputs row"))?
            .movement
            .require("CountryCounterInputs.movement")
            .map_err(|error| error.to_string())?
            .as_ref()
            .ok_or("fresh ETA owner movement packet was refused")?;
        if movement.sample_hour != self.hour {
            return Err(format!(
                "fresh ETA owner inputs sampled at {}, frame is {}",
                movement.sample_hour, self.hour
            ));
        }
        Ok(movement)
    }

    pub(super) fn transport(&self, owner: u32, region: u32) -> Fact<i64> {
        let domain = self
            .owner(owner)?
            .transport
            .as_ref()
            .ok_or("fresh ETA owner transport domain was refused")?;
        owner_entry(domain, |entry| entry.region, region, "owner transport")?
            .speed
            .ok_or_else(|| format!("fresh ETA transport region {region} was refused"))
    }

    fn river(&self, owner: u32, to: u32) -> Fact<i64> {
        let target = self.slot(self.province(to)?.controller)?;
        let domain = self
            .owner(owner)?
            .river
            .as_ref()
            .ok_or("fresh ETA owner river domain was refused")?;
        owner_entry(domain, |entry| entry.target_slot, target, "owner river")?
            .modifier
            .ok_or_else(|| format!("fresh ETA river target {target} was refused"))
    }

    fn rail(&self, from: u32, to: u32) -> Fact<i64> {
        let defines = available(&self.content.rail)?;
        let target_state = self.province(to)?.state;
        let infrastructure = if target_state == 0 {
            0
        } else {
            (*self
                .state(target_state)?
                .railway_infrastructure
                .require("States.railway_infrastructure")
                .map_err(|error| error.to_string())?)
            .ok_or("fresh ETA railway infrastructure was refused")?
        };
        let percentage = self.directed_rail_percentage(from, to)?;
        Ok(fresh_inputs::road_or_rail_speed(
            infrastructure,
            percentage,
            defines.speed,
        ))
    }

    pub(super) fn directed_rail_percentage(&self, from: u32, to: u32) -> Fact<i64> {
        let defines = available(&self.content.rail)?;
        if let Some(row) = available(&self.cooldowns)?.get(&from) {
            let edge = if row.cooldown_ticks <= 0 {
                available(&self.edges)?.get(&(from, to)).copied()
            } else {
                None
            };
            let health = if row.cooldown_ticks <= 0 && edge.is_some() {
                (*row
                    .building
                    .require("RailCooldowns.building")
                    .map_err(|error| error.to_string())?)
                .map(|state| fresh_inputs::RailwayBuildingState {
                    building: state
                        .building
                        .map(|health| fresh_inputs::RailwayBuildingHealth {
                            level: health.level,
                            healthy: health.healthy,
                            partial: health.partial,
                        }),
                })
            } else {
                None
            };
            fresh_inputs::directed_percentage(
                row.cooldown_ticks,
                edge,
                health,
                defines.maximum_level,
            )
            .map_err(str::to_owned)
        } else {
            // Only a complete rail-info census certifies native no-info.
            Ok(0)
        }
    }

    pub(super) fn naval(&self) -> &super::naval::NavalInputs<'a> {
        self.naval
            .get_or_init(|| super::naval::NavalInputs::new(self.frame, self.content))
    }

    pub(super) fn railway(&self, handle: u64) -> Fact<&'a replay::rows::RailwayGunCounterInputRow> {
        available(self.railway.get_or_init(|| {
            index::<tables::RailwayGunCounterInputs, _>(self.frame, |row| row.gun)
        }))?
        .get(&handle)
        .copied()
        .ok_or_else(|| format!("fresh ETA railway gun {handle} is not recorded"))
    }

    fn remaining(
        &self,
        division: &DivisionRow,
        movement: &DivisionMovementRow,
        fresh: &FreshCounterEta,
        route: &[u32],
        source_region: impl Fn(u32) -> Fact<u32>,
    ) -> Fact<i64> {
        if route.is_empty() {
            return Ok(0);
        }
        let map = &self.content.map;
        let defines = &self.content.defines;
        let terrain = |id| {
            map.terrain(id)
                .ok_or_else(|| format!("fresh ETA province {id} has no content terrain"))
        };
        // The native original-location/ordinary-first predicate is reached
        // only after the current candidate edge's transport branch fails.
        let invasion = || -> Fact<bool> {
            if terrain(division.province)?.flags & 3 != 0 {
                return Ok(false);
            }
            movement
                .path
                .first()
                .map_or(Ok(false), |&first| Ok(terrain(first)?.flags & 1 != 0))
        };
        let land_owner = if division.expeditionary_owner as i32 > 0 {
            division.expeditionary_owner
        } else {
            division.roster_owner
        };
        let mut duration = 0_i64;
        let mut normal_terrain_checked = false;
        let mut from = division.province;
        for &to in route {
            if to == 0 {
                return Err("fresh ETA searched route contains a null province".into());
            }
            if from == to {
                continue;
            }
            let source = terrain(from)?;
            let target = terrain(to)?;
            let distance = if source.flags & 3 == 0 && target.flags & 1 != 0 {
                defines.embark_distance
            } else {
                map.distance(from, to)
                    .ok_or_else(|| format!("fresh ETA edge {from}->{to} distance is unavailable"))?
            };
            let speed = if movement.naval_transfer != 0
                || source.flags & 1 == 0
                || target.flags & 1 == 0
            {
                MovementSpeedInputs::Scaled {
                    speed: self.transport(division.logical_owner, source_region(from)?)?,
                    scale: defines.transport_scale,
                }
                .evaluate()
            } else if invasion()? {
                defines.invasion_speed
            } else if movement.strategic_redeployment != 0
                && !fresh
                    .rail
                    .ok_or("fresh ETA strategic gate was refused")?
                    .blocked()
            {
                MovementSpeedInputs::Scaled {
                    speed: self.rail(from, to)?,
                    scale: defines.outer_scale,
                }
                .evaluate()
            } else {
                let normal = fresh
                    .normal
                    .as_ref()
                    .ok_or("fresh ETA normal domain was refused")?;
                let row = self.province(to)?;
                if !normal_terrain_checked {
                    let mut keys = BTreeSet::new();
                    for entry in &normal.terrain_speed {
                        if entry.terrain <= 0 {
                            return Err(
                                "fresh ETA normal terrain contains a nonpositive key".into()
                            );
                        }
                        if !keys.insert(entry.terrain) {
                            return Err("fresh ETA normal terrain contains duplicate keys".into());
                        }
                    }
                    normal_terrain_checked = true;
                }
                let mut additive = normal
                    .terrain_speed
                    .iter()
                    .find(|entry| entry.terrain == target.index)
                    .ok_or_else(|| {
                        format!(
                            "fresh ETA terrain {} is outside its normal domain",
                            target.index
                        )
                    })?
                    .modifier;
                if row.fort_positive != 0 {
                    additive = additive.wrapping_add(normal.fort_penalty);
                }
                if self.snow(to)? {
                    additive = additive.wrapping_add(normal.snow_penalty);
                }
                if row.state != 0 {
                    additive =
                        additive.wrapping_add(self.state(row.state)?.movement_infrastructure);
                }
                let crossing = map.crossing_kind(from, to).ok_or_else(|| {
                    format!("fresh ETA edge {from}->{to} crossing is unavailable")
                })?;
                let river = if (1..=3).contains(&crossing) {
                    Some((
                        self.river(land_owner, to)?
                            .wrapping_add(normal.river_leader),
                        normal.river_adjustment,
                        match crossing {
                            2 => defines.river_crossing_two,
                            3 => defines.river_crossing_three,
                            _ => 0,
                        },
                    ))
                } else {
                    None
                };
                PreparedNormalSpeed {
                    modifier: normal.speed_modifier,
                    base_speed: normal.speed_base,
                    organisation: normal.speed_org,
                    fuel: normal.speed_fuel,
                    outer_scale: defines.outer_scale,
                }
                .evaluate(NormalEdgeOperands { additive, river })
            };
            if speed == 0 {
                // Native BECF90 returns before reaching progress subtraction
                // or any later route operand.
                return Ok(0);
            }
            duration = duration.wrapping_add(eta_ratio(distance, speed).map_err(str::to_owned)?);
            from = to;
        }
        let progress = if !route.is_empty() && route.first() == movement.path.first() {
            eta_ratio(
                movement.numerator,
                fresh
                    .current_speed
                    .ok_or("fresh ETA actual first-edge speed was refused")?,
            )
            .map_err(str::to_owned)?
        } else {
            0
        };
        Ok(duration.wrapping_sub(progress))
    }
}

impl crate::map::raids::reach::CounterFacts for Inputs<'_> {
    fn rail_connected(&self, from: u32, to: u32) -> Fact<bool> {
        // E153A0 returns before reading railway nodes unless both endpoints
        // are LAND, then proves both nodes before either selected building.
        for province in [from, to] {
            if self
                .content
                .map
                .terrain(province)
                .ok_or_else(|| format!("fresh ETA province {province} has no content terrain"))?
                .flags
                & 1
                == 0
            {
                return Ok(false);
            }
        }
        let nodes = available(&self.cooldowns)?;
        let (Some(source), Some(target)) = (nodes.get(&from), nodes.get(&to)) else {
            return Ok(false);
        };
        for row in [source, target] {
            if row.cooldown_ticks != 0 {
                return Ok(false);
            }
            let selected = row
                .building
                .require("RailCooldowns.building")
                .map_err(|error| error.to_string())?
                .as_ref()
                .ok_or("fresh ETA railway selected building was refused")?;
            if selected.building.is_none_or(|health| health.level <= 0) {
                return Ok(false);
            }
        }
        Ok(available(&self.edges)?.contains_key(&(from, to)))
    }

    fn rail_orders_group(&self, handle: u64) -> Fact<u64> {
        let fresh = self
            .railway(handle)?
            .fresh
            .require("RailwayGunCounterInputs.fresh")
            .map_err(|error| error.to_string())?
            .as_ref()
            .ok_or("fresh ETA railway inputs were refused")?;
        if fresh.sample_hour != self.hour {
            return Err("fresh ETA railway group is not from the frame hour".into());
        }
        fresh
            .orders_group
            .ok_or_else(|| "fresh ETA railway orders group was refused".into())
    }

    fn naval_access(&self, moves: &Moves<'_, '_>, handle: u64, to: u32) -> Fact<bool> {
        self.naval().access(moves, handle, to)
    }
    fn all_submarine(&self, handle: u64) -> Fact<bool> {
        self.naval().all_submarine(handle)
    }
    fn hostile_allows(&self, moves: &Moves<'_, '_>, handle: u64, to: u32) -> Fact<bool> {
        self.naval().hostile_allows(moves, handle, to)
    }
}

impl Endpoint {
    fn target(self, movement: &DivisionMovementRow) -> Fact<u32> {
        if movement.path_total == 0 {
            return Err("fresh ETA division has no ordinary route".into());
        }
        let target = match self {
            Self::Next => movement.path.first(),
            Self::Destination => {
                if movement.path.len() != movement.path_total as usize {
                    return Err("fresh ETA ordinary route is incomplete".into());
                }
                movement.path.last()
            }
        };
        target
            .copied()
            .filter(|&id| id != 0)
            .ok_or_else(|| "fresh ETA ordinary route endpoint is unavailable".into())
    }
}

fn current_hour(fresh: &FreshCounterEta, hour: i32) -> Fact<()> {
    if fresh.sample_hour == hour {
        Ok(())
    } else {
        Err(format!(
            "fresh ETA inputs sampled at {}, frame is {hour}",
            fresh.sample_hour
        ))
    }
}

impl<'a> Inputs<'a> {
    pub(super) fn movement(&self, handle: u64) -> Fact<&'a DivisionMovementRow> {
        available(
            self.movement.get_or_init(|| {
                index::<tables::DivisionMovement, _>(self.frame, |row| row.division)
            }),
        )?
        .get(&handle)
        .copied()
        .ok_or_else(|| format!("fresh ETA division {handle:#x} has no DivisionMovement row"))
    }

    fn fresh(&self, movement: &'a DivisionMovementRow) -> Fact<&'a FreshCounterEta> {
        let profile = movement
            .fresh
            .require("DivisionMovement.fresh")
            .map_err(|error| error.to_string())?
            .as_ref()
            .ok_or("fresh ETA division inputs were refused")?;
        current_hour(profile, self.hour)?;
        Ok(profile)
    }

    pub(super) fn derive_to(
        &self,
        moves: &Moves<'_, '_>,
        handle: u64,
        target: u32,
        local_option: bool,
    ) -> Fact<super::CounterPreview> {
        let movement = self.movement(handle)?;
        let division = *available(&self.divisions)?
            .get(&handle)
            .ok_or_else(|| format!("fresh ETA division {handle:#x} is not recorded"))?;
        self.route_identities()?;
        let route = moves.counter_route(
            handle,
            target,
            CounterRequest {
                strategic: movement.strategic_redeployment != 0,
                local_option,
                class: Default::default(),
            },
        )?;
        // BECF90 count<=0 never asks a speed or actual-progress operand.
        // A failed admission/search is distinguishable from missing facts:
        // native false yields Ok(empty), missing simulation inputs yield Err.
        let duration = if route.is_empty() {
            Ok(0)
        } else {
            self.fresh(movement).and_then(|fresh| {
                self.remaining(division, movement, fresh, &route, |from| {
                    moves
                        .strategic_region(from)
                        .ok_or_else(|| format!("fresh ETA source {from} has no strategic region"))
                })
            })
        };
        Ok(super::CounterPreview { duration, route })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use installed_content::movement::{MovementMap, Neighbour, ProvinceTerrain, RailMovementDefines};
    use replay::{
        frame::{Metrics, TableView},
        rows::{
            ControllerRiverSpeed, CountryLinkRow, CountryMovementInputs, CountryRow,
            FreshNormalEta, RailEdgeRow, RailwayBuildingHealth, RailwayBuildingState, Recorded,
            RegionTransportSpeed, StrategicRedeploymentInputs, TableId, TableRows, TerrainSpeed,
        },
    };

    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 20,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture domain unavailable".into(),
                }
                .into()
            }),
            metrics: Metrics::default(),
        }
    }

    fn put(frame: &mut DecodedFrame, rows: TableRows) {
        let table = rows.table();
        *frame.tables[table.index()] = TableView::Complete(rows);
    }

    fn content(source_flags: u8) -> MovementContent {
        MovementContent {
            map: MovementMap::from_parts(&[
                (
                    1,
                    ProvinceTerrain {
                        flags: source_flags,
                        index: 1,
                    },
                    vec![Neighbour {
                        to: 2,
                        distance: Some(400_000),
                        ..Default::default()
                    }],
                ),
                (2, ProvinceTerrain { flags: 1, index: 2 }, vec![]),
            ]),
            defines: replay::eta_derive::Defines {
                embark_distance: 400_000,
                transport_scale: 100_000,
                outer_scale: 100_000,
                ..Default::default()
            },
            rail: Ok(RailMovementDefines {
                speed: [300_000, 0, 0, 0],
                maximum_level: 5,
            }),
            naval: Err("unrelated naval defines are unavailable".into()),
            task_force_terrain: Err("unrelated task-force terrain is unavailable".into()),
        }
    }

    #[test]
    fn idle_explicit_target_keeps_a_known_route_when_duration_profile_is_refused() {
        use crate::map::raids::{reach::Cells, script::World};
        let (assets, mut frame) = crate::map::raids::counter_fixture();
        let mut content = content(1);
        content.map = MovementMap::from_parts(&[
            (
                3,
                ProvinceTerrain { flags: 1, index: 1 },
                vec![Neighbour {
                    to: 4,
                    distance: Some(50_000),
                    ..Default::default()
                }],
            ),
            (4, ProvinceTerrain { flags: 1, index: 2 }, vec![]),
        ]);
        put(
            &mut frame,
            TableRows::Divisions(vec![DivisionRow {
                id: 7,
                province: 3,
                logical_owner: 11,
                roster_owner: 11,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::DivisionStatus(vec![replay::rows::DivisionStatusRow {
                division: 7,
                move_flags: 0,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                division: 7,
                source: 3,
                target: 3,
                fresh: Recorded::Value(None),
                ..Default::default()
            }]),
        );
        {
            let inputs = Inputs::new(&frame, &content);
            let world = World::read(&frame, assets.facts());
            let cells = Cells::default();
            let moves = Moves::for_counter(&world, &assets, &cells, &inputs);
            let preview = inputs.derive_to(&moves, 7, 4, false).unwrap();
            assert_eq!(preview.route, [4]);
            assert!(
                preview
                    .duration
                    .unwrap_err()
                    .contains("inputs were refused")
            );
            let same = inputs.derive_to(&moves, 7, 3, false).unwrap();
            assert!(same.route.is_empty());
            assert_eq!(same.duration, Ok(0));
            assert!(
                inputs
                    .derive(
                        &moves,
                        inputs.movement(7).unwrap(),
                        Endpoint::Destination,
                        false
                    )
                    .unwrap_err()
                    .contains("has no ordinary route")
            );
        }
        let mut profile = normal();
        profile.sample_hour = frame.hour;
        assert_eq!(profile.current_speed, None);
        put(
            &mut frame,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                division: 7,
                source: 3,
                target: 3,
                fresh: Recorded::Value(Some(profile)),
                ..Default::default()
            }]),
        );
        let inputs = Inputs::new(&frame, &content);
        let world = World::read(&frame, assets.facts());
        let cells = Cells::default();
        let moves = Moves::for_counter(&world, &assets, &cells, &inputs);
        let preview = inputs.derive_to(&moves, 7, 4, false).unwrap();
        assert_eq!(preview.route, [4]);
        // Idle has no actual first edge, so the missing current_speed is unreached.
        assert_eq!(preview.duration, Ok(25_000));
    }

    fn division() -> DivisionRow {
        DivisionRow {
            id: 7,
            province: 1,
            logical_owner: 99,
            roster_owner: 99,
            ..Default::default()
        }
    }

    fn movement(first: u32) -> DivisionMovementRow {
        DivisionMovementRow {
            division: 7,
            path: vec![first],
            path_total: 1,
            ..Default::default()
        }
    }

    fn normal() -> FreshCounterEta {
        FreshCounterEta {
            sample_hour: 20,
            normal: Some(FreshNormalEta {
                speed_modifier: 100_000,
                speed_base: 200_000,
                terrain_speed: vec![TerrainSpeed {
                    terrain: 2,
                    modifier: 0,
                }],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn normal_frame() -> DecodedFrame {
        let mut frame = frame();
        put(
            &mut frame,
            TableRows::Provinces(vec![ProvinceRow {
                id: 2,
                ..Default::default()
            }]),
        );
        put(&mut frame, TableRows::WeatherProvinces(vec![]));
        frame
    }

    fn transport_frame(speed: i64) -> DecodedFrame {
        let mut frame = frame();
        put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 99,
                canonical_slot: 1,
            }]),
        );
        put(
            &mut frame,
            TableRows::Countries(vec![CountryRow {
                id: 101,
                canonical_slot: 1,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                forward_visibility: replay::rows::Recorded::NotRecorded,
                country: 101,
                movement: Recorded::Value(Some(CountryMovementInputs {
                    sample_hour: 20,
                    river: None,
                    transport: Some(vec![RegionTransportSpeed {
                        region: 9,
                        speed: Some(speed),
                    }]),
                    ..Default::default()
                })),
                ..Default::default()
            }]),
        );
        frame
    }

    #[test]
    fn fresh_domain_requires_the_exact_hour_even_when_legacy_age_is_valid() {
        let fresh = FreshCounterEta {
            sample_hour: 20,
            ..Default::default()
        };
        assert_eq!(current_hour(&fresh, 20), Ok(()));
        assert_eq!(
            current_hour(&fresh, 21),
            Err("fresh ETA inputs sampled at 20, frame is 21".into())
        );
        assert_eq!(
            current_hour(&fresh, 19),
            Err("fresh ETA inputs sampled at 20, frame is 19".into())
        );
    }

    #[test]
    fn transport_route_ignores_missing_normal_rail_weather_and_unreached_stored_terrain() {
        let frame = transport_frame(200_000);
        let content = content(0);
        let inputs = Inputs::new(&frame, &content);
        let fresh = FreshCounterEta {
            sample_hour: 20,
            ..Default::default()
        };
        // Stored first=3 has no content terrain. Query first=2 is transport,
        // so neither the stored invasion predicate nor progress is reached.
        assert_eq!(
            inputs.remaining(&division(), &movement(3), &fresh, &[2], |_| Ok(9)),
            Ok(200_000)
        );
    }

    #[test]
    fn refused_rail_content_does_not_deny_transport_or_normal_duration() {
        let transport = transport_frame(200_000);
        let land = normal_frame();
        for (frame, flags, fresh) in [
            (&transport, 0, FreshCounterEta::default()),
            (&land, 1, normal()),
        ] {
            let mut content = content(flags);
            content.rail = Err("fixture rail define refused".into());
            let inputs = Inputs::new(frame, &content);
            assert_eq!(
                inputs.remaining(&division(), &movement(3), &fresh, &[2], |_| Ok(9)),
                Ok(200_000)
            );
            assert_eq!(inputs.rail(1, 2), Err("fixture rail define refused".into()));
        }
    }

    #[test]
    fn fresh_progress_uses_actual_speed_only_when_first_edges_match() {
        let frame = normal_frame();
        let content = content(1);
        let inputs = Inputs::new(&frame, &content);
        let mut fresh = normal();
        let mut movement = movement(2);
        movement.numerator = 50_000;
        let no_region = |_| Err("transport was unexpectedly reached".into());
        assert_eq!(
            inputs.remaining(&division(), &movement, &fresh, &[2], no_region),
            Err("fresh ETA actual first-edge speed was refused".into())
        );
        fresh.current_speed = Some(200_000);
        assert_eq!(
            inputs.remaining(&division(), &movement, &fresh, &[2], no_region),
            Ok(175_000)
        );
        movement.path[0] = 3;
        fresh.current_speed = None;
        assert_eq!(
            inputs.remaining(&division(), &movement, &fresh, &[2], no_region),
            Ok(200_000)
        );
        assert_eq!(
            inputs.remaining(&division(), &movement, &fresh, &[], no_region),
            Ok(0)
        );
    }

    #[test]
    fn refused_strategic_gate_does_not_become_a_normal_speed_fallback() {
        let frame = normal_frame();
        let content = content(1);
        let inputs = Inputs::new(&frame, &content);
        let mut movement = movement(3);
        movement.strategic_redeployment = 1;
        let mut fresh = normal();
        assert_eq!(
            inputs.remaining(&division(), &movement, &fresh, &[2], |_| Ok(9)),
            Err("fresh ETA strategic gate was refused".into())
        );
        fresh.rail = Some(StrategicRedeploymentInputs {
            country_modifier: 1,
            ..Default::default()
        });
        assert_eq!(
            inputs.remaining(&division(), &movement, &fresh, &[2], |_| Ok(9)),
            Ok(200_000)
        );
    }

    #[test]
    fn zero_transport_speed_returns_before_missing_actual_progress_speed() {
        let frame = transport_frame(0);
        let content = content(0);
        let inputs = Inputs::new(&frame, &content);
        assert_eq!(
            inputs.remaining(
                &division(),
                &movement(2),
                &FreshCounterEta::default(),
                &[2],
                |_| Ok(9)
            ),
            Ok(0)
        );
    }

    #[test]
    fn earlier_transport_division_overflow_is_not_hidden_by_later_zero_speed() {
        let mut frame = transport_frame(-1);
        put(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                forward_visibility: replay::rows::Recorded::NotRecorded,
                country: 101,
                movement: Recorded::Value(Some(CountryMovementInputs {
                    sample_hour: 20,
                    transport: Some(vec![
                        RegionTransportSpeed {
                            region: 9,
                            speed: Some(-1),
                        },
                        RegionTransportSpeed {
                            region: 10,
                            speed: Some(0),
                        },
                    ]),
                    ..Default::default()
                })),
                ..Default::default()
            }]),
        );
        let mut content = content(0);
        let sea = ProvinceTerrain { flags: 0, index: 1 };
        content.map = MovementMap::from_parts(&[
            (
                1,
                sea,
                vec![Neighbour {
                    to: 2,
                    distance: Some(1_i64 << 58),
                    ..Default::default()
                }],
            ),
            (
                2,
                sea,
                vec![Neighbour {
                    to: 3,
                    distance: Some(100_000),
                    ..Default::default()
                }],
            ),
            (3, sea, vec![]),
        ]);
        // The first fixed5 product is i64::MIN and division by -1 traps.
        // Native has already reached that division before the next speed0.
        assert_eq!(
            Inputs::new(&frame, &content).remaining(
                &division(),
                &movement(3),
                &FreshCounterEta::default(),
                &[2, 3],
                |from| Ok(if from == 1 { 9 } else { 10 }),
            ),
            Err("ETA native signed division overflows".into()),
        );
    }

    #[test]
    fn refused_and_stale_owner_packets_cannot_supply_transport() {
        let mut frame = transport_frame(200_000);
        let content = content(0);
        for (packet, expected) in [
            (
                Recorded::NotRecorded,
                "CountryCounterInputs.movement not recorded",
            ),
            (
                Recorded::Value(None),
                "fresh ETA owner movement packet was refused",
            ),
            (
                Recorded::Value(Some(CountryMovementInputs {
                    sample_hour: 19,
                    transport: Some(vec![RegionTransportSpeed {
                        region: 9,
                        speed: Some(200_000),
                    }]),
                    ..Default::default()
                })),
                "fresh ETA owner inputs sampled at 19, frame is 20",
            ),
        ] {
            put(
                &mut frame,
                TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                    forward_visibility: replay::rows::Recorded::NotRecorded,
                    country: 101,
                    movement: packet,
                    ..Default::default()
                }]),
            );
            assert_eq!(
                Inputs::new(&frame, &content).transport(99, 9),
                Err(expected.into())
            );
        }
    }

    #[test]
    fn rail_admission_requires_zero_ticks_selected_positive_levels_and_direction() {
        let mut frame = frame();
        let content = content(1);
        put(
            &mut frame,
            TableRows::RailEdges(vec![RailEdgeRow {
                from: 1,
                to: 2,
                level: 1,
            }]),
        );
        for (ticks, level, expected) in [(0, 1, true), (-1, 1, false), (1, 1, false), (0, 0, false)]
        {
            put(
                &mut frame,
                TableRows::RailCooldowns(
                    [1, 2]
                        .map(|province| RailCooldownRow {
                            province,
                            cooldown_ticks: ticks,
                            building: Recorded::Value(Some(RailwayBuildingState {
                                building: Some(RailwayBuildingHealth {
                                    level,
                                    healthy: level,
                                    partial: None,
                                }),
                            })),
                            selection: Default::default(),
                            selection_hour: Default::default(),
                        })
                        .to_vec(),
                ),
            );
            let inputs = Inputs::new(&frame, &content);
            assert_eq!(
                crate::map::raids::reach::CounterFacts::rail_connected(&inputs, 1, 2),
                Ok(expected)
            );
            assert_eq!(
                crate::map::raids::reach::CounterFacts::rail_connected(&inputs, 2, 1),
                Ok(false)
            );
        }
    }

    #[test]
    fn rail_admission_does_not_read_nodes_for_sea_or_health_without_both_nodes() {
        let mut frame = frame();
        let sea = content(0);
        assert_eq!(
            crate::map::raids::reach::CounterFacts::rail_connected(
                &Inputs::new(&frame, &sea),
                1,
                2
            ),
            Ok(false),
        );
        put(
            &mut frame,
            TableRows::RailCooldowns(vec![RailCooldownRow {
                province: 1,
                cooldown_ticks: 0,
                building: Recorded::NotRecorded,
                selection: Default::default(),
                selection_hour: Default::default(),
            }]),
        );
        let land = content(1);
        assert_eq!(
            crate::map::raids::reach::CounterFacts::rail_connected(
                &Inputs::new(&frame, &land),
                1,
                2
            ),
            Ok(false),
        );
    }

    #[test]
    fn rail_health_absence_is_consumed_only_on_an_open_directed_edge() {
        let mut frame = frame();
        put(
            &mut frame,
            TableRows::Provinces(vec![ProvinceRow {
                id: 2,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::RailCooldowns(vec![RailCooldownRow {
                province: 1,
                cooldown_ticks: 1,
                building: Recorded::NotRecorded,
                selection: Default::default(),
                selection_hour: Default::default(),
            }]),
        );
        let content = content(1);
        assert_eq!(Inputs::new(&frame, &content).rail(1, 2), Ok(300_000));
        // Edges and health become required only after the cooldown gate opens.
        *frame.tables[TableId::RailCooldowns.index()] =
            TableView::Complete(TableRows::RailCooldowns(vec![RailCooldownRow {
                province: 1,
                cooldown_ticks: 0,
                building: Recorded::NotRecorded,
                selection: Default::default(),
                selection_hour: Default::default(),
            }]));
        put(
            &mut frame,
            TableRows::RailEdges(vec![RailEdgeRow {
                from: 1,
                to: 2,
                level: 5,
            }]),
        );
        assert_eq!(
            Inputs::new(&frame, &content).rail(1, 2),
            Err("RailCooldowns.building not recorded".into())
        );
    }

    #[test]
    fn first_endpoint_remains_available_when_the_ordinary_suffix_is_clipped() {
        let movement = DivisionMovementRow {
            path: vec![2],
            path_total: 3,
            ..Default::default()
        };
        assert_eq!(Endpoint::Next.target(&movement), Ok(2));
        assert_eq!(
            Endpoint::Destination.target(&movement),
            Err("fresh ETA ordinary route is incomplete".into())
        );
    }

    #[test]
    fn absent_weather_record_does_not_require_the_snow_ceiling() {
        let mut frame = normal_frame();
        let content = content(1);
        let fresh = normal();
        assert_eq!(
            Inputs::new(&frame, &content).remaining(
                &division(),
                &movement(3),
                &fresh,
                &[2],
                |_| Ok(9)
            ),
            Ok(200_000)
        );
        put(
            &mut frame,
            TableRows::WeatherProvinces(vec![WeatherProvinceRow {
                id: 2,
                snow: 1,
                ..Default::default()
            }]),
        );
        assert!(
            Inputs::new(&frame, &content)
                .remaining(&division(), &movement(3), &fresh, &[2], |_| Ok(9))
                .unwrap_err()
                .contains("WeatherRegions")
        );
    }

    #[test]
    fn observed_null_rail_building_is_zero_but_missing_selection_is_refused() {
        let mut frame = normal_frame();
        put(
            &mut frame,
            TableRows::RailEdges(vec![RailEdgeRow {
                from: 1,
                to: 2,
                level: 5,
            }]),
        );
        let mut content = content(1);
        content.rail.as_mut().unwrap().speed = [0, 0, 0, 1_000_000];
        for (building, expected) in [
            (Some(RailwayBuildingState { building: None }), Ok(0)),
            (
                None,
                Err("fresh railway building health unavailable".into()),
            ),
        ] {
            put(
                &mut frame,
                TableRows::RailCooldowns(vec![RailCooldownRow {
                    province: 1,
                    cooldown_ticks: 0,
                    building: Recorded::Value(building),
                    selection: Default::default(),
                    selection_hour: Default::default(),
                }]),
            );
            assert_eq!(Inputs::new(&frame, &content).rail(1, 2), expected);
        }
    }

    #[test]
    fn refused_owner_entries_are_independent_but_duplicate_domain_keys_refuse() {
        let mut frame = transport_frame(200_000);
        put(
            &mut frame,
            TableRows::Provinces(vec![ProvinceRow {
                id: 2,
                controller: 0,
                ..Default::default()
            }]),
        );
        let mut domain = CountryMovementInputs {
            sample_hour: 20,
            river: Some(vec![
                ControllerRiverSpeed {
                    target_slot: 0,
                    modifier: Some(-10_000),
                },
                ControllerRiverSpeed {
                    target_slot: 1,
                    modifier: None,
                },
            ]),
            transport: Some(vec![
                RegionTransportSpeed {
                    region: 9,
                    speed: Some(200_000),
                },
                RegionTransportSpeed {
                    region: 10,
                    speed: None,
                },
            ]),
            ..Default::default()
        };
        let content = content(0);
        for duplicate in [false, true] {
            if duplicate {
                domain.river.as_mut().unwrap().push(ControllerRiverSpeed {
                    target_slot: 1,
                    modifier: Some(0),
                });
                domain
                    .transport
                    .as_mut()
                    .unwrap()
                    .push(RegionTransportSpeed {
                        region: 10,
                        speed: Some(0),
                    });
            }
            put(
                &mut frame,
                TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                    forward_visibility: replay::rows::Recorded::NotRecorded,
                    country: 101,
                    movement: Recorded::Value(Some(domain.clone())),
                    ..Default::default()
                }]),
            );
            let inputs = Inputs::new(&frame, &content);
            assert_eq!(
                inputs.transport(99, 9),
                if duplicate {
                    Err("fresh ETA owner transport contains duplicate keys".into())
                } else {
                    Ok(200_000)
                }
            );
            assert_eq!(
                inputs.river(99, 2),
                if duplicate {
                    Err("fresh ETA owner river contains duplicate keys".into())
                } else {
                    Ok(-10_000)
                }
            );
        }
        assert_eq!(
            owner_entry(
                &[
                    RegionTransportSpeed {
                        region: 10,
                        speed: Some(0)
                    },
                    RegionTransportSpeed {
                        region: 9,
                        speed: Some(200_000)
                    },
                ],
                |entry| entry.region,
                9,
                "owner transport",
            ),
            Err("fresh ETA owner transport keys are not increasing".into())
        );
    }

    #[test]
    fn normal_terrain_rejects_duplicate_and_nonpositive_keys_only_when_reached() {
        let frame = normal_frame();
        let land = content(1);
        let inputs = Inputs::new(&frame, &land);
        for terrain in [-1, 0, 2] {
            let mut fresh = normal();
            fresh
                .normal
                .as_mut()
                .unwrap()
                .terrain_speed
                .push(TerrainSpeed {
                    terrain,
                    modifier: 0,
                });
            assert_eq!(
                inputs.remaining(&division(), &movement(3), &fresh, &[2], |_| Ok(9)),
                Err(if terrain <= 0 {
                    "fresh ETA normal terrain contains a nonpositive key"
                } else {
                    "fresh ETA normal terrain contains duplicate keys"
                }
                .into())
            );
            // The same invalid normal domain cannot deny a transport route.
            let transport = transport_frame(200_000);
            let sea = content(0);
            assert_eq!(
                Inputs::new(&transport, &sea).remaining(
                    &division(),
                    &movement(3),
                    &fresh,
                    &[2],
                    |_| Ok(9)
                ),
                Ok(200_000)
            );
        }
        let mut fresh = normal();
        fresh.normal.as_mut().unwrap().terrain_speed.insert(
            0,
            TerrainSpeed {
                terrain: 3,
                modifier: 0,
            },
        );
        assert_eq!(
            inputs.remaining(&division(), &movement(3), &fresh, &[2], |_| Ok(9)),
            Ok(200_000)
        );
    }

    #[test]
    fn row_indexes_refuse_duplicate_aliases_slots_and_directed_edges() {
        let content = content(0);
        let mut frame = transport_frame(200_000);
        put(
            &mut frame,
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 99,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 99,
                    canonical_slot: 2,
                },
            ]),
        );
        assert_eq!(
            Inputs::new(&frame, &content).transport(99, 9),
            Err("fresh ETA CountryLinks contains duplicate keys".into())
        );

        let mut frame = transport_frame(200_000);
        put(
            &mut frame,
            TableRows::Countries(vec![
                CountryRow {
                    id: 101,
                    canonical_slot: 1,
                    ..Default::default()
                },
                CountryRow {
                    id: 102,
                    canonical_slot: 1,
                    ..Default::default()
                },
            ]),
        );
        assert_eq!(
            Inputs::new(&frame, &content).transport(99, 9),
            Err("fresh ETA Countries contains duplicate keys".into())
        );

        put(
            &mut frame,
            TableRows::Countries(vec![
                CountryRow {
                    id: 101,
                    canonical_slot: 1,
                    ..Default::default()
                },
                CountryRow {
                    id: 101,
                    canonical_slot: 2,
                    ..Default::default()
                },
            ]),
        );
        assert_eq!(
            Inputs::new(&frame, &content).countries,
            Err("fresh ETA Countries contains duplicate keys".into())
        );

        let mut frame = normal_frame();
        put(
            &mut frame,
            TableRows::RailCooldowns(vec![RailCooldownRow {
                province: 1,
                cooldown_ticks: 0,
                building: Recorded::NotRecorded,
                selection: Default::default(),
                selection_hour: Default::default(),
            }]),
        );
        put(
            &mut frame,
            TableRows::RailEdges(vec![
                RailEdgeRow {
                    from: 1,
                    to: 2,
                    level: 1,
                },
                RailEdgeRow {
                    from: 1,
                    to: 2,
                    level: 5,
                },
            ]),
        );
        assert_eq!(
            Inputs::new(&frame, &content).rail(1, 2),
            Err("fresh ETA RailEdges contains duplicate keys".into())
        );
    }

    #[test]
    fn route_identity_gate_refuses_incomplete_and_duplicate_core_domains_only() {
        let content = content(1);
        let mut frame = transport_frame(200_000);
        put(&mut frame, TableRows::Provinces(vec![]));
        // States are unavailable: reached edge predicates own that domain.
        assert_eq!(Inputs::new(&frame, &content).route_identities(), Ok(()));
        for rows in [
            TableRows::Provinces(vec![
                ProvinceRow {
                    id: 2,
                    ..Default::default()
                };
                2
            ]),
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 99,
                    canonical_slot: 1
                };
                2
            ]),
            TableRows::Countries(vec![
                CountryRow {
                    id: 101,
                    canonical_slot: 1,
                    ..Default::default()
                };
                2
            ]),
        ] {
            let table = rows.table();
            let original = frame.tables[table.index()].clone();
            put(&mut frame, rows);
            assert_eq!(
                Inputs::new(&frame, &content).route_identities(),
                Err(format!("fresh ETA {table:?} contains duplicate keys"))
            );
            frame.tables[table.index()] = TableView::Unavailable {
                reason: "fixture core gap".into(),
            }
            .into();
            assert!(
                Inputs::new(&frame, &content)
                    .route_identities()
                    .unwrap_err()
                    .contains("fixture core gap")
            );
            frame.tables[table.index()] = original;
        }
    }
}
