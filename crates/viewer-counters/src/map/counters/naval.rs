//! Naval counters: task forces as map sources beside the land units, and the naval base's port
//! items.

use super::*;
use viewer_foundation::map::gui_resources::viewport;

/// Naval and land counters share member bar and hidden-state accumulation.
/// Their kind then selects separate children. Port mission items use their own
/// update and draw paths; supply, capital count, carrier and pride decisions
/// retain their distinct visibility and member-order rules.
#[derive(Clone)]
pub(super) struct Flotilla {
    pub(super) members: Arc<[u64]>,
    /// The item's first admitted task force, which identifies the
    /// counter the viewer, as the game's client, keeps between updates.
    pub(super) source: u64,
    pub(super) owner: u32,
    /// The first task force's logical owner, whose colour a plate takes in
    /// country-colour mode; `None` when the reference is not a linked country.
    pub(super) logical: Option<u32>,
    pub(super) type_tint: Option<type_tint::Side>,
    /// Naval counter window kind. Port mission items use a distinct retained
    /// window and update path.
    pub(super) side: Result<Allegiance, String>,
    /// `None` only for a mixed-owner port item whose Ships census is incomplete.
    pub(super) flag_owner: Option<u32>,
    /// A mission-list unit item in a naval base's `box_ships`.
    pub(super) port: bool,
    /// Coarse grouping at the current camera. Port items never use coarse windows.
    pub(super) coarse: bool,
    pub(super) ships: usize,
    /// Whether any task force here is below the engine's 250 visibility.
    pub(super) hidden: bool,
    /// The detailed selected outline requires at least one fully visible member.
    pub(super) visible_member: bool,
    /// `None` when the hour lacks the ship operands.
    pub(super) cells: Option<NavalCells>,
    pub(super) bars: NavalBars,
    pub(super) supply: counter_inputs::NavalSupply,
    pub(super) status: counter_inputs::NavalStatus,
    /// The port item's `mission_type` frame.
    pub(super) mission: Option<u32>,
    /// `group_marker`'s colour when it shows, or why the hour cannot decide it (the fleet's row
    /// or its reserve test).
    pub(super) marker: Result<Option<[f32; 4]>, String>,
    /// The representative glides along its route this hour.
    pub(super) moving: bool,
    /// A mission-list unit item's count operands; `None` for an at-sea counter.
    pub(super) item: Option<ItemIntel>,
    /// The port callback's retained independent metadata bytes.
    pub(super) port_tooltip:
        Option<viewer_foundation::map::counter_hovers::task_force_port::Metadata>,
    /// `counter_ideology`'s frame at sea: `Some` when the game shows it, `None` when it stays
    /// hidden ([`ideology_frame`], of the first task force's logical owner); `Err` names the
    /// operand the hour lacks. A port item's window never shows it : `Ok(None)`.
    pub(super) ideology: Result<Option<i32>, String>,
}

/// The count operands: the first task force's visibility and handle seed the
/// count ranges, while ship counts sum over every member regardless of visibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ItemIntel {
    pub(super) byte: u16,
    pub(super) first: u64,
    pub(super) ships: usize,
}

/// Below this first-member visibility (0x96), the `type` child uses the no-intel
/// sprite and `pride_of_the_fleet_icon` stays hidden. A port item's members all
/// reach this test; a combat tile's need not.
pub(super) const MISSION_ITEM_DETAIL: u16 = 150;

impl Flotilla {
    /// Whether the `type` child shows the no-intel sprite: a mission item's first member below
    /// [`MISSION_ITEM_DETAIL`], or an at-sea counter with any hidden member
    ///
    pub(super) fn no_intel_type(&self) -> bool {
        match self.item {
            Some(item) => item.byte < MISSION_ITEM_DETAIL,
            None => self.hidden,
        }
    }

    /// Show pride when a qualifying member supplies it; port items also require
    /// full representative visibility.
    pub(super) fn shows_pride(&self) -> Option<bool> {
        if self
            .item
            .is_some_and(|item| item.byte < MISSION_ITEM_DETAIL)
        {
            return Some(false);
        }
        self.status.pride
    }
}

/// What a naval counter's two bars draw.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum NavalBars {
    Hidden,
    /// `(organisation, strength)` fills.
    Fill(f32, f32),
    /// The code shows them, but a member's ratios are not recorded, by why.
    Unknown(String),
}

