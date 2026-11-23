
use super::*;
use replay_derive::directed_wars::Owner;
use viewer_foundation::tables::{Available, rows};
use viewer_foundation::visibility::event_shared;
use viewer_foundation::tooltip_text::localise;
use viewer_foundation::tooltip_text::state_name;
use viewer_runtime::unit_handle::Unit;
use content_reader_unit_path::{
    unit_path::CommonClass, unit_path::CommonCombat, unit_path::CommonMoveText,
    unit_path::CommonMoveWorld, unit_path::CommonUnit, unit_path::NativeArmyOccupant,
    unit_path::selected_transport as transport,
};
use replay_frame::frame::DecodedFrame;

pub struct CommonCells<'a> {
    frame: &'a DecodedFrame,
    wars: Mutex<BTreeMap<Country, Owner<'a>>>,
}

impl<'a> CommonCells<'a> {
    pub fn new(frame: &'a DecodedFrame) -> Self {
        Self {
            frame,
            wars: Default::default(),
        }
    }
}

const TRANSPORT_RETIRED: &str = "the selected transport preview is retired: transport admission past the naval-base checks, convoys and risk are not recorded";

impl<'w, 'a> Moves<'w, 'a> {
    fn runtime_scalars(&self) -> Fact<&replay_data::rows::PoliticalRuntimeScalars> {
        let common = self.common.ok_or("selected common frame context missing")?;
        let [row] = common
            .frame
            .complete::<tables::SimulationFeatures>()
            .map_err(|e| e.to_string())?
        else {
            return Err("selected SimulationFeatures singleton missing or duplicated".into());
        };
        let values = row
            .runtime_scalars
            .require("SimulationFeatures.runtime_scalars")?;
        if !values.sample_hour.fresh_at(common.frame.hour) {
            return Err("selected movement threshold sample is stale".into());
        }
        Ok(values)
    }

    fn selected_railway(
        &self,
        province: u32,
    ) -> Fact<
        Option<(
            &'a replay_data::rows::RailCooldownRow,
            &'a replay_data::rows::RailwaySelectionInputs,
        )>,
    > {
        let rows = self.table::<tables::RailCooldowns>()?;
        let Ok(index) = rows.binary_search_by_key(&province, |row| row.province) else {
            return Ok(None);
        };
        let row = &rows[index];
        let inputs = row
            .selection
            .require("RailCooldowns.selection")?
            .as_ref()
            .ok_or("selected railway input capture refused")?;
        if row
            .selection_hour
            .require("RailCooldowns.selection_hour")?
            .ok_or("selected railway sample hour capture refused")?
            != self.world.frame().hour()
        {
            return Err("selected railway input sample is stale".into());
        }
        Ok(Some((row, inputs)))
    }

    pub fn selected_transport_message(
        &self,
        selected: &[Unit],
        target: u32,
        content: &impl CommonMoveText,
    ) -> Fact<transport::Outputs> {
        let selection: Vec<_> = selected
            .iter()
            .map(|unit| match *unit {
                Unit::Division(handle) => transport::Unit::Army(handle),
                Unit::RailwayGun(handle) => transport::Unit::RailwayGun(handle),
                Unit::TaskForce(_) => transport::Unit::Other,
            })
            .collect();
        transport::message(
            &selection,
            target,
            false,
            &mut PartialTransport { moves: self },
            content,
        )
    }

