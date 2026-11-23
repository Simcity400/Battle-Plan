use replay_data::{rows::DivisionRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};
pub struct Inputs<'a> {
    pub members: &'a BTreeMap<u32, Vec<u32>>,
    pub neighbors: &'a Result<BTreeMap<u32, Vec<u32>>, String>,
    pub border: &'a Result<Defines, String>,
}
pub struct Defines {
    pub org: i64,
    pub strength: i64,
    pub victory: i64,
}
impl Defines {
    pub fn load(d: &content_reader_defines::defines::Defines) -> io::Result<Self> {
        Ok(Self {
            org: d.fixed5("NMilitary", "COMBAT_VALUE_ORG_IMPORTANCE")?,
            strength: d.fixed5("NMilitary", "COMBAT_VALUE_STR_IMPORTANCE")?,
            victory: d.fixed5("NMilitary", "BORDER_WAR_VICTORY")?,
        })
    }
}

pub fn same(links: &BTreeMap<u32, u32>, a: u32, b: u32) -> Result<bool, String> {
    if a == b {
        return Ok(true);
    }
    if a == 0 || b == 0 {
        return Ok(false);
    }
    let slot = |id| {
        links
            .get(&id)
            .copied()
            .ok_or_else(|| format!("CountryLinks lacks border country {id}"))
    };
    Ok(slot(a)? == slot(b)?)
}

pub fn combat_value(d: &DivisionRow, stats: [i64; 3], skill: i32, defines: &Defines) -> i64 {
    let stats = stats
        .into_iter()
        .fold(0_i64, i64::wrapping_add)
        .wrapping_add(i64::from(skill).wrapping_mul(100_000));
    let factor = 100_000_i64
        .wrapping_add(d.organisation.wrapping_mul(defines.org) / 100_000)
        .wrapping_add(d.hit_points.wrapping_mul(defines.strength) / 100_000);
    (factor.wrapping_mul(stats) / 100_000).wrapping_mul(100) / 100_000
}

pub fn enemy_ratio(power: [i64; 2]) -> i64 {
    let total = power[0].wrapping_add(power[1]);
    if total < 1 {
        return 50_000;
    }
    let own = if (power[0] as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        0xffff_ffff
    } else {
        power[0].wrapping_mul(100_000) / total
    };
    100_000_i64.wrapping_sub(own)
}