/// What the ship operands put on one naval counter.
#[derive(Clone)]
pub(super) struct NavalCells {
    /// Capital ships summed over every member regardless of visibility; `None`
    /// when a member's ship lacks the operands.
    pub(super) capitals: Option<usize>,
    /// The `type` child's hull.
    pub(super) hull: counter_order::HullIcon,
    /// Every `air_base_icon` state the unrecorded operands leave possible.
    pub(super) carrier: BTreeSet<Option<(&'static str, u32)>>,
}

/// Ship-model joins and fixed-point bar operands, built once per hour.
#[derive(Clone)]
pub(super) struct ShipTables {
    models: replay_frame::frame::SharedView,
    rosters: BTreeMap<u64, Vec<(usize, i64)>>,
    ratios: BTreeMap<u64, (i64, i64)>,
    base_kinds: BTreeMap<u64, i32>,
    viewer_wings: Option<BTreeMap<u64, bool>>,
}

/// A camera's borrowed content paired with the shared ship operands.
pub(super) struct ShipInputs<'a> {
    pub(super) equipment: Result<&'a installed_content_core::equipment::Database, &'a str>,
    pub(super) tables: Arc<ShipTables>,
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

    #[cfg(test)]
    pub(super) fn from_rosters(rosters: BTreeMap<u64, Vec<counter_order::Hull<'_>>>) -> Arc<Self> {
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
    pub(super) fn hulls(&self, force: u64) -> impl Iterator<Item = counter_order::Hull<'_>> {
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

    /// Whether a task force's first carrier, in roster order, has a carrier air base: the air
    /// map mode's test for drawing its counter. `None` when a ship before it lacks model operands.
    pub(super) fn carries_air_wings(&self, force: u64) -> Option<bool> {
        let ships: Vec<_> = self.hulls(force).map(|hull| hull.model).collect();
        counter_order::first_carrier_has_air_base(&ships)
    }

    /// A task force's strength and organisation ratios; an empty roster returns zero for each.
    pub(super) fn ratio(&self, force: u64) -> (i64, i64) {
        self.tables.ratios.get(&force).copied().unwrap_or((0, 0))
    }

    pub(super) fn cells(&self, members: &[u64], port: bool) -> NavalCells {
        let roster = |force: &u64| self.hulls(*force);
        let capitals = members
            .iter()
            .flat_map(roster)
            .map(|hull| counter_order::is_capital(hull.model))
            .try_fold(0usize, |sum, capital| Some(sum + usize::from(capital?)));
        let hull = counter_order::hull_icon(members.iter().flat_map(roster), self.equipment);
        // The viewer's slot is what the game test, with fog on or off; an
        // incomplete census cannot test it, so every carrier state stays open.
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

/// One admitted task force.
pub(super) struct NavalUnit<'a> {
    pub(super) force: &'a replay_data::rows::TaskForceRow,
    /// The ordinal in its province's occupant array; zero without a recorded order.
    pub(super) ordinal: u32,
    pub(super) slot: u32,
    /// The logical owner, when it is a linked country.
    pub(super) logical: Option<u32>,
    /// The detailed [`unit_flag_owner`]: port items are never coarse, and a coarse sea plate
    /// draws `slot`.
    pub(super) flag_owner: u32,
    pub(super) sight: Sight,
    /// Visibility for this perspective.
    pub(super) byte: u16,
    pub(super) ships: usize,
    /// Where it stands at sea; `None` in port.
    pub(super) position: Option<counter_move::Position>,
    /// Summed `Ships.strength`; partial when the census is.
    pub(super) strength: i64,
}

/// Everything a naval counter item reads besides its own task forces, for one hour.
pub(super) struct NavalContext<'a> {
    pub(super) type_tints: &'a type_tint::Index,
    pub(super) vision: &'a Vision,
    pub(super) native: &'a NativeInputs,
    pub(super) ship_inputs: Option<ShipInputs<'a>>,
    pub(super) aggression: &'a [i64],
    /// Ship handle to `Ships.task_force`, and whether that census is complete.
    pub(super) ship_task_forces: Arc<BTreeMap<u64, u64>>,
    pub(super) ships_complete: bool,
    /// Per task force, its ship count and summed `Ships.strength`, with wrapping addition.
    pub(super) ships: Arc<BTreeMap<u64, (usize, i64)>>,
    /// Canonical slot to `Countries.id`, the key of `CountryCounterInputs`.
    pub(super) country_ids: &'a BTreeMap<u32, u32>,
    /// A logical owner's `counter_ideology` frame at sea ([`ideology_frame`]).
    pub(super) ideology: &'a Ideology<'a>,
    /// `TaskForces.mission` by task force.
    pub(super) missions: Arc<BTreeMap<u64, u32>>,
    /// The recorded task forces, and whether that census is complete.
    pub(super) forces: &'a [replay_data::rows::TaskForceRow],
    /// The mission skip operands, shared with the ship models.
    pub(super) mission_skips: Arc<viewer_foundation::map::task_force_visual::MissionSkips>,
    pub(super) forces_complete: bool,
}

/// The hour's inputs a [`NavalContext`] takes besides the frame.
#[derive(Clone, Copy)]
pub(super) struct NavalSources<'a> {
    pub(super) type_tints: &'a type_tint::Index,
    pub(super) equipment: Result<&'a installed_content_core::equipment::Database, &'a str>,
    pub(super) vision: &'a Vision,
    pub(super) native: &'a NativeInputs,
    pub(super) country_ids: &'a BTreeMap<u32, u32>,
    pub(super) aggression: &'a [i64],
    pub(super) ideology: &'a Ideology<'a>,
}

