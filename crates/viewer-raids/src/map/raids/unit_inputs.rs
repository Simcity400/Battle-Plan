use super::{RaidAssets, script::World};
use content_reader_raids::{raids::RaidGraphicsDefines, raids::RaidType};
use replay_data::{rows::RaidRow, rows::RaidTargetRow, rows::TaskForceRow, rows::tables};
use replay_frame::frame::DecodedSource;
use std::collections::BTreeMap;
pub const NO_BUILDING_TYPE: u32 = 0x4c17;
const SCALE: i64 = 100_000;
const SOURCE_NAVAL_BASE: u32 = engine_native_air::craid_source::KIND_NAVAL_BASE;
const SOURCE_CARRIER: u32 = engine_native_air::craid_source::KIND_CARRIER;
const SOURCE_SUBMARINE: u32 = engine_native_air::craid_source::KIND_SUBMARINE;
const SOURCE_BUILDING: u32 = engine_native_air::craid_source::KIND_BUILDING;
use viewer_gpu::unit_pose::PLANE_UNRECORDED;
pub fn source_province<'a>(
    assets: &RaidAssets,
    world: &World<'a>,
    raid: &RaidRow,
    ships: &mut Option<Result<BTreeMap<u64, &'a TaskForceRow>, String>>,
) -> Result<(u32, Result<Option<u32>, String>), String> {
    let Some(kind) = raid.source.kind.filter(|_| raid.source.kind_present != 0) else {
        return Err("a raid without a source".into());
    };
    let (province, country) = match kind {
        SOURCE_CARRIER | SOURCE_SUBMARINE => {
            let ships = ships.get_or_insert_with(|| {
                let frame = world.frame();
                let forces: BTreeMap<u64, &TaskForceRow> = frame
                    .complete::<tables::TaskForces>()
                    .map_err(|error| error.to_string())?
                    .iter()
                    .map(|row| (row.id, row))
                    .collect();
                Ok(frame
                    .complete::<tables::Ships>()
                    .map_err(|error| error.to_string())?
                    .iter()
                    .filter_map(|ship| Some((ship.id, *forces.get(&ship.task_force)?)))
                    .collect())
            });
            let ships = ships.as_ref().map_err(Clone::clone)?;
            let ship = (u64::from(raid.source.ship_reference_id) << 32)
                | u64::from(raid.source.ship_reference_type);
            let force = ships
                .get(&ship)
                .ok_or("a ship source whose ship or task force is not recorded")?;
            (force.province, Some(force.roster_owner))
        }
        SOURCE_BUILDING => {
            let (building_type, location, country) =
                building_source(raid).map_err(str::to_owned)?;
            let province = building_province(assets, world, building_type, location, country)?
                .ok_or("a building source whose building has no province")?;
            (province, None)
        }
        _ => (raid.source.province, None),
    };
    if province == 0 {
        return Err("a raid source without a province".into());
    }
    let raw = match country {
        Some(raw) => Ok(raw),
        None => world
            .province_row(province)
            .map(|(_, controller)| controller)
            .ok_or_else(|| format!("the raid source province {province} is not recorded")),
    };
    let country = raw.and_then(|raw| {
        if raw == 0 {
            return Ok(None);
        }
        world
            .slot(raw)
            .map(Some)
            .ok_or_else(|| format!("the raid source country {raw} has no CountryLinks row"))
    });
    Ok((province, country))
}

pub fn building_source(raid: &RaidRow) -> Result<(u32, i32, i32), &'static str> {
    let source = &raid.source;
    if source.building_type == NO_BUILDING_TYPE || source.building_location == -1 {
        return Err("a building source without a building");
    }
    Ok((
        source.building_type,
        source.building_location,
        source.building_country,
    ))
}

