use viewer_counter_kit::flotilla::{BARS_UNRECORDED, Flotilla, ItemIntel, NavalBars, NavalCells};
use viewer_native_gui::native_windows::NAVAL_BASE_WINDOW;
use crate::base_windows;

use content_reader_map::map::MapData;
use crate::{air_grid, naval_members, naval_regional, naval_windows};
use crate::bases::NavalBaseIcon;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::io;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use viewer_counter_kit::{
    CounterDefines, CounterFrame, CounterInputs, CounterMap, CounterReport, Countries, JoinRules,
    gui_windows, type_tint,
};
use viewer_counter_kit::counter_groups::Level;
use viewer_foundation::allegiance::Allegiance;
use viewer_foundation::mode_visibility::{PriorityTable, UnitKinds, Visibility};
use viewer_foundation::movement_timing::MovementContent;
use viewer_foundation::perspective::{Sight, Vision};
use viewer_foundation::tables::rows;
use viewer_gpu::province_admission::Admission;
use viewer_unit_reads::{counter_inputs, counter_move, counter_order};
use viewer_unit_reads::counter_inputs::NativeInputs;
#[cfg(test)]
use content_reader_gui::gui::native;
use viewer_native_gui::gui_resources::viewport;

#[derive(Clone)]
pub struct ShipTables {
    models: replay_frame::frame::SharedView,
    rosters: BTreeMap<u64, Vec<(usize, i64)>>,
    ratios: BTreeMap<u64, (i64, i64)>,
    base_kinds: BTreeMap<u64, i32>,
    viewer_wings: Option<BTreeMap<u64, bool>>,
}

pub struct ShipInputs<'a> {
    pub equipment: Result<&'a content_reader_key_names::equipment::Database, &'a str>,
    pub tables: Arc<ShipTables>,
}

impl ShipTables {
    fn read(
        ships: &[replay_data::rows::ShipRow],
        models: &[replay_data::rows::ShipModelRow],
        frame: &DecodedFrame,
        vision: &Vision,
    ) -> io::Result<Option<Self>> {
        if models.iter().any(|model| model.counter.is_none()) {
            return Ok(None);
        }
        let by_ship: BTreeMap<_, _> = models
            .iter()
            .enumerate()
            .map(|(index, model)| (model.ship, (index, model)))
            .collect();
        let mut rosters = BTreeMap::<u64, Vec<(u32, usize, i64)>>::new();
        let mut operands = BTreeMap::<u64, Vec<[i64; 4]>>::new();
        for ship in ships {
            let Some(&(index, model)) = by_ship.get(&ship.id) else {
                return Ok(None);
            };
            let Some(counter) = &model.counter else {
                return Ok(None);
            };
            rosters.entry(ship.task_force).or_default().push((
                counter.roster_ordinal,
                index,
                ship.strength,
            ));
            operands.entry(ship.task_force).or_default().push([
                ship.strength,
                counter.maximum_strength,
                ship.organisation,
                counter.maximum_organisation,
            ]);
        }
        let ratios = operands
            .into_iter()
            .map(|(force, ships)| {
                (
                    force,
                    (
                        counter_inputs::strength_ratio(ships.iter().map(|s| (s[0], s[1]))),
                        counter_inputs::organisation_ratio(ships.iter().map(|s| (s[2], s[3]))),
                    ),
                )
            })
            .collect();
        let rosters = rosters
            .into_iter()
            .map(|(force, mut ships)| {
                ships.sort_by_key(|(ordinal, _, _)| *ordinal);
                (
                    force,
                    ships
                        .into_iter()
                        .map(|(_, index, strength)| (index, strength))
                        .collect(),
                )
            })
            .collect();
        let bases = rows::<tables::AirBases>(frame)?;
        let registrations = rows::<tables::AirBaseRegistrations>(frame)?;
        let viewer_wings = (bases.is_complete() && registrations.is_complete()).then(|| {
            registrations
                .rows()
                .iter()
                .filter(|row| row.slot == vision.ours())
                .map(|row| (row.base, !row.wings.is_empty()))
                .collect()
        });
        Ok(Some(Self {
            models: frame.tables[replay_data::rows::TableId::ShipModels.index()].clone(),
            rosters,
            ratios,
            base_kinds: bases.rows().iter().map(|row| (row.id, row.kind)).collect(),
            viewer_wings,
        }))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_rosters(rosters: BTreeMap<u64, Vec<counter_order::Hull<'_>>>) -> Arc<Self> {
        let mut models = Vec::new();
        let rosters = rosters
            .into_iter()
            .map(|(force, hulls)| {
                (
                    force,
                    hulls
                        .into_iter()
                        .map(|hull| {
                            let index = models.len();
                            models.push(hull.model.clone());
                            (index, hull.strength)
                        })
                        .collect(),
                )
            })
            .collect();
        Arc::new(Self {
            models: replay_frame::frame::TableView::Complete(
                replay_data::rows::TableRows::ShipModels(models),
            )
            .into(),
            rosters,
            ratios: BTreeMap::new(),
            base_kinds: BTreeMap::new(),
            viewer_wings: None,
        })
    }
}

impl ShipInputs<'_> {
    pub fn hulls(&self, force: u64) -> impl Iterator<Item = counter_order::Hull<'_>> {
        let replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::ShipModels(
            models,
        )) = &*self.tables.models
        else {
            unreachable!("prepared ship models are complete")
        };
        self.tables
            .rosters
            .get(&force)
            .into_iter()
            .flatten()
            .map(|&(index, strength)| counter_order::Hull {
                strength,
                model: &models[index],
            })
    }

    pub fn carries_air_wings(&self, force: u64) -> Option<bool> {
        let ships: Vec<_> = self.hulls(force).map(|hull| hull.model).collect();
        counter_order::first_carrier_has_air_base(&ships)
    }

    pub fn ratio(&self, force: u64) -> (i64, i64) {
        self.tables.ratios.get(&force).copied().unwrap_or((0, 0))
    }

    pub fn cells(&self, members: &[u64], port: bool) -> NavalCells {
        let roster = |force: &u64| self.hulls(*force);
        let capitals = members
            .iter()
            .flat_map(roster)
            .map(|hull| counter_order::is_capital(hull.model))
            .try_fold(0usize, |sum, capital| Some(sum + usize::from(capital?)));
        let hull = counter_order::hull_icon(members.iter().flat_map(roster), self.equipment);
        let base = |air: u64| -> Option<counter_order::CarrierBase> {
            let wings = self.tables.viewer_wings.as_ref()?;
            let kind = *self.tables.base_kinds.get(&air)?;
            Some(counter_order::CarrierBase {
                access: wings.contains_key(&air),
                kind,
                stationed: wings.get(&air).copied().filter(|stationed| *stationed),
            })
        };
        let per_force: Vec<BTreeSet<u32>> = members
            .iter()
            .map(|force| {
                let ships: Vec<&replay_data::rows::ShipModelRow> =
                    roster(force).map(|hull| hull.model).collect();
                counter_order::task_force_carrier(&ships, &base)
            })
            .collect();
        let carrier = if port {
            counter_order::port_carrier(&per_force)
        } else {
            counter_order::counter_carrier(&per_force)
                .into_iter()
                .map(counter_order::carrier_icon)
                .collect()
        };
        NavalCells {
            capitals,
            hull,
            carrier,
        }
    }
}