    fn common_owner<T>(
        &self,
        receiver: Country,
        query: impl FnOnce(&Owner<'a>) -> Fact<T>,
    ) -> Fact<T> {
        let common = self.common.ok_or("selected common frame context missing")?;
        let mut owners = common
            .wars
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let std::collections::btree_map::Entry::Vacant(entry) = owners.entry(receiver) {
            entry.insert(Owner::new(
                common.frame,
                self.actual_country_raw(receiver)?,
            )?);
        }
        query(&owners[&receiver])
    }

    fn common_discloses_raw(&self, receiver_raw: u32, target_raw: u32) -> Fact<bool> {
        self.common_discloses_access(receiver_raw, target_raw, false)
    }

    fn common_discloses_access(
        &self,
        receiver_raw: u32,
        target_raw: u32,
        access: bool,
    ) -> Fact<bool> {
        let receiver = self.slot(receiver_raw)?;
        let actual = self.actual_country_raw(receiver)?;
        if actual == target_raw {
            return Ok(true);
        }
        let target = self.slot(target_raw)?;
        if actual != 0 && target_raw != 0 && receiver == target {
            return Ok(true);
        }
        if receiver == 0 || target == 0 {
            return event_shared(
                self.world.frame(),
                self.world.links(),
                receiver_raw,
                target_raw,
                access,
            )
            .map_err(str::to_owned);
        }
        if self.common_owner(receiver, |owner| owner.pointer_active_slot(target))? {
            return Ok(false);
        }
        if access && self.military_access(receiver, target)? {
            return Ok(true);
        }
        if let Some(faction) = self.faction(receiver)?
            && self.faction(target)? == Some(faction)
        {
            return Ok(true);
        }
        self.common_subject_access(receiver, target)
    }

    fn common_subject_access(&self, receiver: Country, target: Country) -> Fact<bool> {
        let hour = self.world.frame().hour();
        let actual = self.actual_country_raw(receiver)?;
        if receiver == 0 {
            let rows = self.table::<tables::ReservedCountryNaming>()?;
            let [row] = rows else {
                return Err("transport actual-zero visibility singleton missing".into());
            };
            let packet = row
                .reverse_visibility
                .require("ReservedCountryNaming.reverse_visibility")?
                .as_ref()
                .ok_or("transport actual-zero subject access refused")?;
            if !packet.sample_hour.fresh_at(hour) {
                return Err("transport actual-zero subject access stale".into());
            }
            let cell = packet
                .cells
                .get(target as usize)
                .and_then(Option::as_ref)
                .ok_or("transport actual-zero subject cell refused")?;
            return replay_derive::transport_subject_access::matches(
                cell.subject
                    .as_ref()
                    .ok_or("transport actual-zero subject pointer refused")?,
                cell.target
                    .ok_or_else(|| "transport actual-zero status target refused".into()),
                |raw| self.slot(raw),
            );
        }
        if target == 0 {
            let rows = self.table::<tables::CountryCounterInputs>()?;
            let row = rows
                .iter()
                .find(|row| row.country == actual)
                .ok_or("transport forward-zero country receiver missing")?;
            let packet = row
                .forward_visibility
                .require("CountryCounterInputs.forward_visibility")?
                .as_ref()
                .ok_or("transport forward-zero subject access refused")?;
            return replay_derive::transport_subject_access::matches(
                packet
                    .subject
                    .as_ref()
                    .ok_or("transport forward-zero subject pointer refused")?,
                packet
                    .target
                    .ok_or_else(|| "transport forward-zero status target refused".into()),
                |raw| self.slot(raw),
            );
        }
        let rows = self.table::<tables::RelationLinks>()?;
        let Some(packet) =
            replay_derive::transport_subject_access::current(rows, actual, receiver, target, hour)?
        else {
            return Ok(false);
        };
        replay_derive::transport_subject_access::matches(
            packet
                .subject
                .as_ref()
                .ok_or("transport current subject pointer refused")?,
            packet
                .target_raw
                .ok_or_else(|| "transport current status target refused".into()),
            |raw| self.slot(raw),
        )
    }

    fn common_task_force(&self, handle: u64, province: u32) -> Fact<Option<&'a TaskForceRow>> {
        let divisions = self.table::<tables::Divisions>()?;
        let forces = self.table::<tables::TaskForces>()?;
        let guns = self.table::<tables::RailwayGunCounterInputs>()?;
        let army = divisions
            .binary_search_by_key(&handle, |row| row.id)
            .is_ok();
        let navy = forces.binary_search_by_key(&handle, |row| row.id).ok();
        let gun = guns.binary_search_by_key(&handle, |row| row.gun).is_ok();
        if usize::from(army) + usize::from(navy.is_some()) + usize::from(gun) != 1 {
            return Err(format!(
                "selected strait occupant {handle:#x} has absent or ambiguous current class census"
            ));
        }
        let Some(at) = navy else {
            return Ok(None);
        };
        if forces[at].province != province {
            return Err("selected strait task force has a different province backlink".into());
        }
        Ok(Some(&forces[at]))
    }

    pub fn selected_strait(&self, through: u32, raw_owner: u32) -> Fact<StraitFleet> {
        for &handle in self.province_units(through)? {
            let Some(task_force) = self.common_task_force(handle, through)? else {
                continue;
            };
            let logical_owner = self.slot(task_force.logical_owner)?;
            if !self.common_owner(logical_owner, |owner| owner.at_war(raw_owner))? {
                continue;
            }
            let ships = self.table::<tables::Ships>()?;
            if !ships.iter().any(|ship| ship.task_force == handle)
                || task_force.route_count >= 1
                || task_force.mission != 0
            {
                continue;
            }
            let models = self.table::<tables::ShipModels>()?;
            let mut all_flag_seven = true;
            for ship in ships.iter().filter(|ship| ship.task_force == handle) {
                let at = models
                    .binary_search_by_key(&ship.id, |row| row.ship)
                    .map_err(|_| {
                        format!(
                            "selected strait ship {:#x} has no current model row",
                            ship.id
                        )
                    })?;
                if models[at].formation_flags & 0x80 == 0 {
                    all_flag_seven = false;
                    break;
                }
            }
            if all_flag_seven {
                continue;
            }
            let level = if self.common_discloses_raw(task_force.roster_owner, raw_owner)? {
                3
            } else {
                self.intel()?
                    .get(&(handle, self.slot(raw_owner)?))
                    .copied()
                    .unwrap_or(0)
            };
            return Ok(if level != 0 {
                StraitFleet::Visible
            } else {
                StraitFleet::Unseen
            });
        }
        Ok(StraitFleet::None)
    }

