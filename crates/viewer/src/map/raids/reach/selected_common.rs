//! Private selected common movement adapter. Existing recorded occupants,
//! raw country links, maximum organisation and naming services are reused.
//! Missing body/acquisition dependencies stay named refusals at first demand.

use super::*;
use crate::map::tables::{Available, rows};
use installed_content::unit_path::{
    CommonClass, CommonCombat, CommonMoveText, CommonMoveWorld, CommonUnit, NativeArmyOccupant,
};
use replay::frame::DecodedFrame;

/// Exactly one borrowed frame/request lifetime. One directed Owner per
/// receiver admits its immutable hour context once; no carried query cache.
pub(crate) struct CommonCells<'a> {
    frame: &'a DecodedFrame,
    wars: Mutex<BTreeMap<Country, crate::map::directed_wars::Owner<'a>>>,
    retreat_layout: OnceLock<Fact<()>>,
}

impl<'a> CommonCells<'a> {
    pub(crate) fn new(frame: &'a DecodedFrame) -> Self {
        Self {
            frame,
            wars: Default::default(),
            retreat_layout: Default::default(),
        }
    }
}

impl<'w, 'a> Moves<'w, 'a> {
    fn selected_railway(
        &self,
        province: u32,
    ) -> Fact<
        Option<(
            &'a replay::rows::RailCooldownRow,
            &'a replay::rows::RailwaySelectionInputs,
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
            != self.world.frame().hour
        {
            return Err("selected railway input sample is stale".into());
        }
        Ok(Some((row, inputs)))
    }

    /// Use actual-location mode. A non-base target or a selection already at
    /// the target has no transport fragment and demands no risk producer.
    pub(crate) fn selected_transport_message(
        &self,
        selected: &[crate::map::unit_selection::Unit],
        target: u32,
        content: &impl CommonMoveText,
    ) -> Fact<installed_content::unit_path::selected_transport::Outputs> {
        self.transport_message(selected, target, content)
    }