pub fn building_province(
    assets: &RaidAssets,
    world: &World<'_>,
    building_type: u32,
    location: i32,
    country: i32,
) -> Result<Option<u32>, String> {
    let kind = assets.building(world, building_type)?;
    let Ok(location) = u32::try_from(location) else {
        return Ok(None);
    };
    if kind.province_max.is_some_and(|cap| cap > 0) {
        return Ok(Some(location).filter(|&id| id != 0));
    }
    assets.anchor_province(
        world,
        &replay_data::rows::RaidScope {
            building_type,
            building_location: location,
            building_flags: 2,
            building_country: country,
            ..Default::default()
        },
    )
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpeedInputs {
    pub wing_max_speed: Option<i64>,
    pub army_max_speed: Option<i64>,
    pub transport_speeds: Vec<i64>,
    pub transport_from_country: u8,
    pub transport_fallback: u8,
    pub ace_alive: Option<u8>,
    pub ace_speed: i64,
    pub ace_type_speed: i64,
    pub ace_wing_planes: Option<i32>,
    pub ace_effectiveness: i64,
    pub ace_bonuses: i64,
}

pub fn plane_inputs(
    frame: &dyn DecodedSource,
    raid: &RaidRow,
) -> Result<(i64, SpeedInputs), String> {
    let travelled = raid.travelled.ok_or(PLANE_UNRECORDED)?;
    let rows = frame
        .complete::<tables::RaidSpeedInputs>()
        .map_err(|error| format!("the raid unit's speed inputs are withheld: {error}"))?;
    let row = rows
        .iter()
        .find(|row| row.id == raid.id && row.reference_type == raid.reference_type)
        .ok_or("the raid has no RaidSpeedInputs row")?;
    Ok((travelled, SpeedInputs::from_row(row, frame)?))
}

impl SpeedInputs {
    pub fn from_row(
        row: &replay_data::rows::RaidSpeedInputRow,
        frame: &dyn DecodedSource,
    ) -> Result<Self, String> {
        let country = |slot: Option<u32>,
                       name: &str,
                       value: fn(&replay_data::rows::CountryCommandPowerRow) -> Option<i64>|
         -> Result<i64, String> {
            let Some(slot) = slot else {
                return Ok(0);
            };
            let rows = frame
                .complete::<tables::CountryCommandPower>()
                .map_err(|error| {
                    format!("the raid ace's country modifiers are withheld: {error}")
                })?;
            let row = rows
                .binary_search_by_key(&slot, |row| row.canonical_slot)
                .map(|at| &rows[at])
                .map_err(|_| format!("country slot {slot} has no CountryCommandPower row"))?;
            value(row).ok_or_else(|| format!("{name} is not a registered modifier"))
        };
        Ok(Self {
            wing_max_speed: row.wing_max_speed,
            army_max_speed: row.army_max_speed,
            transport_speeds: row.transport_speeds.clone(),
            transport_from_country: row.transport_from_country,
            transport_fallback: row.transport_fallback,
            ace_alive: (row.ace_present != 0).then_some(row.ace_alive),
            ace_speed: row.ace_speed,
            ace_type_speed: row.ace_type_speed,
            ace_wing_planes: row.ace_wing_planes,
            ace_effectiveness: country(row.ace_owner, "ace_effectiveness_factor", |row| {
                row.ace_effectiveness_factor
            })?,
            ace_bonuses: country(row.wing_country, "air_ace_bonuses_factor", |row| {
                row.air_ace_bonuses_factor
            })?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeedDefines {
    pub land_speed_modifier: i64,
    pub naval_speed_modifier: i64,
    pub naval_transfer_base_speed: i64,
    pub raid_unit_speed_multiplier: i64,
    pub ace_wing_size: i64,
    pub ace_wing_size_max_bonus: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeedBranch {
    TransportPlanes,
    Convoys,
    Unit,
}

impl SpeedBranch {
    pub fn of(
        kind: &RaidType,
        category: &dyn Fn(&str) -> Result<u64, String>,
    ) -> Result<Self, String> {
        let has = |bit: u64| -> Result<bool, String> {
            let mut found = false;
            for (name, _) in &kind.essential_equipment {
                found |= category(name)? & bit != 0;
            }
            Ok(found)
        };
        Ok(if has(0x10_0000)? {
            Self::TransportPlanes
        } else if has(1)? {
            Self::Convoys
        } else {
            Self::Unit
        })
    }
}

fn times(x: i64, y: i64) -> i64 {
    x.wrapping_mul(y) / SCALE
}

pub fn ace_wing_scale(planes: i64, wing_size: i64, max_bonus: i64) -> i64 {
    if planes == 0 {
        return 100_000;
    }
    let finite = (wing_size as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd;
    if finite {
        (wing_size.wrapping_mul(100_000) / planes.wrapping_mul(100_000)).min(max_bonus)
    } else {
        max_bonus.min(0xffff_ffff)
    }
}

pub fn raid_unit_speed(
    branch: SpeedBranch,
    inputs: &SpeedInputs,
    speed_multiplier: i64,
    defines: &SpeedDefines,
) -> i64 {
    let unit = match branch {
        SpeedBranch::TransportPlanes if inputs.transport_fallback != 0 => 100_000,
        SpeedBranch::TransportPlanes => {
            let mut best: Option<i64> = None;
            for &speed in &inputs.transport_speeds {
                if best.is_none_or(|best| best < speed) {
                    best = Some(speed);
                }
            }
            match best {
                Some(best) => times(best, defines.land_speed_modifier),
                None => 100_000,
            }
        }
        SpeedBranch::Convoys => times(
            defines.naval_speed_modifier,
            defines.naval_transfer_base_speed,
        ),
        SpeedBranch::Unit => {
            if let Some(wing) = inputs.wing_max_speed {
                let mut factor = 100_000_i64;
                if inputs.ace_alive.is_some_and(|alive| alive != 0) {
                    let scale = inputs.ace_wing_planes.map_or(100_000, |planes| {
                        ace_wing_scale(
                            i64::from(planes),
                            defines.ace_wing_size,
                            defines.ace_wing_size_max_bonus,
                        )
                    });
                    let scaled = times(inputs.ace_speed, scale);
                    let scaled = times(inputs.ace_effectiveness.wrapping_add(100_000), scaled);
                    factor = scaled.wrapping_add(100_000);
                    if inputs.ace_type_speed != 0 {
                        factor = factor.wrapping_add(inputs.ace_bonuses);
                    }
                }
                times(times(factor, wing), defines.land_speed_modifier)
            } else if let Some(army) = inputs.army_max_speed {
                times(army, defines.land_speed_modifier)
            } else {
                0
            }
        }
    };
    times(
        times(unit, defines.raid_unit_speed_multiplier),
        speed_multiplier,
    )
}

pub fn target_distance(
    assets: &RaidAssets,
    frame: &dyn DecodedSource,
    raid: &RaidRow,
    target: &RaidTargetRow,
    held: &mut Vec<super::paths::Held>,
) -> Result<i64, String> {
    let kind = raid.source.kind.unwrap_or(u32::MAX);
    let from = raid.source.province;
    let found = if kind == SOURCE_NAVAL_BASE || kind == SOURCE_BUILDING {
        let distances = super::paths::Distances::new(assets, frame);
        let found = distances.distance(kind as u8, from, &target.scope);
        held.extend(distances.take_held());
        found?
    } else {
        assets.province(&target.scope).and_then(|to| {
            assets
                .geometry
                .wrapped_distance(from, to)
                .map(|pixels| pixels.wrapping_mul(100_000) / 300_000)
        })
    };
    Ok(found.unwrap_or(100_000))
}

impl SpeedDefines {
    pub fn read(
        graphics: &RaidGraphicsDefines,
        raids: &super::RaidDefines,
    ) -> Result<Self, String> {
        let speeds = raids.unit_speeds.ok_or(
            "NMilitary.LAND_SPEED_MODIFIER, NNavy.NAVAL_SPEED_MODIFIER or NNavy.NAVAL_TRANSFER_BASE_SPEED cannot be read",
        )?;
        Ok(Self {
            land_speed_modifier: speeds[0],
            naval_speed_modifier: speeds[1],
            naval_transfer_base_speed: speeds[2],
            raid_unit_speed_multiplier: graphics.speed_multiplier_raw,
            ace_wing_size: raids.ace_wing_size,
            ace_wing_size_max_bonus: raids.ace_wing_size_max_bonus,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RaidPool {
    pub archetype_totals: Vec<(u32, i64)>,
    pub nukes: i32,
}

pub fn raid_pool(frame: &dyn DecodedSource, raid: &RaidRow) -> Result<RaidPool, String> {
    let rows = frame
        .complete::<tables::RaidEquipment>()
        .map_err(|error| format!("the raid's own equipment pool is withheld: {error}"))?;
    let row = rows
        .iter()
        .find(|row| row.id == raid.id && row.reference_type == raid.reference_type)
        .ok_or("the raid has no RaidEquipment row (its own equipment pool)")?;
    if row.archetype_tokens.len() != row.archetype_amounts.len() {
        return Err(format!(
            "the raid's RaidEquipment row has {} archetype tokens and {} archetype amounts",
            row.archetype_tokens.len(),
            row.archetype_amounts.len()
        ));
    }
    Ok(RaidPool {
        archetype_totals: row
            .archetype_tokens
            .iter()
            .copied()
            .zip(row.archetype_amounts.iter().copied())
            .collect(),
        nukes: row.nukes,
    })
}

pub fn unit_at_source(world: &World<'_>, raid: &RaidRow, source: u32) -> Result<bool, String> {
    let frame = world.frame();
    let complete = |error: std::io::Error| error.to_string();
    if raid.unit != 0
        && let Some(division) = frame
            .complete::<tables::Divisions>()
            .map_err(complete)?
            .iter()
            .find(|division| division.id == raid.unit)
    {
        return Ok(division.province == source);
    }
    if raid.air_wing != 0
        && let Some(wing) = frame
            .complete::<tables::AirWings>()
            .map_err(complete)?
            .iter()
            .find(|wing| wing.id == raid.air_wing)
    {
        let base = frame
            .complete::<tables::AirBases>()
            .map_err(complete)?
            .iter()
            .find(|base| base.id == wing.air_base)
            .and_then(|base| base.province);
        return Ok(base == Some(source));
    }
    Ok(false)
}

pub fn pool_meets(
    kind: &RaidType,
    pool: &RaidPool,
    archetype_name: &dyn Fn(u32) -> Result<String, String>,
) -> Result<bool, String> {
    let mut have: BTreeMap<String, i64> = BTreeMap::new();
    for &(token, total) in &pool.archetype_totals {
        have.entry(archetype_name(token)?).or_insert(total);
    }
    for (list, nukes) in [
        (&kind.essential_equipment, kind.nukes),
        (&kind.additional_equipment, kind.additional_nukes),
    ] {
        if pool.nukes < nukes {
            return Ok(false);
        }
        for (archetype, amount) in list {
            let units = (have.get(archetype).copied().unwrap_or(0) / 100_000) as i32;
            if units < *amount {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
