//! Native mission-effective aircraft count of the flying map scenarios:
//! Sortie efficiency is applied to the recorded wing operands.
//! Products keep the native low 64 bits and every fixed5 division truncates
//! toward zero before the next factor. A console efficiency override
//! recorded hourly replaces the efficiency law.
//!
//! The simulation's stored mission efficiency and aircraft count
//! are not this value: they refresh only past period, capacity and host-combat
//! gates during the previous hourly update, with the base's carrier status.

use std::{collections::BTreeMap, io};

use installed_content_core::defines::Defines;
use replay_data::{
    rows::AirBaseRow, rows::AirMissionInputRow, rows::AirSortieBaseCountryRow,
    rows::AirSortieModifierRow, rows::AirSortieSpecialBaseRow, rows::AirWingRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;

use crate::map::tables::{Available, rows};

pub const ONE: i64 = 100_000;

/// Content defines of the count law and of mine-laying admission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortieDefines {
    /// NAir.MISSION_EFFICIENCY_MULT_AT_LACK_OF_FUEL, unclamped.
    fuel_penalty_raw: i64,
    /// NAir.MISSILE_LAUNCHER_SLOTS: the game stores 0
    /// for a missing or non-number value and its loader raises it to 1.
    missile_launcher_slots: i32,
    /// NNavy.CARRIER_OFFENSIVE_STANCE_SORTIE_RATIO, indexed by task-force stance.
    carrier_stance_ratios: Vec<i64>,
    /// NNavy.NAVAL_MINES_IN_REGION_MAX; its loader
    /// raises a value below one to one.
    pub mine_maximum_raw: i64,
}

impl SortieDefines {
    pub fn load(defines: &Defines) -> io::Result<Self> {
        Ok(Self {
            fuel_penalty_raw: defines.fixed5("NAir", "MISSION_EFFICIENCY_MULT_AT_LACK_OF_FUEL")?,
            missile_launcher_slots: defines.int32("NAir", "MISSILE_LAUNCHER_SLOTS")?.max(1),
            // An empty table refuses only the carrier wings whose stance it lacks.
            carrier_stance_ratios: defines
                .fixed5_array("NNavy", "CARRIER_OFFENSIVE_STANCE_SORTIE_RATIO")?,
            mine_maximum_raw: defines
                .fixed5("NNavy", "NAVAL_MINES_IN_REGION_MAX")?
                .max(ONE),
        })
    }
}

/// Key 0x11's runtime definition and the DLC mask its gate reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ModifierMetadata {
    flags: u32,
    feature_selector: u32,
    callback_kind: u8,
    feature_flags: u32,
}