    pub fn for_common(
        world: &'w World<'a>,
        assets: ReadOperands<'w>,
        cells: &'w Cells,
        common: &'w CommonCells<'a>,
    ) -> Fact<Self> {
        if !std::ptr::eq(world.frame().decoded_frame(), common.frame) {
            return Err("selected common movement frame/world association differs".into());
        }
        Ok(Self {
            world,
            assets,
            cells,
            counter: None,
            common: Some(common),
        })
    }

    pub fn common_army_scalars(&self, handle: u64) -> Fact<(Army, u32, u32)> {
        let row = self
            .division(handle)?
            .ok_or("selected army is missing from Divisions")?;
        let movement = self.movement(handle)?;
        Ok((
            Army {
                location: row.province,
                orders_group: row.orders_group,
                retreating: movement.retreat != 0,
                naval_transfer: movement.naval_transfer != 0,
                strength: row.hit_points,
                organisation: row.organisation,
                ..Default::default()
            },
            row.logical_owner,
            movement.combat_count,
        ))
    }

    pub fn common_army_message(
        &self,
        handle: u64,
        target: u32,
        text: &mut String,
        content: &impl CommonMoveText,
    ) -> Fact<bool> {
        let (army, logical_owner_raw, combat_count) = self.common_army_scalars(handle)?;
        let unit = CommonUnit {
            class: CommonClass::Army,
            handle,
            logical_owner_raw,
            combat_count,
            army: &army,
        };
        let graph = self.assets.moves.as_ref().map_err(Clone::clone)?;
        Mover { graph, world: self }.selected_move_message(&unit, target, text, content)
    }
    fn common_land_kind(&self, combat: CommonCombat) -> Fact<()> {
        let rows = self.table::<tables::LandCombatDetails>()?;
        let at = rows
            .binary_search_by_key(&combat.combat, |row| row.combat)
            .map_err(|_| "selected unit reference has no current land class operands")?;
        let border = rows[at]
            .border_war
            .require("LandCombatDetails.border_war")?;
        if combat.kind != u8::from(border.is_some()) {
            return Err(
                "selected unit reference kind differs from current land class operands".into(),
            );
        }
        Ok(())
    }

    pub(super) fn common_gun(
        &self,
        handle: u64,
    ) -> Fact<&'a replay_data::rows::RailwayGunCounterInputRow> {
        let rows = self.table::<tables::RailwayGunCounterInputs>()?;
        let at = rows
            .binary_search_by_key(&handle, |row| row.gun)
            .map_err(|_| "selected railway gun has no current counter row")?;
        Ok(&rows[at])
    }

    fn common_controller_area(&self, province: u32) -> Fact<u32> {
        self.province(province)?
            .controller_area
            .require("Provinces.controller_area")?
            .ok_or_else(|| format!("province {province} controller-area capture refused"))
    }

    fn common_area_node(&self, label: u32) -> Fact<&'a replay_data::rows::ControllerAreaNode> {
        if label == 0 || self.common_controller_area(label)? != label {
            return Err(format!(
                "selected area {label} is not the controller area of its label province"
            ));
        }
        self.province(label)?
            .area_node
            .require("Provinces.area_node")?
            .as_ref()
            .ok_or_else(|| format!("selected area {label} graph capture refused"))
    }

    fn common_area_land_count(&self, label: u32) -> Fact<i32> {
        let counts = get(&self.cells.area_members, || {
            let mut counts = BTreeMap::<u32, i32>::new();
            for (&province, row) in self.world.provinces() {
                let area = row
                    .controller_area
                    .require("Provinces.controller_area")?
                    .ok_or_else(|| {
                        format!("province {province} controller-area capture refused")
                    })?;
                let count = counts.entry(area).or_default();
                *count = count.wrapping_add(1);
            }
            Ok(counts)
        })?;
        counts
            .get(&label)
            .copied()
            .ok_or_else(|| format!("selected area {label} has no member province"))
    }

    fn partial_transport_admission(
        &self,
        unit: transport::Unit,
        source: u32,
        target: u32,
    ) -> Fact<transport::Admission> {
        transport::admit(
            unit,
            source,
            &mut PartialAdmission {
                moves: self,
                unit,
                source,
                target,
            },
        )
        .map_err(str::to_owned)
    }

    fn transport_access(&self, unit: transport::Unit, target: u32) -> Fact<bool> {
        let owner = match unit {
            transport::Unit::Army(handle) => {
                self.division(handle)?
                    .ok_or("selected transport division owner missing")?
                    .logical_owner
            }
            transport::Unit::RailwayGun(handle) => self.common_gun(handle)?.logical_owner,
            transport::Unit::Other => {
                return Err("selected transport access of a non-candidate".into());
            }
        };
        self.common_discloses_access(owner, self.controller_raw(target)?, true)
    }

    pub(super) fn common_gun_scalars(&self, handle: u64) -> Fact<(Army, u32, u32)> {
        let row = self.common_gun(handle)?;
        let army = Army {
            location: row.province,
            retreating: row.retreat != 0,
            naval_transfer: row.naval_transfer != 0,
            organisation: 10_000_000,
            ..Default::default()
        };
        Ok((army, row.logical_owner, row.combat_count))
    }

    pub fn common_railway_message(
        &self,
        handle: u64,
        target: u32,
        text: &mut String,
        content: &impl CommonMoveText,
    ) -> Fact<bool> {
        let (army, logical_owner_raw, combat_count) = self.common_gun_scalars(handle)?;
        let unit = CommonUnit {
            class: CommonClass::RailwayGun,
            handle,
            logical_owner_raw,
            combat_count,
            army: &army,
        };
        let graph = self.assets.moves.as_ref().map_err(Clone::clone)?;
        Mover { graph, world: self }.selected_move_message(&unit, target, text, content)
    }
}