/// A country slot's `counter_ideology` frame at sea, as [`ideology_frame`] decides it.
pub(super) type Ideology<'a> = dyn Fn(u32) -> Result<Option<i32>, String> + Sync + 'a;

/// Interpret the ideology group's raw frame bits as a signed integer.
fn set_frame_argument(number: u32) -> i32 {
    i32::from_ne_bytes(number.to_ne_bytes())
}

/// Ideology overlay follows the logical owner's ruling party when its civil-war
/// status permits it. The ideology group's recorded frame bits are used as a
/// signed frame argument. Missing reached politics or ideology data stays unknown.
pub(super) fn ideology_frame(
    frame: &DecodedFrame,
    ideologies: &installed_content_core::ideologies::Ideologies,
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
pub(super) struct NavalTables {
    ship_inputs: Option<Arc<ShipTables>>,
    ship_task_forces: Arc<BTreeMap<u64, u64>>,
    ships_complete: bool,
    ships: Arc<BTreeMap<u64, (usize, i64)>>,
    missions: Arc<BTreeMap<u64, u32>>,
    mission_skips: Arc<viewer_foundation::map::task_force_visual::MissionSkips>,
    forces_complete: bool,
}

impl NavalTables {
    pub(super) fn read(frame: &DecodedFrame, vision: &Vision) -> io::Result<Self> {
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
                viewer_foundation::map::task_force_visual::MissionSkips::read(frame)?,
            ),
            forces_complete: forces.is_complete(),
        })
    }
}