impl ModifierMetadata {
    /// An entry counts only with its feature enabled; flag 0x10
    /// clamps it to [0, 1].
    fn effective(self, raw: i64) -> i64 {
        let enabled = self.feature_selector == 0
            || (self.callback_kind == 1 && self.feature_selector & self.feature_flags != 0);
        if !enabled {
            0
        } else if self.flags & 0x10 != 0 {
            raw.clamp(0, ONE)
        } else {
            raw
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Container {
    capacity: i64,
    supply: i64,
    fuel_demand: i64,
    fuel_allocated: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EfficiencyInputs {
    coverage: i64,
    penalty: i64,
    mission_mask: u32,
    /// Capacity and supply of the mission country's container.
    mission: Container,
    /// Fuel of the effective country's container.
    effective: Container,
    /// Weather, country and fleet-leader entries, already effective.
    modifier: i64,
    /// Present only with a mission region.
    country_region: Option<i64>,
    fuel_penalty: i64,
}

#[inline]
fn fixed_mul(left: i64, right: i64) -> i64 {
    left.wrapping_mul(right) / ONE
}

/// Full fuel without a demand, and for the three extreme
/// allocations the native code special-cases.
fn fuel_ratio(container: Container) -> i64 {
    if container.fuel_demand <= 0
        || matches!(container.fuel_allocated, i64::MAX | i64::MIN)
        || container.fuel_allocated == i64::MIN + 1
    {
        return ONE;
    }
    container
        .fuel_allocated
        .wrapping_mul(ONE)
        .wrapping_div(container.fuel_demand)
        .clamp(0, ONE)
}

fn sortie_efficiency(input: &EfficiencyInputs) -> i64 {
    let coverage = input.coverage.wrapping_mul(ONE) / ONE;
    let mut value = fixed_mul(
        input.mission.supply,
        fixed_mul(input.mission.capacity, coverage),
    );
    if input.mission_mask & 0x60 == 0 {
        value = fixed_mul(ONE.wrapping_sub(input.penalty), value);
    }
    value = fixed_mul(ONE.wrapping_add(input.modifier), value);
    let fuel = fuel_ratio(input.effective);
    if fuel < ONE {
        let factor = input
            .fuel_penalty
            .wrapping_add(fixed_mul(ONE.wrapping_sub(input.fuel_penalty), fuel));
        value = fixed_mul(factor, value);
    }
    if let Some(modifier) = input.country_region {
        value = fixed_mul(ONE.wrapping_add(modifier), value);
    }
    value.clamp(0, ONE)
}

/// Base-kind branches selected by the air base's kind field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Base {
    /// Kind 1: a state base's launcher level or the content define.
    Rocket(i64),
    /// Kind 2 always counts one.
    Gun,
    /// A host base's stance ratio, applied after rounding.
    Ordinary { stance_ratio: Option<i64> },
}

/// The caller reads the low 32 bits as a signed count.
fn scenario_plane_count(base: Base, wing_planes: i32, efficiency: i64) -> i32 {
    match base {
        Base::Gun => 1,
        Base::Rocket(slots) => {
            // The sign-extended slots times the efficiency,
            // wrapping. A product with a remainder is rounded up in fixed5:
            // the quotient plus one when positive, multiplied back by 100000
            // (wrapping), and the result divided again
            // before its low 32 bits are compared as a signed value.
            // The multiply-back can overflow, so the steps do not cancel.
            let mut raw = slots.wrapping_mul(efficiency);
            if raw % ONE != 0 {
                raw = (raw / ONE + i64::from(raw > 0)).wrapping_mul(ONE);
            }
            ((raw / ONE) as i32).min(wing_planes)
        }
        Base::Ordinary { stance_ratio } => {
            let raw = i64::from(wing_planes).wrapping_mul(efficiency);
            // The game rounds half away from zero into an int before the stance product.
            let rounded = if raw < 0 {
                raw.wrapping_sub(ONE / 2) / ONE
            } else {
                raw.wrapping_add(ONE / 2) / ONE
            };
            let count = i64::from(rounded as i32);
            stance_ratio.map_or(count, |ratio| fixed_mul(count, ratio)) as i32
        }
    }
}

/// This hour's recorded sortie operands, joined by their keys.
pub struct SortieSources<'a> {
    missions: BTreeMap<u64, &'a AirMissionInputRow>,
    containers: BTreeMap<(u64, u32), &'a AirSortieBaseCountryRow>,
    modifiers: BTreeMap<(u8, u32, u32), i64>,
    special: BTreeMap<u64, &'a AirSortieSpecialBaseRow>,
    metadata: ModifierMetadata,
}

impl<'a> SortieSources<'a> {
    /// `Err` names the incomplete table.
    pub fn read(frame: &'a DecodedFrame) -> io::Result<Result<Self, &'static str>> {
        let (
            Available::Complete(missions),
            Available::Complete(containers),
            Available::Complete(modifiers),
            Available::Complete(special),
            Available::Complete([metadata]),
        ) = (
            rows::<tables::AirMissionInputs>(frame)?,
            rows::<tables::AirSortieBaseCountries>(frame)?,
            rows::<tables::AirSortieModifiers>(frame)?,
            rows::<tables::AirSortieSpecialBases>(frame)?,
            rows::<tables::AirSortieModifierMetadata>(frame)?,
        )
        else {
            return Ok(Err("complete same-hour sortie operands are required"));
        };
        let feature_flags = if metadata.feature_selector != 0 && metadata.callback_kind == 1 {
            // The feature-flags getter reads the active DLC mask.
            match rows::<tables::SimulationFeatures>(frame)? {
                Available::Complete([features]) => features.dlc_mask,
                _ => return Ok(Err("the sortie modifier gate needs the DLC mask")),
            }
        } else {
            0
        };
        Ok(Ok(Self {
            missions: missions.iter().map(|row| (row.wing, row)).collect(),
            containers: containers
                .iter()
                .map(|row| ((row.base, row.country), row))
                .collect(),
            modifiers: modifiers
                .iter()
                .map(|row| ((row.scope, row.subject, row.region), row.raw))
                .collect(),
            special: special.iter().map(|row| (row.base, row)).collect(),
            metadata: ModifierMetadata {
                flags: metadata.flags,
                feature_selector: metadata.feature_selector,
                callback_kind: metadata.callback_kind,
                feature_flags,
            },
        }))
    }

    pub fn mission(&self, wing: u64) -> Option<&'a AirMissionInputRow> {
        self.missions.get(&wing).copied()
    }

    /// Host admission: a stateless base's task force in combat admits
    /// only a mask with bit 0 or 2 whose mission region is the task force
    /// location's region. `Err` names a missing or unsupported operand.
    pub fn host_admits(&self, wing: &AirWingRow, base: &AirBaseRow) -> Result<bool, &'static str> {
        if base.state.is_some() {
            return Ok(true);
        }
        if base.kind == 2 {
            return Err("a stateless kind-2 base's host is not supported");
        }
        let host = self
            .special
            .get(&base.id)
            .ok_or("host base lacks its task force")?;
        let combats = host
            .task_force_combats
            .ok_or("host base lacks its task force")?;
        Ok(combats <= 0
            || (wing.mission_mask & 5 != 0 && wing.mission_region == host.task_force_region))
    }

    /// A missing row is a null or all-zero container. `fuel` refuses a row
    /// recorded without it, which only an effective country's carries.
    fn container(&self, base: u64, country: u32, fuel: bool) -> Result<Container, &'static str> {
        let Some(row) = self.containers.get(&(base, country)) else {
            return Ok(Container::default());
        };
        let (fuel_demand, fuel_allocated) = match (row.fuel_demand_raw, row.fuel_allocated_raw) {
            (Some(demand), Some(allocated)) => (demand, allocated),
            _ if fuel => return Err("the effective country's base container lacks its fuel"),
            _ => (0, 0),
        };
        Ok(Container {
            capacity: row.capacity_raw,
            supply: row.supply_raw,
            fuel_demand,
            fuel_allocated,
        })
    }