impl CommonMoveWorld for Moves<'_, '_> {
    fn current_subject_access(&self, receiver: Country, target: Country) -> Fact<bool> {
        self.common_subject_access(receiver, target)
    }
    fn canonical_country(&self, raw: u32) -> Fact<Country> {
        self.slot(raw)
    }
    fn actual_country_raw(&self, country: Country) -> Fact<u32> {
        if country == 0 {
            return Ok(0);
        }
        self.world
            .raw_country(country)
            .ok_or_else(|| format!("selected current receiver slot {country} has no Countries row"))
    }
    fn controller_raw(&self, province: u32) -> Fact<u32> {
        Ok(self.province(province)?.controller)
    }
    fn state_owner_raw(&self, province: u32) -> Fact<u32> {
        Ok(self.province(province)?.owner)
    }
    fn province_state(&self, province: u32) -> Fact<Option<u32>> {
        self.state_of(province)
    }
    fn controller_area_equal(&self, from: u32, to: u32) -> Fact<bool> {
        Ok(self.common_controller_area(from)? == self.common_controller_area(to)?)
    }
    fn owner_area_equal(&self, from: u32, to: u32) -> Fact<bool> {
        let area = |province: u32| -> Fact<u32> {
            self.province(province)?
                .owner_area
                .require("Provinces.owner_area")?
                .ok_or_else(|| format!("province {province} owner-area capture refused"))
        };
        Ok(area(from)? == area(to)?)
    }
    fn controller_area(&self, province: u32) -> Fact<u32> {
        self.common_controller_area(province)
    }
    fn area_node(&self, label: u32) -> Fact<content_reader_unit_path::unit_path::AreaNode> {
        let node = self.common_area_node(label)?;
        let first_land = node
            .first_land
            .ok_or_else(|| format!("selected area {label} representative capture refused"));
        let nonsea_index = first_land.clone().and_then(|first| {
            self.assets
                .geometry
                .nonsea_indices
                .get(&first)
                .copied()
                .ok_or_else(|| format!("selected area representative {first} has no nonsea index"))
        });
        Ok(content_reader_unit_path::unit_path::AreaNode {
            label,
            first_land_province: first_land,
            nonsea_index,
            land_count: self.common_area_land_count(label),
            neighbors: node
                .neighbours
                .clone()
                .ok_or_else(|| format!("selected area {label} contact capture refused")),
        })
    }
    fn area_nonsea_count(&self, label: u32) -> Fact<u32> {
        self.common_area_node(label)?;
        u32::try_from(self.assets.geometry.nonsea_indices.len())
            .map_err(|_| "selected area nonsea domain exceeds native u32".into())
    }
    fn unit_exiled_raw(&self, unit: &CommonUnit<'_>) -> Fact<u8> {
        match unit.class {
            CommonClass::Army => Ok(u8::from(
                self.move_flags(unit.handle)? & DivisionStatusRow::MOVE_EXILED != 0,
            )),
            CommonClass::RailwayGun => Ok(self.common_gun(unit.handle)?.exiled),
        }
    }
    fn railway_path_owner_admits(&self, owner: u32, controller: u32, exiled: u8) -> Fact<bool> {
        if exiled != 0 {
            self.common_owner(self.slot(owner)?, |owner| Ok(!owner.at_war(controller)?))
        } else {
            self.common_discloses_access(owner, controller, true)
        }
    }
    fn directed_war(&self, receiver: Country, target: Country) -> Fact<bool> {
        self.common_owner(receiver, |owner| owner.pointer_active_slot(target))
    }
    fn full_unit_combats(&self, unit: &CommonUnit<'_>) -> Fact<Vec<CommonCombat>> {
        let packet = (match unit.class {
            CommonClass::Army => self
                .movement(unit.handle)?
                .combat_references
                .require("DivisionMovement.combat_references")?,
            CommonClass::RailwayGun => self
                .common_gun(unit.handle)?
                .combat_references
                .require("RailwayGunCounterInputs.combat_references")?,
        })
        .as_ref()
        .ok_or("selected unit combat-reference capture refused")?;
        if !packet.valid_at(self.world.frame().hour(), unit.combat_count) {
            return Err(
                "selected unit combat references are stale, malformed or have a different count"
                    .into(),
            );
        }
        Ok(packet
            .entries
            .iter()
            .map(|entry| CommonCombat {
                combat: entry.combat,
                kind: entry.kind,
            })
            .collect())
    }
    fn combat_has_unit(&self, combat: CommonCombat, unit: u64, attacking: bool) -> Fact<bool> {
        match combat.kind {
            0 | 1 => {
                self.common_land_kind(combat)?;
                let rows = self.table::<tables::LandCombats>()?;
                let at = rows
                    .binary_search_by_key(&combat.combat, |row| row.id)
                    .map_err(|_| "selected unit references a missing land/border combat")?;
                let row = &rows[at];
                Ok((if attacking {
                    &row.attackers
                } else {
                    &row.defenders
                })
                .contains(&unit))
            }
            2 => {
                let rows = self.table::<tables::NavalCombatDetails>()?;
                let at = rows
                    .binary_search_by_key(&combat.combat, |row| row.combat)
                    .map_err(|_| "selected unit references a missing naval combat side")?;
                Ok(rows[at].sides[usize::from(!attacking)]
                    .units
                    .contains(&unit))
            }
            _ => Err("selected unit combat reference has an unsupported native class".into()),
        }
    }
    fn combat_minimum_duration(&self, combat: CommonCombat) -> Fact<bool> {
        match combat.kind {
            0 | 1 => {
                self.common_land_kind(combat)?;
                let rows = self.table::<tables::LandCombats>()?;
                let at = rows
                    .binary_search_by_key(&combat.combat, |row| row.id)
                    .map_err(|_| "selected unit references a missing ordinary land combat")?;
                Ok(self.assets.combat_minimum_time <= rows[at].duration)
            }
            2 => {
                let details = rows::<tables::NavalCombatDetails>(self.world.frame())
                    .map_err(|error| error.to_string())?;
                let rows = match details {
                    Available::Complete(rows) | Available::Truncated { rows, .. } => rows,
                    Available::Unavailable { reason } => {
                        return Err(format!("selected NavalCombatDetails unavailable: {reason}"));
                    }
                    Available::NotRecorded { mode } => {
                        return Err(format!(
                            "selected NavalCombatDetails is not recorded in a {mode} recording"
                        ));
                    }
                };
                let at = rows
                    .binary_search_by_key(&combat.combat, |row| row.combat)
                    .map_err(|_| "selected unit references a missing naval combat duration")?;
                let minimum = self
                    .assets
                    .naval_combat_min_duration
                    .as_ref()
                    .map_err(Clone::clone)?;
                Ok(*minimum <= rows[at].duration)
            }
            _ => Err(format!(
                "selected combat duration check for authenticated kind {} is unsupported",
                combat.kind
            )),
        }
    }
    fn unit_route_first(&self, unit: &CommonUnit<'_>) -> Fact<Option<u32>> {
        match unit.class {
            CommonClass::Army => {
                let row = self.movement(unit.handle)?;
                if (row.path_total as i32) <= 0 {
                    return Ok(None);
                }
                row.path
                    .first()
                    .copied()
                    .map(Some)
                    .ok_or_else(|| "selected army first ordinary province is unavailable".into())
            }
            CommonClass::RailwayGun => {
                Ok(self.common_gun(unit.handle)?.ordinary_path.first().copied())
            }
        }
    }
    fn primary_path_combat(&self, unit: &CommonUnit<'_>, province: u32) -> Fact<bool> {
        if (unit.combat_count as i32) <= 0 {
            return Ok(false);
        }
        for combat in self.full_unit_combats(unit)? {
            let location = match combat.kind {
                0 | 1 => {
                    let rows = self.table::<tables::LandCombats>()?;
                    rows.binary_search_by_key(&combat.combat, |row| row.id)
                        .ok()
                        .map(|at| rows[at].province)
                }
                2 => {
                    let rows = self.table::<tables::NavalCombats>()?;
                    rows.binary_search_by_key(&combat.combat, |row| row.id)
                        .ok()
                        .map(|at| rows[at].province)
                }
                _ => return Err("selected primary-path combat class unsupported".into()),
            }
            .ok_or("selected primary-path combat location missing")?;
            if location == province {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn province_units(&self, province: u32) -> Fact<&[u64]> {
        let rows = self.table::<tables::ProvinceUnitOrder>()?;
        let at = rows
            .binary_search_by_key(&province, |row| row.province)
            .map_err(|_| "selected province has no complete occupant-order row")?;
        Ok(&rows[at].mixed)
    }
    fn army_occupant(&self, handle: u64, province: u32) -> Fact<Option<NativeArmyOccupant>> {
        let divisions = self.table::<tables::Divisions>()?;
        let forces = self.table::<tables::TaskForces>()?;
        let guns = self.table::<tables::RailwayGunCounterInputs>()?;
        let army = divisions.binary_search_by_key(&handle, |row| row.id).ok();
        let navy = forces.binary_search_by_key(&handle, |row| row.id).is_ok();
        let gun = guns.binary_search_by_key(&handle, |row| row.gun).is_ok();
        if usize::from(army.is_some()) + usize::from(navy) + usize::from(gun) != 1 {
            return Err(format!(
                "selected occupant {handle:#x} has absent or ambiguous current class census"
            ));
        }
        if let Some(at) = army {
            if divisions[at].province != province {
                return Err("selected army occupant has a different province backlink".into());
            }
            Ok(Some(NativeArmyOccupant {
                handle,
                logical_owner_raw: divisions[at].logical_owner,
            }))
        } else {
            Ok(None)
        }
    }
    fn occupant_retreating(&self, handle: u64) -> Fact<bool> {
        Ok(self.movement(handle)?.retreat != 0)
    }
    fn occupant_strength(&self, handle: u64) -> Fact<i64> {
        Ok(self
            .division(handle)?
            .ok_or("selected class0 occupant is missing")?
            .hit_points)
    }
    fn unit_orders_group(&self, unit: &CommonUnit<'_>) -> Fact<u64> {
        if unit.class == CommonClass::Army {
            return Ok(self
                .division(unit.handle)?
                .ok_or("selected army orders group missing")?
                .orders_group);
        }
        let packet = self
            .common_gun(unit.handle)?
            .fresh
            .require("RailwayGunCounterInputs.fresh")?
            .as_ref()
            .ok_or("selected railway orders-group capture refused")?;
        if !packet.sample_hour.fresh_at(self.world.frame().hour()) {
            return Err("selected railway orders-group sample is stale".into());
        }
        packet
            .orders_group
            .ok_or_else(|| "selected railway orders-group lookup refused".into())
    }
    fn ordered_enemy_raws(&self, country: Country) -> Fact<Vec<u32>> {
        let raw = self.actual_country_raw(country)?;
        let rows = self.table::<tables::CountryWars>()?;
        let at = rows
            .binary_search_by_key(&raw, |row| row.country)
            .map_err(|_| "selected receiver has no current ordered diplomacy enemy row")?;
        let packet = rows[at]
            .ordered
            .require("CountryWars.ordered")?
            .as_ref()
            .ok_or("selected ordered diplomacy enemy capture refused")?;
        if !packet.sample_hour.fresh_at(self.world.frame().hour()) {
            return Err("selected ordered diplomacy enemy sample is stale".into());
        }
        Ok(packet.entries.clone())
    }
    fn move_strength_threshold(&self) -> Fact<i64> {
        self.runtime_scalars()?
            .move_strength_threshold
            .ok_or_else(|| "selected move_strength_threshold capture refused".into())
    }
    fn move_organisation_threshold(&self) -> Fact<i64> {
        self.runtime_scalars()?
            .move_organisation_threshold
            .ok_or_else(|| "selected move_organisation_threshold capture refused".into())
    }
    fn maximum_organisation(&self, unit: &CommonUnit<'_>) -> Fact<i64> {
        let rows = self.table::<tables::DivisionPanelStats>()?;
        let at = rows
            .binary_search_by_key(&unit.handle, |row| row.division)
            .map_err(|_| "selected army maximum organisation is missing")?;
        rows[at]
            .maximum_organisation()
            .ok_or_else(|| "selected army maximum organisation was refused when recorded".into())
    }
    fn unit_exiled(&self, unit: &CommonUnit<'_>) -> Fact<bool> {
        Ok(self.unit_exiled_raw(unit)? != 0)
    }
    fn unit_exfiltrates(&self, unit: &CommonUnit<'_>) -> Fact<bool> {
        match unit.class {
            CommonClass::Army => Ok(self.move_flags(unit.handle)?
                & DivisionStatusRow::MOVE_CAN_EXFILTRATE_FROM_COAST
                != 0),
            CommonClass::RailwayGun => {
                Err("selected railway real-template coast-exfiltration receiver unproved".into())
            }
        }
    }
    fn unit_retreat_ban(&self, unit: &CommonUnit<'_>, attacking: bool) -> Fact<bool> {
        if unit.class == CommonClass::RailwayGun {
            return Ok(false);
        }
        let ban = if attacking {
            DivisionRetreatBanRow::ATTACKING
        } else {
            DivisionRetreatBanRow::DEFENDING
        };
        Ok(self.retreat_bans(unit.handle)? & ban != 0)
    }
    fn railway_retreat_edge(&self, source: u32, target: u32) -> Fact<bool> {
        let graph = self.assets.moves.as_ref().map_err(Clone::clone)?;
        let Some(index) = graph.selected_neighbour_index(source, target)? else {
            return Ok(false);
        };
        let Some((row, inputs)) = self.selected_railway(source)? else {
            return Ok(false);
        };
        let level = inputs
            .levels
            .as_ref()
            .ok_or("selected railway indexed levels capture refused")?
            .get(index)
            .ok_or("selected railway indexed level missing for content neighbour")?;
        if *level <= 0 || row.cooldown_ticks != 0 {
            return Ok(false);
        }
        Ok(inputs
            .building_level
            .ok_or("selected railway building level capture refused")?
            .is_some_and(|level| level > 0))
    }
    fn combat_attacker_at(&self, combat: CommonCombat, target: u32) -> Fact<bool> {
        self.common_land_kind(combat)?;
        let rows = self.table::<tables::LandCombats>()?;
        let at = rows
            .binary_search_by_key(&combat.combat, |row| row.id)
            .map_err(|_| "selected counterattack combat side missing")?;
        let mut unknown = None;
        for location in self.attacker_locations(&rows[at])? {
            match location {
                Ok(location) if location == target => return Ok(true),
                Ok(_) => {}
                Err(reason) => {
                    unknown.get_or_insert(reason);
                }
            }
        }
        unknown.map_or(Ok(false), Err)
    }
    fn selected_path(
        &self,
        unit: &CommonUnit<'_>,
        target: u32,
        _: &mut String,
        _: &impl CommonMoveText,
    ) -> Fact<bool> {
        let graph = self.assets.moves.as_ref().map_err(Clone::clone)?;
        let mover = Mover { graph, world: self };
        match unit.class {
            CommonClass::Army => mover.selected_army_path(unit, target),
            CommonClass::RailwayGun => mover.selected_railway_path(unit, target),
        }
    }
    fn selected_transport(
        &self,
        unit: &CommonUnit<'_>,
        target: u32,
        text: &mut String,
        content: &impl CommonMoveText,
    ) -> Fact<bool> {
        let candidate = match unit.class {
            CommonClass::Army => transport::Unit::Army(unit.handle),
            CommonClass::RailwayGun => transport::Unit::RailwayGun(unit.handle),
        };
        let admission = self.partial_transport_admission(candidate, unit.army.location, target)?;
        if let Some(reason) = admission.reason {
            text.push_str(&match reason {
                transport::Reason::NotEnough {
                    available,
                    required,
                } => content.transport_stock(available, required)?,
                reason => content.plain(reason.key())?,
            });
        }
        Ok(admission.allowed)
    }
    fn railway_cooldown(&self, province: u32) -> Fact<Option<i32>> {
        let rows = self.table::<tables::RailCooldowns>()?;
        Ok(rows
            .binary_search_by_key(&province, |row| row.province)
            .ok()
            .map(|at| rows[at].cooldown_ticks))
    }
    fn railway_accessible(&self, province: u32) -> Fact<bool> {
        let Some((row, inputs)) = self.selected_railway(province)? else {
            return Ok(false);
        };
        if inputs
            .pair_count
            .ok_or("selected railway raw pair count capture refused")?
            == 0
            || row.cooldown_ticks != 0
        {
            return Ok(false);
        }
        Ok(inputs
            .building_level
            .ok_or("selected railway building level capture refused")?
            .is_some_and(|level| level > 0))
    }
}

pub struct CommonText<'a> {
    pub frame: &'a DecodedFrame,
    pub loc: &'a content_reader_localisation::localisation::Localisation,
    pub recorded_language: &'a str,
    pub state_keys: &'a BTreeMap<u32, Fact<String>>,
}

impl CommonMoveText for CommonText<'_> {
    fn transport_stock(&self, available: i32, required: i32) -> Fact<String> {
        localise(
            self.loc,
            "TRANSPORT_NOT_ENOUGH",
            &[
                (
                    "FREE",
                    content_reader_localisation::localisation::Argument::Int32(available),
                ),
                (
                    "NEED",
                    content_reader_localisation::localisation::Argument::Int32(required),
                ),
            ],
        )
    }
    fn transport_risk(&self, risk: i64) -> Fact<String> {
        localise(
            self.loc,
            "NAVAL_TRANSPORT_INTERCEPT_WARNING",
            &[(
                "ATTR",
                content_reader_localisation::localisation::Argument::Fixed5(risk),
            )],
        )
    }
    fn transport_description(&self, convoys: i32, icon: bool) -> Fact<String> {
        localise(
            self.loc,
            if icon {
                "NAVAL_TRANSPORT_MOVE_DESC_ICON"
            } else {
                "NAVAL_TRANSPORT_MOVE_DESC"
            },
            &[(
                "CONVOYS",
                content_reader_localisation::localisation::Argument::Int32(convoys),
            )],
        )
    }
    fn days(&self, key: &str, days: i32) -> Fact<String> {
        localise(
            self.loc,
            key,
            &[(
                "DAYS",
                content_reader_localisation::localisation::Argument::Int32(days),
            )],
        )
    }
    fn plain(&self, key: &str) -> Fact<String> {
        localise(self.loc, key, &[])
    }
    fn state(&self, key: &str, state: u32) -> Fact<String> {
        let rows = self
            .frame
            .complete::<tables::States>()
            .map_err(|e| e.to_string())?;
        let at = rows
            .binary_search_by_key(&state, |row| row.id)
            .map_err(|_| "selected warning state is missing from States")?;
        let row = &rows[at];
        let name = state_name(
            self.loc,
            self.recorded_language,
            state,
            &row.custom_name,
            &row.owner_name,
            || {
                let key = self
                    .state_keys
                    .get(&state)
                    .ok_or("selected state has no content name key")?
                    .as_ref()
                    .map_err(Clone::clone)?;
                self.plain(key)
            },
        )?;
        localise(
            self.loc,
            key,
            &[(
                "STATE",
                content_reader_localisation::localisation::Argument::Text(&name),
            )],
        )
    }
    fn owner(&self, key: &str, raw_owner: u32) -> Fact<String> {
        let slot = if raw_owner == 0 {
            0
        } else {
            let rows = self
                .frame
                .complete::<tables::CountryLinks>()
                .map_err(|e| e.to_string())?;
            let at = rows
                .binary_search_by_key(&raw_owner, |row| row.id)
                .map_err(|_| "selected warning owner has no CountryLinks row")?;
            rows[at].canonical_slot
        };
        let name = content_reader_country_names::country_names::CountryNames::read_selected(
            self.frame,
            self.loc,
            [slot],
        )
        .name(slot)?;
        localise(
            self.loc,
            key,
            &[(
                "OWNER",
                content_reader_localisation::localisation::Argument::Text(&name),
            )],
        )
    }
    fn rule(
        &self,
        key: &str,
        rule: &content_reader_map::adjacency_rules::AdjacencyRule,
    ) -> Fact<String> {
        let name = self.plain(&rule.name)?;
        localise(
            self.loc,
            key,
            &[(
                "RULE_NAME",
                content_reader_localisation::localisation::Argument::Text(&name),
            )],
        )
    }
    fn warning(&self, key: &str, raw: i64) -> Fact<String> {
        let value = content_reader_parser::fixed5::fixed5_to_string(raw, 0);
        localise(
            self.loc,
            key,
            &[(
                "VAL",
                content_reader_localisation::localisation::Argument::Text(&value),
            )],
        )
    }
}

struct PartialTransport<'m, 'w, 'a> {
    moves: &'m Moves<'w, 'a>,
}

