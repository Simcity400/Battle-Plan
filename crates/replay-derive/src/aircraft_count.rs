
use std::{collections::BTreeMap, io};

use replay_data::{
    rows::AirBaseRow, rows::AirMissionInputRow, rows::AirSortieBaseCountryRow,
    rows::AirSortieModifierRow, rows::AirSortieSpecialBaseRow, rows::AirWingRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;

use crate::air_mission_sources::{AirRegistrySources, MissionCountrySources, complete};

pub const ONE: i64 = 100_000;

pub fn empty_menu_efficiency(
    frame: &DecodedFrame,
    country: u32,
    region: u32,
) -> Result<i64, String> {
    let modifiers = frame
        .complete::<tables::AirSortieModifiers>()
        .map_err(|e| e.to_string())?;
    if !modifiers.iter().any(|row| {
        row.scope == AirSortieModifierRow::MENU_MODIFIERS_STATUS
            && row.subject == country
            && row.region == 0
            && row.raw == 1
    }) {
        return Err("air combat menu modifier coverage is unavailable".into());
    }
    let [metadata] = frame
        .complete::<tables::AirSortieModifierMetadata>()
        .map_err(|e| e.to_string())?
    else {
        return Err("sortie modifier metadata is unavailable".into());
    };
    let features = if metadata.feature_selector != 0 && metadata.callback_kind == 1 {
        let [features] = frame
            .complete::<tables::SimulationFeatures>()
            .map_err(|e| e.to_string())?
        else {
            return Err("sortie modifier gate needs the DLC mask".into());
        };
        features.dlc_mask
    } else {
        0
    };
    let metadata = ModifierMetadata {
        flags: metadata.flags,
        feature_selector: metadata.feature_selector,
        callback_kind: metadata.callback_kind,
        feature_flags: features,
    };
    let get = |scope, region| {
        metadata.effective(
            modifiers
                .iter()
                .find(|row| row.scope == scope && row.subject == country && row.region == region)
                .map_or(0, |row| row.raw),
        )
    };
    Ok(ONE
        .wrapping_add(get(AirSortieModifierRow::COUNTRY_REGION, region))
        .wrapping_add(get(AirSortieModifierRow::COUNTRY, 0)))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortieDefines {
    pub fuel_penalty_raw: i64,
    pub missile_launcher_slots: i32,
    pub carrier_stance_ratios: Vec<i64>,
    pub mine_maximum_raw: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ModifierMetadata {
    flags: u32,
    feature_selector: u32,
    callback_kind: u8,
    feature_flags: u32,
}

impl ModifierMetadata {
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
    mission: Container,
    effective: Container,
    modifier: i64,
    country_region: Option<i64>,
    fuel_penalty: i64,
}

#[inline]
fn fixed_mul(left: i64, right: i64) -> i64 {
    left.wrapping_mul(right) / ONE
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Base {
    Rocket(i64),
    Gun,
    Ordinary { stance_ratio: Option<i64> },
}

fn scenario_plane_count(base: Base, wing_planes: i32, efficiency: i64) -> i32 {
    match base {
        Base::Gun => 1,
        Base::Rocket(slots) => {
            let mut raw = slots.wrapping_mul(efficiency);
            if raw % ONE != 0 {
                raw = (raw / ONE + i64::from(raw > 0)).wrapping_mul(ONE);
            }
            ((raw / ONE) as i32).min(wing_planes)
        }
        Base::Ordinary { stance_ratio } => {
            let raw = i64::from(wing_planes).wrapping_mul(efficiency);
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

pub fn efficiency_override(
    modifiers: &BTreeMap<(u8, u32, u32), i64>,
    (effective_raw, effective_slot): (u32, u32),
) -> Option<i64> {
    let slot = if effective_raw == 0 {
        0
    } else {
        effective_slot
    };
    modifiers
        .get(&(AirSortieModifierRow::EFFICIENCY_OVERRIDE, slot, 0))
        .copied()
}

pub struct SortieSources<'a> {
    missions: BTreeMap<u64, &'a AirMissionInputRow>,
    containers: BTreeMap<(u64, u32), &'a AirSortieBaseCountryRow>,
    modifiers: BTreeMap<(u8, u32, u32), i64>,
    special: BTreeMap<u64, &'a AirSortieSpecialBaseRow>,
    metadata: ModifierMetadata,
}

impl<'a> SortieSources<'a> {
    pub fn read(frame: &'a DecodedFrame) -> io::Result<Result<Self, &'static str>> {
        let (Some(missions), Some(containers), Some(modifiers), Some(special), Some([metadata])) = (
            complete::<tables::AirMissionInputs>(frame)?,
            complete::<tables::AirSortieBaseCountries>(frame)?,
            complete::<tables::AirSortieModifiers>(frame)?,
            complete::<tables::AirSortieSpecialBases>(frame)?,
            complete::<tables::AirSortieModifierMetadata>(frame)?,
        ) else {
            return Ok(Err("complete same-hour sortie operands are required"));
        };
        let feature_flags = if metadata.feature_selector != 0 && metadata.callback_kind == 1 {
            match complete::<tables::SimulationFeatures>(frame)? {
                Some([features]) => features.dlc_mask,
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

    pub fn empty_region_efficiency(&self, country: u32, region: u32) -> Result<i64, &'static str> {
        self.menu_modifier(AirSortieModifierRow::COUNTRY_REGION, country, region)
            .map(|(_, regional)| {
                ONE.wrapping_add(regional).wrapping_add(self.modifier(
                    AirSortieModifierRow::COUNTRY,
                    country,
                    0,
                ))
            })
    }

    pub fn menu_modifier(
        &self,
        scope: u8,
        country: u32,
        region: u32,
    ) -> Result<(i64, i64), &'static str> {
        let (subject, covered_region) = if scope == AirSortieModifierRow::WEATHER {
            (0, region)
        } else {
            (country, 0)
        };
        if self.modifiers.get(&(
            AirSortieModifierRow::MENU_MODIFIERS_STATUS,
            subject,
            covered_region,
        )) != Some(&1)
        {
            return Err("air combat menu modifier coverage is unavailable");
        }
        let raw = self
            .modifiers
            .get(&(scope, country, region))
            .copied()
            .unwrap_or(0);
        Ok((raw, self.metadata.effective(raw)))
    }

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
            Base::Gun => 0,
            _ => self.efficiency(
                wing,
                input,
                base,
                mission_slot,
                effective,
                defines.fuel_penalty_raw,
            )?,
        };
        Ok(scenario_plane_count(arm, wing.planes, efficiency))
    }

    pub fn wing_efficiency(
        &self,
        wing: &AirWingRow,
        registry: &AirRegistrySources,
        countries: &MissionCountrySources<'_>,
        fuel_penalty: i64,
    ) -> Result<i64, &'static str> {
        let base = registry
            .bases
            .get(&wing.air_base)
            .ok_or("wing pool base does not resolve")?;
        let input = self
            .mission(wing.id)
            .ok_or("wing has no current sortie operands")?;
        let mission = countries
            .resolve(wing)
            .ok_or("wing's mission country is unresolved")?;
        let effective = countries
            .effective(wing)
            .ok_or("wing's effective country is unresolved")?;
        self.efficiency(wing, input, base, mission.slot, effective, fuel_penalty)
    }

    pub fn efficiency(
        &self,
        wing: &AirWingRow,
        input: &AirMissionInputRow,
        base: &AirBaseRow,
        mission_slot: u32,
        effective: (u32, u32),
        fuel_penalty: i64,
    ) -> Result<i64, &'static str> {
        let (effective_raw, effective_slot) = effective;
        if let Some(raw) = efficiency_override(&self.modifiers, effective) {
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
            fuel_penalty,
        }))
    }
}

#[cfg(test)]
mod tests;