    fn modifier(&self, scope: u8, subject: u32, region: u32) -> i64 {
        self.metadata.effective(
            self.modifiers
                .get(&(scope, subject, region))
                .copied()
                .unwrap_or(0),
        )
    }

    /// The wing's scenario plane count from its pool base. `mission_slot` is
    /// the resolved mission country; `effective` the wing's raw effective tag
    /// and its slot. `Err` names a missing operand.
    pub fn count(
        &self,
        wing: &AirWingRow,
        input: &AirMissionInputRow,
        base: &AirBaseRow,
        mission_slot: u32,
        effective: (u32, u32),
        defines: &SortieDefines,
    ) -> Result<i32, &'static str> {
        let special = self.special.get(&base.id);
        let arm = match (base.kind, base.state) {
            (2, _) => Base::Gun,
            (1, Some(_)) => Base::Rocket(i64::from(
                special
                    .and_then(|row| row.rocket_slots)
                    .ok_or("state rocket base lacks its launcher level")?,
            )),
            (1, None) => Base::Rocket(i64::from(defines.missile_launcher_slots)),
            (_, Some(_)) => Base::Ordinary { stance_ratio: None },
            (_, None) => {
                let stance = special
                    .and_then(|row| row.carrier_stance)
                    .ok_or("carrier base lacks its task-force stance")?;
                let ratio = usize::try_from(stance)
                    .ok()
                    .and_then(|index| defines.carrier_stance_ratios.get(index))
                    .ok_or("carrier stance has no content sortie ratio")?;
                Base::Ordinary {
                    stance_ratio: Some(*ratio),
                }
            }
        };
        let efficiency = match arm {
            // The gun arm ignores the efficiency.
            Base::Gun => 0,
            _ => self.efficiency(wing, input, base, mission_slot, effective, defines)?,
        };
        Ok(scenario_plane_count(arm, wing.planes, efficiency))
    }

    /// Sortie efficiency with the ordinary land-base and mission parameters.
    pub fn efficiency(
        &self,
        wing: &AirWingRow,
        input: &AirMissionInputRow,
        base: &AirBaseRow,
        mission_slot: u32,
        effective: (u32, u32),
        defines: &SortieDefines,
    ) -> Result<i64, &'static str> {
        let (effective_raw, effective_slot) = effective;
        // The override loop matches `canonical_slot`, zero for a null tag, and
        // returns the stored value unclamped.
        let override_slot = if effective_raw == 0 {
            0
        } else {
            effective_slot
        };
        if let Some(&raw) =
            self.modifiers
                .get(&(AirSortieModifierRow::EFFICIENCY_OVERRIDE, override_slot, 0))
        {
            return Ok(raw);
        }
        let positive = (effective_raw as i32) > 0;
        let mut modifier = 0_i64;
        if let Some(region) = wing.mission_region {
            modifier = self.modifier(AirSortieModifierRow::WEATHER, 0, region);
        }
        if positive {
            modifier = modifier.wrapping_add(self.modifier(
                AirSortieModifierRow::COUNTRY,
                effective_slot,
                0,
            ));
        }
        if base.state.is_none() {
            let leader = self
                .special
                .get(&base.id)
                .map_or(0, |row| row.leader_modifier_raw);
            modifier = modifier.wrapping_add(self.metadata.effective(leader));
        }
        // The game applies the country-region term whenever the mission has
        // a region, resolving any tag; only positive tags are recorded.
        let country_region = match wing.mission_region {
            None => None,
            Some(_) if !positive => {
                return Err("a non-positive effective country's region modifier is not recorded");
            }
            Some(region) => {
                Some(self.modifier(AirSortieModifierRow::COUNTRY_REGION, effective_slot, region))
            }
        };
        Ok(sortie_efficiency(&EfficiencyInputs {
            coverage: input.coverage_raw,
            penalty: input.reassignment_penalty_raw,
            mission_mask: wing.mission_mask,
            mission: self.container(base.id, mission_slot, false)?,
            effective: self.container(base.id, effective_slot, true)?,
            modifier,
            country_region,
            fuel_penalty: defines.fuel_penalty_raw,
        }))
    }
}

#[cfg(test)]
mod tests;