impl<'a> NavalContext<'a> {
    #[cfg(test)]
    pub(super) fn read(
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

    pub(super) fn build(frame: &'a DecodedFrame, sources: NavalSources<'a>) -> io::Result<Self> {
        Self::cached(frame, sources, &NavalTables::read(frame, sources.vision)?)
    }

    pub(super) fn cached(
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

    /// A task force the perspective is shown at all, as one counter member.
    pub(super) fn unit(
        &self,
        force: &'a replay_data::rows::TaskForceRow,
        owner: &impl Fn(u32) -> Option<u32>,
        ordinal: u32,
        position: Option<counter_move::Position>,
    ) -> Option<NavalUnit<'a>> {
        let slot = owner(force.roster_owner)?;
        let byte = self.vision.navy_byte(force.id, slot);
        let sight = self.vision.sees_navy(force.id, slot);
        let (ships, strength) = self.ships.get(&force.id).copied().unwrap_or_default();
        (sight != Sight::None).then(|| NavalUnit {
            force,
            ordinal,
            slot,
            logical: owner(force.logical_owner),
            flag_owner: unit_flag_owner(slot, force.expeditionary_owner, false, owner),
            sight,
            byte,
            ships,
            position,
            strength,
        })
    }

    /// the recorded roster's first task force is in reserve.
    /// A missing roster or mission leaves the result unknown.
    pub(super) fn reserve(&self, fleet: u64) -> Option<bool> {
        if fleet == 0 {
            return Some(false);
        }
        self.native.fleet(fleet).and_then(|row| {
            counter_inputs::reserve_fleet(row, |force| self.missions.get(&force).copied())
        })
    }

    fn fleet_color(&self, fleet: u64) -> Option<[f32; 4]> {
        let row = self.native.fleet(fleet)?;
        Some(counter_inputs::color(
            row.red, row.green, row.blue, row.alpha,
        ))
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

    /// Builds one counter item from its task forces in admission order.
    /// One plate of `units`; `coarse` is whether its column draws coarse (port items never do).
    pub(super) fn flotilla(&self, units: &[&NavalUnit<'_>], port: bool, coarse: bool) -> Flotilla {
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
            // Port bars draw for an own item whatever the visibility,
            // and otherwise only at the first member's visibility 250; the means are over every
            // member.
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
            // An own item whose first and last members share a fleet.
            Some(own && fleet == last.force.fleet && fleet != 0)
        } else if own && fleet != 0 {
            // an own representative's fleet that the game does not call
            // reserve, with its recorded colour.
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
            Some(true) => self.fleet_color(fleet).map(Some).ok_or_else(|| {
                format!("group_marker is undecided: fleet {fleet}'s colour (FleetCounterInputs) is not recorded")
            }),
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
            viewer_foundation::map::counter_hovers::task_force_port::Metadata::from_members(
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
                .sum(),
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
            // Test only the first and last missions of an own item.
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
                ships: units.iter().map(|unit| unit.ships).sum(),
            }),
            port_tooltip,
        }
    }
}

/// Whether a task force placed at visual kind `kind` in a province of `region` stays an
/// ordinary counter source. the game buckets combat task forces for their models, but
/// Exclude combat task forces from the counter member list.
/// Outside combat, one on its mission here is left to the mission icon
/// ([`MissionSkips::skip`], which the ship models share, so its formation is suppressed too).
/// One whose skip operands are unrecorded is refused by reason.
fn stays_a_source(
    skips: &viewer_foundation::map::task_force_visual::MissionSkips,
    force: &replay_data::rows::TaskForceRow,
    kind: u32,
    region: Option<u32>,
    report: &mut CounterReport,
) -> bool {
    // Read the signed member count, independently of the stack kind.
    if force.combat_references > 0 {
        return false;
    }
    match skips.skip(force, kind, region) {
        Some(skipped) => !skipped,
        None => {
            report.refuse_task_forces(
                "its mission skip's counter inputs or current region are not recorded",
                [force.id],
            );
            false
        }
    }
}

/// Seed both flag choices with the first force's detailed flag owner. With multiple
/// roster owners, sum each owner's ship strengths and select the lowest signed total,
/// breaking ties by ascending raw country reference. Draw this mixed-owner choice
/// only for the mixed case; otherwise draw the first force's flag. Owners compare
/// by canonical slot and keep their first raw reference as the tie key. An incomplete
/// `Ships` census withholds the mixed flag because its totals are unknown.
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

/// [`counter_inputs::member_mean`] of each member's pair; a member whose pair is not recorded
/// leaves the fills unknown.
pub(super) fn mean_bars(ratios: impl Iterator<Item = Option<(i64, i64)>>) -> NavalBars {
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

/// Why a naval counter's bar fills are undecided.
pub(super) const BARS_UNRECORDED: &str = "the strength and organisation bars are undecided: a \
     member's ratios (the ship counter operands) are not recorded";

/// Two task forces first pass the common owner test ([`JoinRules::owners_join`]:
/// roster owners in country mode, allegiance categories otherwise). Then any
/// two coarse or foreign members may join. An own representative accepts its
/// own fleet or another fleet when both fleets are reserve fleets.
pub(super) fn naval_can_join(
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

/// Why the air map mode cannot test a task force for carriers: its ships' model operands are not
/// all recorded this hour.
const CARRIER_TEST_UNRECORDED: &str = "the air map mode keeps only fleets carrying air wings, and this hour's ship models cannot tell";

/// Whether a map mode keeps an at-sea task force's counter: every one, none in the operatives
/// mode, and in the air mode those whose first carrier has an air base (`carries`), `None` when
/// that cannot be told.
pub(super) fn mode_keeps_task_force(
    kinds: UnitKinds,
    carries: impl FnOnce() -> Option<bool>,
) -> Option<bool> {
    match kinds {
        UnitKinds::All => Some(true),
        UnitKinds::Nothing => Some(false),
        UnitKinds::CarrierFleets => carries(),
    }
}

/// Offers every task force at sea to its province's source for its visual bucket.
///
/// Walk each province's mixed occupant array, whose task forces share the land
/// units' buckets. the game then excludes task forces in combat; a docked one (bucket 19)
/// is also left out of the ordinary pass and belongs to the naval base.
pub(super) fn add_task_forces<'a>(
    inputs: &CounterInputs<'_>,
    owner: &impl Fn(u32) -> Option<u32>,
    tables: &CounterTables<'a>,
    context: &NavalContext<'a>,
    admission: &Admission,
    sources: &mut BTreeMap<StackKey, Source<'a>>,
    out: &mut CounterFrame,
) {
    let world = inputs.route_world();
    let land = |id| is_land(inputs, id);
    let anchor = |id, slot| inputs.content.anchor(id, slot);
    for force in context.forces {
        // At sea a task force is a unit source like a land stack, so the same camera admission
        // decides whether its province activates one.
        if !admission.admits_province(&inputs.content.bounds, force.province) {
            continue;
        }
        let ordinal = match tables.order {
            Some(order) => match order.mixed(force.id, force.province) {
                Some(ordinal) => ordinal,
                None => continue,
            },
            None => 0,
        };
        let placement =
            match tables
                .task_forces
                .visual(force, inputs.movement, world.as_ref(), &anchor, &land)
            {
                Ok(placement) => placement,
                Err(reason) => {
                    out.report.refuse_task_forces(reason, [force.id]);
                    continue;
                }
            };
        if let Some(reason) = placement.unplaced_route {
            out.report
                .unplaced_task_force_routes
                .entry(reason)
                .or_default()
                .insert(force.id);
        }
        // The ordinary pass skips bucket 19; the naval base shows a docked task force.
        if placement.kind == SHIP_IN_PORT {
            continue;
        }
        // Docked fleets stay with their naval base whatever the map mode's unit kinds.
        let carries = || {
            context
                .ship_inputs
                .as_ref()
                .and_then(|ships| ships.carries_air_wings(force.id))
        };
        match mode_keeps_task_force(inputs.visibility.unit_kinds, carries) {
            Some(true) => {}
            Some(false) => continue,
            None => {
                out.report
                    .refuse_task_forces(CARRIER_TEST_UNRECORDED, [force.id]);
                continue;
            }
        }
        if !stays_a_source(
            &context.mission_skips,
            force,
            placement.kind,
            inputs.content.region(force.province),
            &mut out.report,
        ) {
            continue;
        }
        if let Some(unit) = context.unit(force, owner, ordinal, Some(placement.position)) {
            sources
                .entry((force.province, placement.kind))
                .or_default()
                .units
                .push(SourceUnit::Naval(unit));
        }
    }
}

/// Whether a province is land: the content movement map's terrain flag, else the map's own kind.
pub(super) fn is_land(inputs: &CounterInputs<'_>, province: u32) -> bool {
    province_is_land(inputs.map, inputs.movement, province)
}

fn province_is_land(map: &MapData, movement: Option<&MovementContent>, province: u32) -> bool {
    movement
        .and_then(|movement| movement.map.terrain(province))
        .map_or_else(
            || {
                map.definitions
                    .get(&province)
                    .is_some_and(|definition| matches!(definition.kind, ProvinceKind::Land))
            },
            |terrain| terrain.flags & 1 != 0,
        )
}

/// Port frames use coarse sorting in the political mode or beyond its mode's coarse
/// distance. Only political mode and strategic-level grouping replace that comparator
/// with allegiance boundaries; closer strategic views keep fleet/mission/selection splits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PortGrouping {
    cleared: bool,
    coarse: bool,
    allegiance: bool,
}

pub(super) fn port_grouping(
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
    first_side: Allegiance,
    next_side: Allegiance,
) -> bool {
    if grouping.allegiance {
        naval_regional::allegiance_break(first_side, next_side, first.selected, next.selected)
    } else {
        naval_regional::compare(first, next, grouping.coarse) < 0
    }
}

fn port_runs(
    grouping: PortGrouping,
    keys: &[naval_regional::SortKey],
    side: impl Fn(usize) -> Allegiance,
) -> Vec<std::ops::Range<usize>> {
    let mut runs = Vec::new();
    let mut start = 0;
    for next in 1..=keys.len() {
        if next == keys.len()
            || port_boundary(grouping, keys[start], keys[next], side(start), side(next))
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

/// Camera-independent port membership and anchor decisions. Comparator keys and
/// runs still read the camera height, map mode and local task-force selection.
#[derive(Clone, Default)]
pub(super) struct PortTables {
    anchors: Vec<(u32, [f32; 3])>,
    unanchored: usize,
    rosters: BTreeMap<u32, Vec<(usize, u32)>>,
    undecided: BTreeMap<u32, Cow<'static, str>>,
    refused: BTreeMap<&'static str, BTreeSet<u64>>,
}

impl PortTables {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn read(
        frame: &DecodedFrame,
        content: &CounterContent,
        map: &MapData,
        movement: Option<&MovementContent>,
        tables: &CounterTables<'_>,
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
        for (index, force) in context.forces.iter().enumerate() {
            if force.province == 0
                || force.fleet == 0
                || !province_is_land(map, movement, force.province)
                || !base_provinces.contains(&force.province)
            {
                continue;
            }
            match port_admission(
                force,
                context,
                order,
                || tables.task_forces.docked(force.id),
                owner,
            ) {
                PortAdmission::Out => {}
                PortAdmission::Undecided(reason) => {
                    out.refused.entry(reason).or_default().insert(force.id);
                    out.undecided.insert(force.province, port_undecided(reason));
                }
                PortAdmission::In(ordinal, unit) => {
                    out.rosters
                        .entry(unit.force.province)
                        .or_default()
                        .push((index, ordinal));
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

/// The naval base icons whose windows hold the port items, and the items: the game walks each
/// land province's naval occupant array, independently of the ordinary
/// sources, admits task forces with a fleet, ships and an empty ordinary route, sorts them and
/// appends each comparator-defined run to the base's `box_ships`.
///
/// The base icons are planned below `NAVAL_BASE_DISTANCE_CUTOFF`, whatever their items. The items
/// are cleared and nothing is filled when the camera is coarse in the default map
/// mode, and while the viewing country takes part in a peace conference (`context` is `None` then,
/// [`super::units_shown`]).
#[allow(clippy::too_many_arguments)]
pub(super) fn port_counters(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    owner: &impl Fn(u32) -> Option<u32>,
    tables: &CounterTables<'_>,
    context: Option<&NavalContext<'_>>,
    icons: &[NavalBaseIcon],
    cached: &PortTables,
    out: &mut CounterFrame,
) -> io::Result<()> {
    port_counters_with(
        inputs, frame, owner, tables, context, icons, cached, None, out,
    )
}

/// Retains only pure, ordered port flotillas; resource requests and native call generation
/// remain on the original path, in the original order, on every preparation.
#[allow(clippy::too_many_arguments)]
pub(super) fn port_counters_with(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    owner: &impl Fn(u32) -> Option<u32>,
    tables: &CounterTables<'_>,
    context: Option<&NavalContext<'_>>,
    icons: &[NavalBaseIcon],
    cached: &PortTables,
    retained: Option<&mut PortPlans>,
    out: &mut CounterFrame,
) -> io::Result<()> {
    // The raids map mode clears naval base icons, and their port counters with them.
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
    // A coarse camera, or a peace conference (no context), empties every box; a recorded order
    // the hour does not hold whole leaves every base's items undecided. A free eye empties the
    // boxes of the bases beyond the coarse grouping distance.
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
    let homes = crate::map::naval_selection::home_bases(
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

/// Every base's port items are undecided when the recorded occupant arrays
/// (`ProvinceUnitOrder`) the pass walks are incomplete for this hour.
pub(super) const PORT_ORDER_UNRECORDED: &str = "port items are not reproduced: the recorded \
     naval occupant arrays (ProvinceUnitOrder) are not whole this hour";

/// Why a task force at a naval base, and so the base's port roster, is undecided: its ships are
/// not in the incomplete Ships census.
pub(super) const PORT_SHIPS_UNRECORDED: &str = "task-force ship census incomplete";

/// Why a task force at a naval base, and so the base's port roster, is undecided: its roster
/// owner is not a linked country.
pub(super) const PORT_OWNER_UNLINKED: &str = "task-force roster owner not a linked country";

/// A naval base's port roster is undecided when its recorded occupant array names
/// a task force whose row is absent from the incomplete `TaskForces` census.
pub(super) const PORT_FORCE_UNRECORDED: &str = "task-force row absent from an incomplete census";

/// Why a base's port roster is undecided, for a task force's `reason`.
fn port_undecided(reason: &str) -> Cow<'static, str> {
    format!("port items are not reproduced: {reason}").into()
}

/// Port membership needs ships, province-occupant membership, an empty ordinary
/// route and the docked display state. Missing reached operands remain unknown.
enum PortAdmission<'a> {
    Out,
    Undecided(&'static str),
    In(u32, NavalUnit<'a>),
}

fn port_admission<'a>(
    force: &'a replay_data::rows::TaskForceRow,
    context: &NavalContext<'a>,
    order: &counter_order::RecordedOrder,
    docked: impl FnOnce() -> Result<bool, &'static str>,
    owner: &impl Fn(u32) -> Option<u32>,
) -> PortAdmission<'a> {
    match context.ships.get(&force.id) {
        Some((count, _)) if *count > 0 => {}
        Some(_) => return PortAdmission::Out,
        None if context.ships_complete => return PortAdmission::Out,
        None => return PortAdmission::Undecided(PORT_SHIPS_UNRECORDED),
    }
    let Some(ordinal) = order.naval(force.id, force.province) else {
        return PortAdmission::Out;
    };
    match docked() {
        Ok(true) => {}
        Ok(false) => return PortAdmission::Out,
        Err(reason) => return PortAdmission::Undecided(reason),
    }
    if owner(force.roster_owner).is_none() {
        return PortAdmission::Undecided(PORT_OWNER_UNLINKED);
    }
    match context.unit(force, owner, ordinal, None) {
        Some(unit) => PortAdmission::In(ordinal, unit),
        None => PortAdmission::Out,
    }
}

/// Each naval base province whose recorded occupant array names task forces absent
/// from the incomplete `TaskForces` census, with those task forces. Whether they join
/// its port roster is undecided ([`PORT_FORCE_UNRECORDED`]); none are reported while
/// the census is complete.
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

/// A semantic memo contains neither native windows nor resolved atlas resources.
#[derive(Default)]
pub(super) struct PortPlans(BTreeMap<u32, PortPlan>);

struct PortPlan {
    grouping: PortGrouping,
    roster: Vec<(usize, u32)>,
    flotillas: Vec<Flotilla>,
}

impl PortPlans {
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }
}

/// Each naval base province's port items this hour: their calls, in the order the code appends
/// them, or why the province's roster is undecided (a task force dropped for an operand the hour
/// lacks, or a roster the native sort refuses), each such task force listed by why.
#[allow(clippy::too_many_arguments)]
fn port_items(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    owner: &impl Fn(u32) -> Option<u32>,
    tables: &CounterTables<'_>,
    context: &NavalContext<'_>,
    cached: &PortTables,
    mut retained: Option<&mut PortPlans>,
    out: &mut CounterFrame,
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
    let resources = gui_windows::gui_assets(inputs, None);
    let ctx = gui_windows::NavyContext {
        settings: inputs.settings,
        selected: inputs.selected,
        defines: &inputs.content.defines,
        art: &art,
        gui: &resources,
    };
    let air_census = (inputs.visibility.priorities == PriorityTable::StrategicAir)
        .then(|| air_grid::Census::new(frame));
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
        #[cfg(test)]
        if retained.is_some() {
            super::preparation::plan_probe::record(|counts| {
                counts.ordered_groups += 1;
                counts.ordered_group_members_equal += u64::from(members_equal);
                counts.semantic_plans += 1;
                counts.semantic_plan_hits += u64::from(hit);
            });
        }
        let flotillas = if hit {
            before.unwrap().flotillas.clone()
        } else {
            let mut units: Vec<_> = roster
                .iter()
                .filter_map(|&(index, ordinal)| {
                    context.unit(&context.forces[index], owner, ordinal, None)
                })
                .collect();
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
            let flotillas: Vec<_> =
                port_runs(grouping, &keys, |at| tables.countries.side(units[at].slot))
                    .into_iter()
                    .map(|run| {
                        context.flotilla(&units[run].iter().collect::<Vec<_>>(), true, false)
                    })
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

/// Count each port item beyond `box_ships`'s slot limits, which the game does not add.
pub(super) const PORT_GRID_FULL: &str =
    "a port item is past box_ships's slot limits, which do not add it; not drawn";

#[cfg(test)]
mod tests;

#[cfg(test)]
mod flag_tests;