pub struct NavalUnit<'a> {
    pub force: &'a replay_data::rows::TaskForceRow,
    pub ordinal: u32,
    pub slot: u32,
    pub logical: Option<u32>,
    pub flag_owner: u32,
    pub sight: Sight,
    pub byte: u16,
    pub ships: Option<usize>,
    pub position: Option<counter_move::Position>,
    pub strength: i64,
}

pub struct NavalContext<'a> {
    pub type_tints: &'a type_tint::Index,
    pub vision: &'a Vision,
    pub native: &'a NativeInputs,
    pub ship_inputs: Option<ShipInputs<'a>>,
    pub aggression: &'a [i64],
    pub ship_task_forces: Arc<BTreeMap<u64, u64>>,
    pub ships_complete: bool,
    pub ships: Arc<BTreeMap<u64, (usize, i64)>>,
    pub country_ids: &'a BTreeMap<u32, u32>,
    pub ideology: &'a Ideology<'a>,
    pub missions: Arc<BTreeMap<u64, u32>>,
    pub forces: &'a [replay_data::rows::TaskForceRow],
    pub mission_skips: Arc<viewer_unit_reads::task_force_visual::MissionSkips>,
    pub forces_complete: bool,
}

#[derive(Clone, Copy)]
pub struct NavalSources<'a> {
    pub type_tints: &'a type_tint::Index,
    pub equipment: Result<&'a content_reader_key_names::equipment::Database, &'a str>,
    pub vision: &'a Vision,
    pub native: &'a NativeInputs,
    pub country_ids: &'a BTreeMap<u32, u32>,
    pub aggression: &'a [i64],
    pub ideology: &'a Ideology<'a>,
}

pub type Ideology<'a> = dyn Fn(u32) -> Result<Option<i32>, String> + Sync + 'a;

fn set_frame_argument(number: u32) -> i32 {
    i32::from_ne_bytes(number.to_ne_bytes())
}

pub fn ideology_frame(
    frame: &DecodedFrame,
    ideologies: &content_reader_key_names::ideologies::Ideologies,
    country_ids: &BTreeMap<u32, u32>,
    slot: u32,
) -> Result<Option<i32>, String> {
    let hour = frame.hour;
    if !dashboard_text::identity::civil_war(frame, slot)? {
        return Ok(None);
    }
    let raw = *country_ids
        .get(&slot)
        .ok_or_else(|| format!("country slot {slot} has no Countries row at hour {hour}"))?;
    let politics = frame
        .complete::<tables::Politics>()
        .map_err(|error| error.to_string())?;
    let ruling = politics
        .iter()
        .find(|row| row.country == raw)
        .ok_or_else(|| format!("country {raw} has no Politics row at hour {hour}"))?
        .ruling_party
        .ok_or_else(|| format!("country {raw} has no ruling party at hour {hour}"))?;
    let parties = frame
        .complete::<tables::PoliticalParties>()
        .map_err(|error| error.to_string())?;
    let party = parties
        .iter()
        .find(|row| row.country == raw && row.id == ruling)
        .ok_or_else(|| {
            format!(
                "country {raw}'s ruling party {ruling} has no PoliticalParties row at hour {hour}"
            )
        })?;
    let token = String::from_utf8_lossy(&party.token);
    match ideologies.frame(&token)? {
        Some(number) => Ok(Some(set_frame_argument(number))),
        None => Err(format!(
            "ideology group {token} (country {raw}'s ruling party) is defined by no ideologies file"
        )),
    }
}