impl transport::MessageInputs for PartialTransport<'_, '_, '_> {
    fn target_base(&mut self, target: u32) -> Fact<bool> {
        self.moves.has_naval_base(target)
    }
    fn source(&mut self, unit: transport::Unit) -> Fact<u32> {
        match unit {
            transport::Unit::Army(handle) => Ok(self
                .moves
                .division(handle)?
                .ok_or("selected transport division actual location missing")?
                .province),
            transport::Unit::RailwayGun(handle) => Ok(self.moves.common_gun(handle)?.province),
            transport::Unit::Other => Err("selected transport source of a non-candidate".into()),
        }
    }
    fn same_area(&mut self, source: u32, target: u32) -> Fact<bool> {
        self.moves.controller_area_equal(source, target)
    }
    fn source_base(&mut self, source: u32) -> Fact<bool> {
        self.moves.has_naval_base(source)
    }
    fn admission(
        &mut self,
        unit: transport::Unit,
        source: u32,
        target: u32,
    ) -> Fact<transport::Admission> {
        self.moves.partial_transport_admission(unit, source, target)
    }
    fn weight(&mut self, _: transport::Unit) -> Fact<i64> {
        Err(TRANSPORT_RETIRED.into())
    }
    fn risk(&mut self, _: transport::Unit, _: u32, _: u32, _: i32) -> Fact<i64> {
        Err(TRANSPORT_RETIRED.into())
    }
}