    fn common_owner<T>(
        &self,
        receiver: Country,
        query: impl FnOnce(&crate::map::directed_wars::Owner<'a>) -> Fact<T>,
    ) -> Fact<T> {
        let common = self.common.ok_or("selected common frame context missing")?;
        let mut owners = common
            .wars
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let std::collections::btree_map::Entry::Vacant(entry) = owners.entry(receiver) {
            entry.insert(crate::map::directed_wars::Owner::new(
                common.frame,
                self.actual_country_raw(receiver)?,
            )?);
        }
        query(&owners[&receiver])
    }

    fn common_discloses_raw(&self, receiver_raw: u32, target_raw: u32) -> Fact<bool> {
        self.common_discloses_access(receiver_raw, target_raw, false)
    }

    pub(super) fn common_transport_access(&self, receiver_raw: u32, target_raw: u32) -> Fact<bool> {
        self.common_discloses_access(receiver_raw, target_raw, true)
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
        // Preserve the recorded zero-cell observation and its failure domains.
        // Directed war from another query cannot repair refused forward visibility
        // or erase historical field absence.
        if receiver == 0 || target == 0 {
            return crate::map::visibility::event_shared(
                self.world.frame(),
                &self.world.links,
                receiver_raw,
                target_raw,
                access,
            )
            .map_err(str::to_owned);
        }
        // Read the relation and war bytes directly, without a signed-positive
        // receiver gate. Transport uses mode 1/flag 0: hourly RelationLinks supplies
        // the sparse pointer census before faction/subject checks. Keep this access policy.
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
        let hour = self.world.frame().hour;
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
            if packet.sample_hour != hour {
                return Err("transport actual-zero subject access stale".into());
            }
            let cell = packet
                .cells
                .get(target as usize)
                .and_then(Option::as_ref)
                .ok_or("transport actual-zero subject cell refused")?;
            return replay::transport_subject_access::matches(
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
            return replay::transport_subject_access::matches(
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
            replay::transport_subject_access::current(rows, actual, receiver, target, hour)?
        else {
            return Ok(false);
        };
        replay::transport_subject_access::matches(
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

    /// Bind both strait predicates to this frame's directed owner, preserving
    /// the caller's raw target. Ordinary raid strait_fleet keeps its own adapter.
    pub(crate) fn selected_strait(&self, through: u32, raw_owner: u32) -> Fact<StraitFleet> {
        for &handle in self.province_units(through)? {
            let Some(task_force) = self.common_task_force(handle, through)? else {
                continue;
            };
            let logical_owner = self.slot(task_force.logical_owner)?;
            if !self.common_owner(logical_owner, |owner| owner.at_war(raw_owner))? {
                continue;
            }
            // Complete Ships is the native task-force ship census: no matching
            // entries proves a zero-length roster, unlike a missing task-force
            // or province-order row. Unrelated model rows are not demanded.
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
                // Return at the first clear flag, then read visibility without reaching
                // later ship models.
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
                // Complete UnitIntel retains every per-viewer entry; an absent pair
                // contributes zero.
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

    /// The caller owns CommonCells only while this exact World/frame lives.
    /// Native selection remains a caller-local list and never enters capture.
    pub(crate) fn for_common(
        world: &'w World<'a>,
        assets: &'w RaidAssets,
        cells: &'w Cells,
        common: &'w CommonCells<'a>,
    ) -> Fact<Self> {
        if !std::ptr::eq(world.frame(), common.frame) {
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

    /// Do not call Moves::army: that eager raid adapter reconstructs land combats
    /// and demands commander bans before the common access short circuit.
    pub(crate) fn common_army_scalars(&self, handle: u64) -> Fact<(Army, u32, u32)> {
        let row = self
            .division(handle)?
            .ok_or("selected army is missing from Divisions")?;
        let movement = self.movement(handle)?;
        Ok((
            Army {
                location: row.province,
                // Canonical owner and route-first are demanded only by their
                // reached native branches. No strategic-navy country is used here.
                orders_group: row.orders_group,
                retreating: movement.retreat != 0,
                naval_transfer: movement.naval_transfer != 0,
                strength: row.hit_points,
                organisation: row.organisation,
                // A full-combat adapter must also bind current native commander
                // bans. Daily DivisionRetreatBans cannot certify same-hour changes.
                ..Default::default()
            },
            row.logical_owner,
            movement.combat_count,
        ))
    }

    pub(crate) fn common_army_message(
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
    fn common_area(&self, province: u32) -> Fact<&'a replay::rows::ProvinceAreaInputs> {
        let packet = self
            .province(province)?
            .selected_areas
            .require("Provinces.selected_areas")?
            .as_ref()
            .ok_or("selected province area acquisition refused")?;
        if packet.sample_hour != self.world.frame().hour {
            return Err("selected province area identities are stale".into());
        }
        Ok(packet)
    }
    fn common_area_node(&self, label: u32) -> Fact<&'a replay::rows::SelectedAreaNode> {
        let packet = self.common_area(label)?;
        if label == 0 || packet.controller != label {
            return Err("selected area dictionary root identity differs".into());
        }
        packet
            .node
            .as_ref()
            .ok_or_else(|| "selected area graph acquisition refused".into())
    }
    fn common_selected_unit(&self, handle: u64) -> Fact<&'a replay::rows::SelectedUnitInputs> {
        let packet = self
            .movement(handle)?
            .selected
            .require("DivisionMovement.selected")?
            .as_ref()
            .ok_or("selected army route/commander inputs capture refused")?;
        if packet.sample_hour != self.world.frame().hour {
            return Err("selected army route/commander inputs are stale".into());
        }
        Ok(packet)
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
    ) -> Fact<&'a replay::rows::RailwayGunCounterInputRow> {
        let rows = self.table::<tables::RailwayGunCounterInputs>()?;
        let at = rows
            .binary_search_by_key(&handle, |row| row.gun)
            .map_err(|_| "selected railway gun has no current counter row")?;
        Ok(&rows[at])
    }

    pub(crate) fn common_railway_message(
        &self,
        handle: u64,
        target: u32,
        text: &mut String,
        content: &impl CommonMoveText,
    ) -> Fact<bool> {
        let row = self.common_gun(handle)?;
        let army = Army {
            location: row.province,
            retreating: row.retreat != 0,
            naval_transfer: row.naval_transfer != 0,
            // Railway organization is this fixed5 constant, independent of the army
            // scalar getter.
            organisation: 10_000_000,
            ..Default::default()
        };
        let unit = CommonUnit {
            class: CommonClass::RailwayGun,
            handle,
            logical_owner_raw: row.logical_owner,
            combat_count: row.combat_count,
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
        Ok(self.common_area(from)?.controller == self.common_area(to)?.controller)
    }
    fn owner_area_equal(&self, from: u32, to: u32) -> Fact<bool> {
        let first = self
            .common_area(from)?
            .owner
            .ok_or("selected source area-owner capture refused")?;
        let second = self
            .common_area(to)?
            .owner
            .ok_or("selected target area-owner capture refused")?;
        Ok(first == second)
    }
    fn controller_area(&self, province: u32) -> Fact<u32> {
        Ok(self.common_area(province)?.controller)
    }
    fn area_node(&self, label: u32) -> Fact<installed_content::unit_path::AreaNode> {
        let node = self.common_area_node(label)?;
        Ok(installed_content::unit_path::AreaNode {
            label,
            first_land_province: node
                .first_land
                .ok_or_else(|| "selected area first-land identity refused".into()),
            nonsea_index: node
                .nonsea_index
                .ok_or_else(|| "selected area nonsea index refused".into()),
            land_count: node
                .land_count
                .ok_or_else(|| "selected area signed land count refused".into()),
            neighbors: node
                .neighbours
                .clone()
                .ok_or_else(|| "selected area ordered adjacency capture refused".into()),
        })
    }
    fn area_nonsea_count(&self, label: u32) -> Fact<u32> {
        // The recorded area count bounds temporary area storage. Visited identities
        // use the map's area domain.
        self.common_area_node(label)?
            .nonsea_count
            .ok_or("selected area temporary domain refused")?;
        u32::try_from(self.assets.geometry.nonsea_indices.len())
            .map_err(|_| "selected area nonsea domain exceeds native u32".into())
    }
    fn unit_exiled_raw(&self, unit: &CommonUnit<'_>) -> Fact<u8> {
        match unit.class {
            CommonClass::Army => self
                .common_selected_unit(unit.handle)?
                .exiled
                .ok_or_else(|| "selected army raw exile capture refused".into()),
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
        if !packet.valid_at(self.world.frame().hour, unit.combat_count) {
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
                // Existing full sides, including participants in neither the
                // committed nor reserve arrays. Never use combat_role here.
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
            // Border-war combat uses the land minimum-duration rule.
            0 | 1 => {
                self.common_land_kind(combat)?;
                let rows = self.table::<tables::LandCombats>()?;
                let at = rows
                    .binary_search_by_key(&combat.combat, |row| row.id)
                    .map_err(|_| "selected unit references a missing ordinary land combat")?;
                Ok(self.assets.combat_minimum_time <= rows[at].duration)
            }
            2 => {
                // Compare signed elapsed combat duration with NNavy.COMBAT_MIN_DURATION.
                // The hourly details walk already records that operand.
                let details = rows::<tables::NavalCombatDetails>(self.world.frame())
                    .map_err(|error| error.to_string())?;
                let rows = match details {
                    Available::Complete(rows) | Available::Truncated { rows, .. } => rows,
                    Available::Unavailable { reason } => {
                        return Err(format!("selected NavalCombatDetails unavailable: {reason}"));
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
        // Read combat location in the unit's full reference order.
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
        // Authenticated class censuses, not reference-type constants. This
        // per-handle lookup happens only as far as the native caller scans.
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
        if packet.sample_hour != self.world.frame().hour {
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
        if packet.sample_hour != self.world.frame().hour {
            return Err("selected ordered diplomacy enemy sample is stale".into());
        }
        Ok(packet.entries.clone())
    }
    fn move_strength_threshold(&self) -> Fact<i64> {
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
        if values.sample_hour != common.frame.hour {
            return Err("selected movement threshold sample is stale".into());
        }
        values
            .move_strength_threshold
            .ok_or_else(|| "selected move_strength_threshold capture refused".into())
    }
    fn move_organisation_threshold(&self) -> Fact<i64> {
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
        if values.sample_hour != common.frame.hour {
            return Err("selected movement threshold sample is stale".into());
        }
        values
            .move_organisation_threshold
            .ok_or_else(|| "selected move_organisation_threshold capture refused".into())
    }
    fn maximum_organisation(&self, unit: &CommonUnit<'_>) -> Fact<i64> {
        let rows = self.table::<tables::DivisionStatus>()?;
        let at = rows
            .binary_search_by_key(&unit.handle, |row| row.division)
            .map_err(|_| "selected army maximum organisation is missing")?;
        Ok(rows[at].maximum_organisation)
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
        // Railway returns zero without reading group or leader.
        if unit.class == CommonClass::RailwayGun {
            return Ok(false);
        }
        let packet = self.common_selected_unit(unit.handle)?;
        if !packet
            .commander
            .ok_or("selected commander stat-context capture refused")?
        {
            return Ok(false);
        }
        let common = self.common.ok_or("selected common frame context missing")?;
        common
            .retreat_layout
            .get_or_init(|| {
                replay::fixed_layout::validate_recording(
                    common.frame,
                    replay::fixed_layout::RETREAT_BANS,
                )
            })
            .as_ref()
            .map_err(Clone::clone)?;
        packet.retreat[usize::from(attacking)]
            .map(|value| value > 0)
            .ok_or_else(|| {
                format!(
                    "selected same-hour {} retreat modifier capture refused",
                    if attacking { "attacking" } else { "defending" }
                )
            })
    }
    fn railway_retreat_edge(&self, source: u32, target: u32) -> Fact<bool> {
        let graph = self.assets.moves.as_ref().map_err(Clone::clone)?;
        let Some(index) = graph.selected_neighbour_index(source, target)? else {
            return Ok(false);
        };
        let Some((row, inputs)) = self.selected_railway(source)? else {
            return Ok(false);
        };
        // Require positive indexed level, zero cooldown, then positive building
        // maximum and health. Read neither sorted pairs nor target rail state.
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
        for &handle in &rows[at].attackers {
            // The LandCombats census authenticates the army class; another
            // class with the same numeric handle cannot supply its location.
            let row = self
                .division(handle)?
                .ok_or("selected counterattack army location missing")?;
            if row.province == target {
                return Ok(true);
            }
        }
        Ok(false)
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
        let admission =
            self.transport_admission(unit.class, unit.handle, unit.army.location, target)?;
        self.transport_reason(admission, text, content)
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
        // Test cooldown against zero, including signed-negative values. Do not
        // read pairs, indexed level or building health, or approximate by sign.
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

/// Reached warning arguments resolve through existing content stack services.
/// No warning key, country-name ladder or state-name ladder is content-guessed.
pub(crate) struct CommonText<'a> {
    pub(crate) frame: &'a DecodedFrame,
    pub(crate) loc: &'a installed_content::localisation::Localisation,
    pub(crate) recorded_language: &'a str,
    pub(crate) state_keys: &'a BTreeMap<u32, Fact<String>>,
}

impl CommonMoveText for CommonText<'_> {
    fn transport_stock(&self, available: i32, required: i32) -> Fact<String> {
        crate::combat::text::localise(
            self.loc,
            "TRANSPORT_NOT_ENOUGH",
            &[
                ("FREE", installed_content::localisation::Argument::Int32(available)),
                ("NEED", installed_content::localisation::Argument::Int32(required)),
            ],
        )
    }
    fn transport_risk(&self, risk: i64) -> Fact<String> {
        crate::combat::text::localise(
            self.loc,
            "NAVAL_TRANSPORT_INTERCEPT_WARNING",
            &[("ATTR", installed_content::localisation::Argument::Fixed5(risk))],
        )
    }
    fn transport_description(&self, convoys: i32, icon: bool) -> Fact<String> {
        crate::combat::text::localise(
            self.loc,
            if icon {
                "NAVAL_TRANSPORT_MOVE_DESC_ICON"
            } else {
                "NAVAL_TRANSPORT_MOVE_DESC"
            },
            &[("CONVOYS", installed_content::localisation::Argument::Int32(convoys))],
        )
    }
    fn days(&self, key: &str, days: i32) -> Fact<String> {
        crate::combat::text::localise(
            self.loc,
            key,
            &[("DAYS", installed_content::localisation::Argument::Int32(days))],
        )
    }
    fn plain(&self, key: &str) -> Fact<String> {
        crate::combat::text::localise(self.loc, key, &[])
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
        let name = crate::combat::text::state_name(
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
        crate::combat::text::localise(
            self.loc,
            key,
            &[("STATE", installed_content::localisation::Argument::Text(&name))],
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
        let name =
            installed_content::country_names::CountryNames::read_selected(self.frame, self.loc, [slot])
                .name(slot)?;
        crate::combat::text::localise(
            self.loc,
            key,
            &[("OWNER", installed_content::localisation::Argument::Text(&name))],
        )
    }
    fn rule(&self, key: &str, rule: &installed_content::adjacency_rules::AdjacencyRule) -> Fact<String> {
        // Use the reached adjacency rule's stored name key for localization.
        let name = self.plain(&rule.name)?;
        crate::combat::text::localise(
            self.loc,
            key,
            &[("RULE_NAME", installed_content::localisation::Argument::Text(&name))],
        )
    }
    fn warning(&self, key: &str, raw: i64) -> Fact<String> {
        // Bind `VAL` including its terminating NUL with precision zero.
        let value = installed_content::fixed5::fixed5_to_string(raw, 0);
        crate::combat::text::localise(
            self.loc,
            key,
            &[("VAL", installed_content::localisation::Argument::Text(&value))],
        )
    }
}

#[cfg(test)]
#[path = "selected_common/tests.rs"]
mod tests;