#[derive(Clone)]
pub struct NavalTables {
    ship_inputs: Option<Arc<ShipTables>>,
    ship_task_forces: Arc<BTreeMap<u64, u64>>,
    ships_complete: bool,
    ships: Arc<BTreeMap<u64, (usize, i64)>>,
    missions: Arc<BTreeMap<u64, u32>>,
    mission_skips: Arc<viewer_unit_reads::task_force_visual::MissionSkips>,
    forces_complete: bool,
}

impl NavalTables {
    pub fn read(frame: &DecodedFrame, vision: &Vision) -> io::Result<Self> {
        let ship_rows = rows::<tables::Ships>(frame)?;
        let mut ships = BTreeMap::<u64, (usize, i64)>::new();
        for ship in ship_rows.rows() {
            let entry = ships.entry(ship.task_force).or_default();
            entry.0 += 1;
            entry.1 = entry.1.wrapping_add(ship.strength);
        }
        let models = rows::<tables::ShipModels>(frame)?;
        let ship_inputs = if ship_rows.is_complete() && models.is_complete() {
            ShipTables::read(ship_rows.rows(), models.rows(), frame, vision)?.map(Arc::new)
        } else {
            None
        };
        let forces = rows::<tables::TaskForces>(frame)?;
        Ok(Self {
            ship_inputs,
            ship_task_forces: Arc::new(
                ship_rows
                    .rows()
                    .iter()
                    .map(|ship| (ship.id, ship.task_force))
                    .collect(),
            ),
            ships_complete: ship_rows.is_complete(),
            ships: Arc::new(ships),
            missions: Arc::new(
                forces
                    .rows()
                    .iter()
                    .map(|force| (force.id, force.mission))
                    .collect(),
            ),
            mission_skips: Arc::new(
                viewer_unit_reads::task_force_visual::MissionSkips::read(frame)?,
            ),
            forces_complete: forces.is_complete(),
        })
    }
}