struct PartialAdmission<'m, 'w, 'a> {
    moves: &'m Moves<'w, 'a>,
    unit: transport::Unit,
    source: u32,
    target: u32,
}

impl transport::AdmissionInputs for PartialAdmission<'_, '_, '_> {
    fn target_base(&mut self) -> transport::Fact<bool> {
        self.moves
            .has_naval_base(self.target)
            .map_err(|_| "selected transport target naval-base census unavailable")
    }
    fn source_land(&mut self) -> transport::Fact<bool> {
        self.moves
            .assets
            .moves
            .as_ref()
            .map_err(|_| "selected transport source content graph unavailable")?
            .selected_land(self.source)
            .map_err(|_| "selected transport source province/terrain unavailable")
    }
    fn source_base(&mut self) -> transport::Fact<bool> {
        self.moves
            .has_naval_base(self.source)
            .map_err(|_| "selected transport source naval-base census unavailable")
    }
    fn coastal_special_template(&mut self) -> transport::Fact<bool> {
        Err(TRANSPORT_RETIRED)
    }
    fn transfer(&mut self) -> transport::Fact<Option<transport::Transfer>> {
        Err(TRANSPORT_RETIRED)
    }
    fn weight(&mut self) -> transport::Fact<i64> {
        Err(TRANSPORT_RETIRED)
    }
    fn available_convoys(&mut self) -> transport::Fact<i32> {
        Err(TRANSPORT_RETIRED)
    }
    fn military_access(&mut self) -> transport::Fact<bool> {
        self.moves
            .transport_access(self.unit, self.target)
            .map_err(|_| "selected transport current receiver access refused")
    }
    fn sea_route(&mut self) -> transport::Fact<bool> {
        Err(TRANSPORT_RETIRED)
    }
}

#[cfg(test)]
#[path = "selected_common/tests.rs"]
mod tests;