pub fn power(
    content: &Inputs<'_>,
    frame: &DecodedFrame,
    viewer_raw: u32,
    links: &BTreeMap<u32, u32>,
    state: u32,
    states: &BTreeMap<u32, &replay_data::rows::StateRow>,
    countries: &BTreeMap<u32, &replay_data::rows::CountryRow>,
) -> Result<[i64; 2], String> {
    let flags = frame
        .complete::<tables::StateFlags>()
        .map_err(|e| e.to_string())?;
    let flags: BTreeMap<_, _> = flags.iter().map(|row| (row.state, row)).collect();
    let data = content;
    let defines = data.border.as_ref().map_err(Clone::clone)?;
    let neighbors = data
        .neighbors
        .as_ref()
        .map_err(Clone::clone)?
        .get(&state)
        .ok_or("border state neighbors absent")?;
    let mut demanded = vec![state];
    for &id in neighbors {
        if id == state {
            continue;
        }
        let neighbor = states.get(&id).ok_or("neighbor state absent")?;
        let active = flags
            .get(&id)
            .ok_or("neighbor active predicate absent")?
            .border_conflict_active
            .require("StateFlags.border_conflict_active")?
            != &0;
        if active && !same(links, neighbor.controller, viewer_raw)? {
            demanded.push(id);
        }
    }
    let divisions = frame
        .complete::<tables::Divisions>()
        .map_err(|e| e.to_string())?;
    let division_by_id: BTreeMap<_, _> = divisions.iter().map(|d| (d.id, d)).collect();
    let statuses = frame
        .complete::<tables::DivisionStatus>()
        .map_err(|e| e.to_string())?;
    let statuses: BTreeMap<_, _> = statuses.iter().map(|row| (row.division, row)).collect();
    let order = frame
        .complete::<tables::ProvinceUnitOrder>()
        .map_err(|e| e.to_string())?;
    let order: BTreeMap<_, _> = order.iter().map(|row| (row.province, row)).collect();
    let provinces = frame
        .complete::<tables::Provinces>()
        .map_err(|e| e.to_string())?;
    let provinces: BTreeMap<_, _> = provinces.iter().map(|row| (row.id, row)).collect();
    let groups = frame
        .complete::<tables::OrdersGroups>()
        .map_err(|e| e.to_string())?;
    let groups: BTreeMap<_, _> = groups.iter().map(|row| (row.handle, row)).collect();
    let leaders = frame
        .complete::<tables::UnitLeaders>()
        .map_err(|e| e.to_string())?;
    let leaders: BTreeMap<_, _> = leaders
        .iter()
        .map(|row| {
            (
                (u64::from(row.id) << 32 | u64::from(row.reference_type)),
                row,
            )
        })
        .collect();
    let mut power = [0_i64; 2];
    let mut controllers = Vec::new();
    for state in demanded {
        for id in data
            .members
            .get(&state)
            .ok_or("border state content absent")?
        {
            let province = provinces.get(id).ok_or("border province absent")?;
            let roster = order
                .get(id)
                .ok_or_else(|| format!("selected border province {id} unit roster unavailable"))?;
            for handle in &roster.mixed {
                let Some(division) = division_by_id.get(handle) else {
                    continue;
                };
                let stats = statuses
                    .get(handle)
                    .ok_or_else(|| format!("selected border army {handle} status unavailable"))?
                    .border_combat_stats.require("DivisionStatus.border_combat_stats")?
                    .as_ref().copied()
                    .ok_or_else(|| format!("selected border army {handle} cached defense/soft/hard attack unavailable"))?;
                let skill = if division.orders_group == 0 {
                    0
                } else {
                    let group = groups
                        .get(&division.orders_group)
                        .ok_or("border army orders group absent")?;
                    if group.leader == 0 {
                        0
                    } else {
                        let leader = leaders
                            .get(&group.leader)
                            .ok_or("border army leader absent")?;
                        if leader.skill_status == 0 {
                            return Err("border army leader skill unavailable".into());
                        }
                        leader.skill_level
                    }
                };
                let side = usize::from(!same(links, division.logical_owner, viewer_raw)?);
                power[side] =
                    power[side].wrapping_add(combat_value(division, stats, skill, defines));
            }
            let mut known = false;
            for &raw in &controllers {
                known |= same(links, raw, province.controller)?;
            }
            if !known {
                controllers.push(province.controller);
            }
        }
    }
    for raw in controllers {
        let slot = links
            .get(&raw)
            .ok_or("border province controller unresolved")?;
        let level = countries
            .get(slot)
            .ok_or("selected controller absent")?
            .border_land_doctrine_level
            .require("Countries.border_land_doctrine_level")?
            .as_ref()
            .copied()
            .ok_or_else(|| format!("selected controller {raw} land-doctrine cache unavailable"))?;
        let side = usize::from(!same(links, raw, viewer_raw)?);
        power[side] = power[side].wrapping_mul(i64::from(level.wrapping_add(1)));
    }
    Ok(power)
}

pub fn enemy_share(
    content: &Inputs<'_>,
    frame: &DecodedFrame,
    country: u32,
    links: &BTreeMap<u32, u32>,
    state: u32,
) -> Result<i64, String> {
    let states = frame
        .complete::<tables::States>()
        .map_err(|e| e.to_string())?;
    let states: BTreeMap<_, _> = states.iter().map(|row| (row.id, row)).collect();
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())?;
    let countries: BTreeMap<_, _> = countries
        .iter()
        .map(|row| (row.canonical_slot, row))
        .collect();
    power(content, frame, country, links, state, &states, &countries).map(enemy_ratio)
}