impl<'a> NavalContext<'a> {
    #[cfg(test)]
    pub fn read(
        shown: bool,
        frame: &'a DecodedFrame,
        sources: NavalSources<'a>,
    ) -> io::Result<Option<Self>> {
        if shown {
            Self::build(frame, sources).map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn build(frame: &'a DecodedFrame, sources: NavalSources<'a>) -> io::Result<Self> {
        Self::cached(frame, sources, &NavalTables::read(frame, sources.vision)?)
    }

    pub fn cached(
        frame: &'a DecodedFrame,
        sources: NavalSources<'a>,
        tables: &NavalTables,
    ) -> io::Result<Self> {
        Ok(Self {
            type_tints: sources.type_tints,
            vision: sources.vision,
            native: sources.native,
            ship_inputs: tables.ship_inputs.as_ref().map(|tables| ShipInputs {
                equipment: sources.equipment,
                tables: Arc::clone(tables),
            }),
            aggression: sources.aggression,
            ship_task_forces: Arc::clone(&tables.ship_task_forces),
            ships_complete: tables.ships_complete,
            ships: Arc::clone(&tables.ships),
            country_ids: sources.country_ids,
            ideology: sources.ideology,
            missions: Arc::clone(&tables.missions),
            forces: rows::<tables::TaskForces>(frame)?.rows(),
            mission_skips: Arc::clone(&tables.mission_skips),
            forces_complete: tables.forces_complete,
        })
    }

    pub fn unit(
        &self,
        force: &'a replay_data::rows::TaskForceRow,
        owner: &impl Fn(u32) -> Option<u32>,
        ordinal: u32,
        position: Option<counter_move::Position>,
    ) -> Option<NavalUnit<'a>> {
        let operands = naval_members::NavalOperands::of(
            &naval_members::ForceFields::of(force),
            self.ships.get(&force.id).copied(),
            self.ships_complete,
            self.vision,
            owner,
        )?;
        Some(NavalUnit {
            force,
            ordinal,
            slot: operands.slot,
            logical: operands.logical,
            flag_owner: operands.flag_owner,
            sight: operands.sight,
            byte: operands.byte,
            ships: operands.ships,
            position,
            strength: operands.strength,
        })
    }

    pub fn reserve(&self, fleet: u64) -> Option<bool> {
        if fleet == 0 {
            return Some(false);
        }
        self.native.fleet(fleet).and_then(|row| {
            counter_inputs::reserve_fleet(row, |force| self.missions.get(&force).copied())
        })
    }

    fn fleet_color(&self, fleet: u64) -> Result<[f32; 4], String> {
        let row = self.native.fleet(fleet).ok_or_else(|| {
            format!(
                "group_marker is undecided: fleet {fleet}'s colour (FleetCounterInputs) is not recorded"
            )
        })?;
        counter_inputs::color(row.red, row.green, row.blue, row.alpha).ok_or_else(|| {
            format!(
                "group_marker is refused: fleet {fleet}'s colour (FleetCounterInputs) has non-finite components"
            )
        })
    }

    fn member<'b>(&'b self, unit: &NavalUnit<'_>) -> counter_inputs::NavalMember<'b> {
        let force = unit.force;
        counter_inputs::NavalMember {
            visible: unit.sight == Sight::Full,
            own: unit.slot == self.vision.ours(),
            mission: force.mission,
            has_fleet: force.fleet != 0,
            inputs: self.native.task_force(force.id),
            fleet_regions: self
                .native
                .fleet(force.fleet)
                .map(|fleet| fleet.mission_region_count),
            pride: counter_inputs::pride(
                force.id,
                self.country_ids
                    .get(&unit.slot)
                    .and_then(|id| self.native.pride_ship(*id)),
                |ship| self.ship_task_forces.get(&ship).copied(),
                self.ships_complete,
            ),
        }
    }

    pub fn flotilla(&self, units: &[&NavalUnit<'_>], port: bool, coarse: bool) -> Flotilla {
        let first = units[0];
        let last = units[units.len() - 1];
        let members: Arc<[u64]> = units.iter().map(|unit| unit.force.id).collect();
        let native_members: Vec<counter_inputs::NavalMember<'_>> =
            units.iter().map(|unit| self.member(unit)).collect();
        let ratio = |unit: &&NavalUnit<'_>| {
            self.ship_inputs
                .as_ref()
                .map(|inputs| inputs.ratio(unit.force.id))
        };
        let own = first.slot == self.vision.ours();
        let type_tint = self
            .type_tints
            .of(first.force.roster_owner, first.force.logical_owner);
        let bars = if port {
            if first.sight != Sight::Full && !own {
                NavalBars::Hidden
            } else {
                mean_bars(units.iter().map(ratio))
            }
        } else {
            mean_bars(
                units
                    .iter()
                    .filter(|unit| unit.sight == Sight::Full)
                    .map(ratio),
            )
        };
        let fleet = first.force.fleet;
        let shown = if port {
            Some(own && fleet == last.force.fleet && fleet != 0)
        } else if own && fleet != 0 {
            self.reserve(fleet).map(|reserve| !reserve)
        } else {
            Some(false)
        };
        let marker = match shown {
            None => Err(format!(
                "group_marker is undecided: fleet {fleet}'s reserve test (FleetCounterInputs, its \
                 task forces' missions) is not recorded"
            )),
            Some(false) => Ok(None),
            Some(true) => self.fleet_color(fleet).map(Some),
        };
        let side = if port {
            self.type_tints
                .mission_item(own, first.force.roster_owner, first.force.logical_owner)
                .ok_or_else(|| {
                    format!(
                        "a naval mission item's allegiance is unknown: {}",
                        self.type_tints.problem()
                    )
                })
        } else {
            Ok(type_tint::plate(own, type_tint))
        };
        let port_tooltip = port.then(|| {
            viewer_native_gui::counter_hovers::task_force_port::Metadata::from_members(
                &native_members,
                self.aggression,
                if (first.force.expeditionary_owner as i32) > 0 {
                    first.force.expeditionary_owner
                } else {
                    first.force.roster_owner
                },
                first.byte,
                side.as_ref()
                    .map(|side| *side == Allegiance::Enemy)
                    .map_err(Clone::clone),
            )
        });
        Flotilla {
            members: Arc::clone(&members),
            source: first.force.id,
            owner: first.slot,
            logical: first.logical,
            type_tint,
            side,
            flag_owner: if port {
                port_flag_owner(units, self.ships_complete)
            } else if coarse {
                Some(first.slot)
            } else {
                Some(first.flag_owner)
            },
            port,
            coarse: coarse && !port,
            ships: units
                .iter()
                .filter(|unit| unit.sight == Sight::Full)
                .map(|unit| unit.ships)
                .sum::<Option<usize>>(),
            hidden: units.iter().any(|unit| unit.sight == Sight::Partial),
            visible_member: units.iter().any(|unit| unit.sight == Sight::Full),
            cells: self
                .ship_inputs
                .as_ref()
                .map(|inputs| inputs.cells(&members, port)),
            bars,
            supply: if port {
                counter_inputs::port_supply(&native_members)
            } else {
                counter_inputs::sea_supply(&native_members)
            },
            status: if port {
                counter_inputs::port_status(&native_members, self.aggression)
            } else {
                counter_inputs::at_sea_status(&native_members, self.aggression)
            },
            mission: (port && own && first.force.mission == last.force.mission)
                .then_some(first.force.mission),
            marker,
            moving: first
                .position
                .as_ref()
                .is_some_and(|position| position.motion.is_some()),
            ideology: match (port, first.logical) {
                (true, _) => Ok(None),
                (false, Some(slot)) => (self.ideology)(slot),
                (false, None) => {
                    Err("the first task force's logical owner is not a linked country".to_owned())
                }
            },
            item: port.then(|| ItemIntel {
                byte: first.byte,
                first: first.force.id,
                ships: units.iter().map(|unit| unit.ships).sum::<Option<usize>>(),
            }),
            port_tooltip,
        }
    }
}

fn port_flag_owner(units: &[&NavalUnit<'_>], ships_complete: bool) -> Option<u32> {
    let first = units[0];
    if units.iter().all(|unit| unit.slot == first.slot) {
        return Some(first.flag_owner);
    }
    if !ships_complete {
        return None;
    }
    let mut totals: BTreeMap<u32, (i32, i64)> = BTreeMap::new();
    for unit in units {
        let (_, total) = totals
            .entry(unit.slot)
            .or_insert((unit.force.roster_owner as i32, 0));
        *total = total.wrapping_add(unit.strength);
    }
    totals
        .into_iter()
        .min_by_key(|&(_, (reference, total))| (total, reference))
        .map(|(slot, _)| slot)
}

pub fn mean_bars(ratios: impl Iterator<Item = Option<(i64, i64)>>) -> NavalBars {
    let Some(ratios) = ratios.collect::<Option<Vec<_>>>() else {
        return NavalBars::Unknown(BARS_UNRECORDED.to_owned());
    };
    let strength = counter_inputs::member_mean(ratios.iter().map(|pair| pair.0));
    let organisation = counter_inputs::member_mean(ratios.iter().map(|pair| pair.1));
    match (organisation, strength) {
        (Some(organisation), Some(strength)) => NavalBars::Fill(
            counter_inputs::bar_fill(organisation),
            counter_inputs::bar_fill(strength),
        ),
        _ => NavalBars::Hidden,
    }
}


pub fn naval_can_join(
    rep: &NavalUnit<'_>,
    candidate: &NavalUnit<'_>,
    rules: JoinRules,
    countries: &Countries,
    context: &NavalContext<'_>,
) -> bool {
    if !rules.owners_join(countries, rep.slot, candidate.slot) {
        return false;
    }
    if rules.coarse || rep.slot != context.vision.ours() {
        return true;
    }
    let (a, b) = (rep.force.fleet, candidate.force.fleet);
    if a == b {
        return true;
    }
    a != 0 && b != 0 && context.reserve(a) == Some(true) && context.reserve(b) == Some(true)
}

pub const CARRIER_TEST_UNRECORDED: &str = "the air map mode keeps only fleets carrying air wings, and this hour's ship models cannot tell";

pub fn mode_keeps_task_force(
    kinds: UnitKinds,
    carries: impl FnOnce() -> Option<bool>,
) -> Option<bool> {
    match kinds {
        UnitKinds::All => Some(true),
        UnitKinds::Nothing => Some(false),
        UnitKinds::CarrierFleets => carries(),
    }
}

pub fn add_task_forces<'a>(
    inputs: &CounterInputs<'_>,
    owner: &impl Fn(u32) -> Option<u32>,
    tables: &NavalLinks<'a>,
    context: &NavalContext<'a>,
    admission: &Admission,
    mut add: impl FnMut((u32, u32), NavalUnit<'a>),
    report: &mut CounterReport,
) {
    let ground = naval_members::SeaGround {
        content: inputs.content,
        map: inputs.map,
        movement: inputs.movement,
        world: inputs.route_world(),
    };
    let bases = BTreeSet::new();
    for force in context.forces {
        if !admission.admits_province(&inputs.content.bounds, force.province) {
            continue;
        }
        let joins = naval_members::HourJoins {
            land: tables,
            context,
            owner,
            ground,
            row: force,
            bases: &bases,
        };
        let (sea, placement) = naval_members::sea_place(
            &naval_members::ForceFields::of(force),
            inputs.visibility.unit_kinds,
            &joins,
        );
        if let Some(reason) = placement.as_ref().and_then(|placement| placement.unplaced_route) {
            report
                .unplaced_task_force_routes
                .entry(reason)
                .or_default()
                .insert(force.id);
        }
        match sea {
            naval_members::Sea::Refused(reason) => report.refuse_task_forces(reason, [force.id]),
            naval_members::Sea::Out(_) => {}
            naval_members::Sea::Source { kind, ordinal, .. } => {
                let position = placement.map(|placement| placement.position);
                if let Some(unit) = context.unit(force, owner, ordinal, position) {
                    add((force.province, kind), unit);
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortGrouping {
    cleared: bool,
    coarse: bool,
    allegiance: bool,
}

pub fn port_grouping(
    defines: &CounterDefines,
    visibility: Visibility,
    height: f32,
) -> PortGrouping {
    let political = visibility.political_mode;
    PortGrouping {
        cleared: political && defines.coarse(height, false),
        coarse: political || defines.coarse(height, visibility.strategic_grouping),
        allegiance: political || defines.groups.level(height) == Level::Strategic,
    }
}

fn port_boundary(
    grouping: PortGrouping,
    first: naval_regional::SortKey,
    next: naval_regional::SortKey,
    first_owner: (u32, Option<Allegiance>),
    next_owner: (u32, Option<Allegiance>),
) -> bool {
    if grouping.allegiance {
        naval_regional::allegiance_break(first_owner, next_owner, first.selected, next.selected)
    } else {
        naval_regional::compare(first, next, grouping.coarse) < 0
    }
}

fn port_runs(
    grouping: PortGrouping,
    keys: &[naval_regional::SortKey],
    owner: impl Fn(usize) -> (u32, Option<Allegiance>),
) -> Vec<std::ops::Range<usize>> {
    let mut runs = Vec::new();
    let mut start = 0;
    for next in 1..=keys.len() {
        if next == keys.len()
            || port_boundary(grouping, keys[start], keys[next], owner(start), owner(next))
        {
            runs.push(start..next);
            start = next;
        }
    }
    runs
}

#[cfg(test)]
type PortSortKey = naval_regional::SortKey;

#[cfg(test)]
fn port_sort_compare(a: &PortSortKey, b: &PortSortKey) -> i32 {
    naval_regional::compare(*a, *b, true)
}

#[cfg(test)]
fn sort_port_units(
    units: &mut [NavalUnit<'_>],
    context: &NavalContext<'_>,
) -> Result<(), &'static str> {
    naval_regional::sort_own(units, &BTreeSet::new(), context, true)
        .map(|_| ())
        .map_err(|_| "port sort inputs unavailable")
}

#[derive(Clone, Default)]
pub struct PortTables {
    anchors: Vec<(u32, [f32; 3])>,
    unanchored: usize,
    rosters: BTreeMap<u32, Vec<(u64, u32)>>,
    undecided: BTreeMap<u32, Cow<'static, str>>,
    refused: BTreeMap<&'static str, BTreeSet<u64>>,
}

impl PortTables {
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_rosters(rosters: BTreeMap<u32, Vec<(u64, u32)>>) -> Self {
        Self { rosters, ..Self::default() }
    }

    pub fn summary(&self) -> naval_members::PortSummary {
        naval_members::PortSummary {
            anchors: self.anchors.iter().map(|&(province, anchor)| (province, anchor.map(f32::to_bits))).collect(),
            unanchored: self.unanchored,
            undecided: self.undecided.iter().map(|(&province, why)| (province, why.clone().into_owned())).collect(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn read(
        frame: &DecodedFrame,
        content: &CounterMap,
        map: &MapData,
        movement: Option<&MovementContent>,
        tables: &NavalLinks<'_>,
        context: &NavalContext<'_>,
        owner: &impl Fn(u32) -> Option<u32>,
    ) -> io::Result<Self> {
        let mut out = Self::default();
        let bases = rows::<tables::NavalBases>(frame)?;
        for base in bases.rows() {
            match content.port(base.province) {
                Some(anchor) => out.anchors.push((base.province, anchor)),
                None => out.unanchored += 1,
            }
        }
        let Some(order) = tables.order else {
            return Ok(out);
        };
        let base_provinces: BTreeSet<_> = bases.rows().iter().map(|base| base.province).collect();
        let ground = naval_members::SeaGround {
            content,
            map,
            movement,
            world: None,
        };
        for force in context.forces {
            let joins = naval_members::HourJoins {
                land: tables,
                context,
                owner,
                ground,
                row: force,
                bases: &base_provinces,
            };
            match naval_members::port_place(&naval_members::ForceFields::of(force), &joins) {
                naval_members::Port::Out => {}
                naval_members::Port::Undecided(reason) => {
                    out.refused.entry(reason).or_default().insert(force.id);
                    out.undecided.insert(force.province, port_undecided(reason));
                }
                naval_members::Port::In(ordinal) => {
                    out.rosters
                        .entry(force.province)
                        .or_default()
                        .push((force.id, ordinal));
                }
            }
        }
        for (province, absent) in unrecorded_port_forces(&base_provinces, order, context) {
            out.refused
                .entry(PORT_FORCE_UNRECORDED)
                .or_default()
                .extend(absent);
            out.undecided
                .insert(province, port_undecided(PORT_FORCE_UNRECORDED));
        }
        for roster in out.rosters.values_mut() {
            roster.sort_by_key(|(_, ordinal)| *ordinal);
        }
        Ok(out)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn port_counters(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    owner: &impl Fn(u32) -> Option<u32>,
    tables: &NavalLinks<'_>,
    context: Option<&NavalContext<'_>>,
    icons: &[NavalBaseIcon],
    cached: &PortTables,
    out: &mut CounterFrame<impl Sized, base_windows::PreparedBases, impl Sized, naval_windows::Plan>,
) -> io::Result<()> {
    port_counters_with(
        inputs, frame, owner, tables, context, icons, cached, None, out,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn port_counters_with(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    owner: &impl Fn(u32) -> Option<u32>,
    tables: &NavalLinks<'_>,
    context: Option<&NavalContext<'_>>,
    icons: &[NavalBaseIcon],
    cached: &PortTables,
    retained: Option<&mut PortPlans>,
    out: &mut CounterFrame<impl Sized, base_windows::PreparedBases, impl Sized, naval_windows::Plan>,
) -> io::Result<()> {
    if !inputs.visibility.province_icons || !inputs.shows(inputs.content.defines.naval_base_cutoff)
    {
        return Ok(());
    }
    let windows = inputs.windows;
    let size = match windows.naval_base_frame(viewport(inputs.viewport, inputs.ui_scale)) {
        None => {
            *out.report
                .icon_gui_notes
                .entry(format!(
                    "{NAVAL_BASE_WINDOW} is not drawn: no GUI template is registered"
                ))
                .or_default() += rows::<tables::NavalBases>(frame)?.rows().len();
            return Ok(());
        }
        Some(Ok(frame)) => frame.size,
        Some(Err(reason)) => {
            let why = format!("port items are not placed: {NAVAL_BASE_WINDOW} {reason}");
            *out.report
                .navy_counter_gaps
                .entry(why.clone().into())
                .or_default() += 1;
            out.naval.bases_undecided = Some(why.into());
            return Ok(());
        }
    };
    let bases = rows::<tables::NavalBases>(frame)?;
    if !bases.is_complete() {
        let why = "the naval base windows are not kept this hour: the NavalBases census is incomplete, so which provinces hold a base is unknown";
        *out.report.naval_window_notes.entry(why.into()).or_default() += 1;
        out.naval.bases_undecided = Some(why.into());
        return Ok(());
    }
    let anchors = &cached.anchors;
    let unanchored = cached.unanchored;
    out.report.unanchored += unanchored;
    let ports = match (context, tables.order) {
        (None, _) => Ok(None),
        (Some(_), _)
            if inputs.camera.free().is_none()
                && port_grouping(
                    &inputs.content.defines,
                    inputs.visibility,
                    inputs.camera.height,
                )
                .cleared =>
        {
            Ok(None)
        }
        (Some(context), Some(_)) => {
            let mut items =
                port_items(inputs, frame, owner, tables, context, cached, retained, out)?;
            if inputs.camera.free().is_some() {
                let at: BTreeMap<u32, [f32; 3]> = anchors.iter().copied().collect();
                let defines = &inputs.content.defines;
                items.retain(|province, _| {
                    at.get(province).is_none_or(|anchor| {
                        !port_grouping(defines, inputs.visibility, inputs.icon_height(*anchor))
                            .cleared
                    })
                });
            }
            Ok(Some(items))
        }
        (Some(_), None) => Err(PORT_ORDER_UNRECORDED.into()),
    };
    let homes = home_bases(
        frame,
        tables.vision.ours(),
        inputs.selected_fleets,
    );
    let mut glyphs = icons.to_vec();
    match homes {
        Ok(homes) => {
            for glyph in &mut glyphs {
                glyph.highlighted = homes.contains(&glyph.province);
            }
        }
        Err(why) => {
            *out.report
                .icon_gui_notes
                .entry(format!("selected fleet home bases: {why}"))
                .or_default() += 1;
        }
    }
    out.naval.bases = Some(naval_windows::BasePass {
        bases: anchors.clone(),
        glyphs,
        ports,
        size,
        unanchored,
    });
    Ok(())
}

pub const PORT_ORDER_UNRECORDED: &str = "port items are not reproduced: the recorded \
     naval occupant arrays (ProvinceUnitOrder) are not whole this hour";

pub const PORT_SHIPS_UNRECORDED: &str = "task-force ship census incomplete";

pub const PORT_OWNER_UNLINKED: &str = "task-force roster owner not a linked country";

pub const PORT_FORCE_UNRECORDED: &str = "task-force row absent from an incomplete census";

pub const PORT_FORCE_GONE: &str = "a task force the port roster admitted has no row at the hour drawn";

pub fn port_undecided(reason: &str) -> Cow<'static, str> {
    format!("port items are not reproduced: {reason}").into()
}

fn unrecorded_port_forces(
    bases: &BTreeSet<u32>,
    order: &counter_order::RecordedOrder,
    context: &NavalContext<'_>,
) -> Vec<(u32, Vec<u64>)> {
    if context.forces_complete {
        return Vec::new();
    }
    let recorded: BTreeSet<u64> = context.forces.iter().map(|force| force.id).collect();
    bases
        .iter()
        .filter_map(|&province| {
            let absent: Vec<u64> = order
                .naval_roster(province)
                .iter()
                .copied()
                .filter(|force| !recorded.contains(force))
                .collect();
            (!absent.is_empty()).then_some((province, absent))
        })
        .collect()
}

#[derive(Default)]
pub struct PortPlans(BTreeMap<u32, PortPlan>);

struct PortPlan {
    grouping: PortGrouping,
    roster: Vec<(u64, u32)>,
    flotillas: Vec<Flotilla>,
}

impl PortPlans {
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

#[allow(clippy::too_many_arguments)]
fn port_items(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    owner: &impl Fn(u32) -> Option<u32>,
    tables: &NavalLinks<'_>,
    context: &NavalContext<'_>,
    cached: &PortTables,
    mut retained: Option<&mut PortPlans>,
    out: &mut CounterFrame<impl Sized, base_windows::PreparedBases, impl Sized, naval_windows::Plan>,
) -> io::Result<naval_windows::PortItems> {
    for (&reason, forces) in &cached.refused {
        out.report
            .refuse_task_forces(reason, forces.iter().copied());
    }
    let navy = inputs.windows.navy();
    let art = gui_windows::CounterArt {
        assets: inputs.assets,
        countries: tables.countries,
    };
    let resources = viewer_counter_kit::gui_windows::gui_assets(inputs, None);
    let ctx = gui_windows::NavyContext {
        settings: inputs.settings,
        selected: inputs.selected,
        defines: &inputs.content.defines,
        art: &art,
        gui: &resources,
    };
    let air_census = (inputs.visibility.priorities == PriorityTable::StrategicAir)
        .then(|| air_grid::Census::new(frame));
    let by_handle: BTreeMap<u64, &replay_data::rows::TaskForceRow> =
        context.forces.iter().map(|force| (force.id, force)).collect();
    let mut ports: naval_windows::PortItems = cached
        .undecided
        .iter()
        .map(|(&province, why)| (province, Err(why.clone())))
        .collect();
    for (&province, roster) in &cached.rosters {
        if ports.contains_key(&province) {
            continue;
        }
        let height = inputs
            .content
            .port(province)
            .map_or(inputs.camera.height, |anchor| inputs.icon_height(anchor));
        let grouping = port_grouping(&inputs.content.defines, inputs.visibility, height);
        let before = retained.as_ref().and_then(|memo| memo.0.get(&province));
        let members_equal = before.is_some_and(|before| before.roster == *roster);
        let hit = inputs.camera.free().is_none()
            && members_equal
            && before.is_some_and(|before| before.grouping == grouping);
        viewer_counter_kit::plan_probe::record(|counts| {
            if retained.is_some() {
                counts.ordered_groups += 1;
                counts.ordered_group_members_equal += u64::from(members_equal);
                counts.semantic_plans += 1;
                counts.semantic_plan_hits += u64::from(hit);
            }
        });
        let flotillas = if hit {
            before.unwrap().flotillas.clone()
        } else {
            let mut units = Vec::with_capacity(roster.len());
            let mut gone = None;
            for &(force, ordinal) in roster {
                match by_handle.get(&force) {
                    Some(row) => units.extend(context.unit(row, owner, ordinal, None)),
                    None => {
                        gone = Some(force);
                        break;
                    }
                }
            }
            if let Some(force) = gone {
                out.report.refuse_task_forces(PORT_FORCE_GONE, [force]);
                ports.insert(province, Err(port_undecided(PORT_FORCE_GONE)));
                if let Some(memo) = retained.as_deref_mut() {
                    memo.0.remove(&province);
                }
                continue;
            }
            let keys = match naval_regional::sort_own(
                &mut units,
                inputs.selected,
                context,
                grouping.coarse,
            ) {
                Ok(keys) => keys,
                Err(reason) => {
                    out.report
                        .refuse_task_forces(reason, units.iter().map(|unit| unit.force.id));
                    ports.insert(province, Err(port_undecided(reason)));
                    if let Some(memo) = retained.as_deref_mut() {
                        memo.0.remove(&province);
                    }
                    continue;
                }
            };
            let flotillas: Vec<_> = port_runs(grouping, &keys, |at| {
                (units[at].slot, tables.countries.known_side(units[at].slot))
            })
            .into_iter()
            .map(|run| context.flotilla(&units[run].iter().collect::<Vec<_>>(), true, false))
            .collect();
            if let Some(memo) = retained.as_deref_mut() {
                memo.0.insert(
                    province,
                    PortPlan {
                        grouping,
                        roster: roster.clone(),
                        flotillas: flotillas.clone(),
                    },
                );
            }
            flotillas
        };
        let mut items = Vec::new();
        for flotilla in &flotillas {
            if flotilla.hidden {
                out.report.no_intel_counters += 1;
            }
            out.report.unavailable_naval_flags += usize::from(flotilla.flag_owner.is_none());
            if let Some(census) = &air_census {
                let grid = match &flotilla.cells {
                    Some(cells) if cells.carrier.iter().all(Option::is_none) => Ok(Vec::new()),
                    Some(cells) if cells.carrier.iter().all(Option::is_some) => {
                        census.as_ref().map_err(Clone::clone).and_then(|census| {
                            census.carrier(
                                inputs,
                                context.ship_inputs.as_ref(),
                                &flotilla.members,
                                context.vision.ours(),
                            )
                        })
                    }
                    _ => Err("carrier wing grid visibility is undecided".into()),
                };
                out.naval
                    .carrier_ports
                    .insert((province, items.len()), grid);
            }
            items.push(gui_windows::navy_calls(
                &ctx,
                navy,
                &gui_windows::NavyState {
                    flotilla,
                    countries: tables.countries,
                },
                &mut out.report.navy_counter_gaps,
            ));
        }
        ports.insert(province, Ok(items));
    }
    Ok(ports)
}

pub const PORT_GRID_FULL: &str =
    "a port item is past box_ships's slot limits, which do not add it; not drawn";

#[cfg(test)]
mod tests;

#[cfg(test)]
mod flag_tests;

pub struct NavalLinks<'a> {
    pub countries: &'a Arc<Countries>,
    pub vision: &'a Vision,
    pub order: Option<&'a counter_order::RecordedOrder>,
    pub task_forces: &'a viewer_unit_reads::task_force_visual::TaskForceMotions,
}


pub fn home_bases(frame: &DecodedFrame, viewer: u32, selected: &[u64]) -> io::Result<Vec<u32>> {
    if selected.is_empty() {
        return Ok(Vec::new());
    }
    let fleets = frame
        .complete::<tables::Fleets>()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let mut result = Vec::new();
    for id in selected {
        let Some(fleet) = fleets
            .binary_search_by_key(id, |row| row.id)
            .ok()
            .map(|at| &fleets[at])
        else {
            continue;
        };
        let owner = links
            .binary_search_by_key(&fleet.owner, |row| row.id)
            .ok()
            .map(|at| links[at].canonical_slot);
        if owner == Some(viewer) && fleet.home_base != 0 && !result.contains(&fleet.home_base) {
            result.push(fleet.home_base);
        }
    }
    Ok(result)
}