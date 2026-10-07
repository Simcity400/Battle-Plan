//! Border power and tooltip (1409C8D50 / 1409C9B50 / 140C6E140).
use super::localise;
use installed_content::localisation::{Argument as Arg, Localisation};
use replay::{
    frame::DecodedFrame,
    rows::{DivisionRow, StateFlagRow, tables},
};
use std::{collections::BTreeMap, io};

pub(super) struct Defines {
    org: i64,
    strength: i64,
    victory: i64,
}
impl Defines {
    pub(super) fn load(d: &installed_content::defines::Defines) -> io::Result<Self> {
        Ok(Self {
            org: d.fixed5("NMilitary", "COMBAT_VALUE_ORG_IMPORTANCE")?,
            strength: d.fixed5("NMilitary", "COMBAT_VALUE_STR_IMPORTANCE")?,
            victory: d.fixed5("NMilitary", "BORDER_WAR_VICTORY")?,
        })
    }
}

fn same(links: &BTreeMap<u32, u32>, a: u32, b: u32) -> Result<bool, String> {
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

fn combat_value(d: &DivisionRow, stats: [i64; 3], skill: i32, defines: &Defines) -> i64 {
    let stats = stats
        .into_iter()
        .fold(0_i64, i64::wrapping_add)
        .wrapping_add(i64::from(skill).wrapping_mul(100_000));
    let factor = 100_000_i64
        .wrapping_add(d.organisation.wrapping_mul(defines.org) / 100_000)
        .wrapping_add(d.hit_points.wrapping_mul(defines.strength) / 100_000);
    (factor.wrapping_mul(stats) / 100_000).wrapping_mul(100) / 100_000
}

/// 1409C8D50 returns the enemy fraction; its tooltip substitutes the complement.
fn enemy_ratio(power: [i64; 2]) -> i64 {
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

fn power(
    content: &super::Content,
    frame: &DecodedFrame,
    viewer_raw: u32,
    links: &BTreeMap<u32, u32>,
    state: u32,
    states: &BTreeMap<u32, &replay::rows::StateRow>,
    countries: &BTreeMap<u32, &replay::rows::CountryRow>,
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
                }; // native class !=0
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
                        // GetSkillLevel@140C0A4A0 reads +0x1B8 without the +0x18 validity gate.
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
            // Native controller collection is outside the unit-loop/nonempty gate.
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

/// Numeric getter used by fresh attrition; the optional native text pointer is null.
pub(super) fn ratio(
    content: &super::Content,
    frame: &DecodedFrame,
    country: u32,
    state: u32,
    links: &BTreeMap<u32, u32>,
) -> Result<i64, String> {
    let flag = frame
        .complete::<tables::StateFlags>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.state == state)
        .ok_or("border state active predicate absent")?;
    if *flag
        .border_conflict_active
        .require("StateFlags.border_conflict_active")?
        == 0
    {
        return Ok(0);
    }
    let states = frame
        .complete::<tables::States>()
        .map_err(|e| e.to_string())?;
    let states = states.iter().map(|row| (row.id, row)).collect();
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())?;
    let countries = countries
        .iter()
        .map(|row| (row.canonical_slot, row))
        .collect();
    power(content, frame, country, links, state, &states, &countries).map(enemy_ratio)
}

pub(super) fn build(
    content: &super::Content,
    loc: &Localisation,
    frame: &DecodedFrame,
    viewer: u32,
    links: &BTreeMap<u32, u32>,
    state: u32,
    flag: &StateFlagRow,
) -> Result<String, String> {
    if *flag
        .border_conflict_active
        .require("StateFlags.border_conflict_active")?
        == 0
    {
        return Ok(String::new());
    }
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
    let viewer_raw = countries
        .get(&viewer)
        .ok_or("border viewing country absent")?
        .id;
    let hovered = states.get(&state).ok_or("border state absent")?;
    let mut involved = same(links, hovered.controller, viewer_raw)?;
    if !involved {
        'pairs: for pair in flag
            .border_conflict_countries
            .require("StateFlags.border_conflict_countries")?
        {
            for &raw in pair {
                if same(links, raw as u32, viewer_raw)? {
                    involved = true;
                    break 'pairs;
                }
            }
        }
    }
    if !involved {
        return localise(loc, "BORDER_WAR_NOT_OURS", &[]).map(|text| format!("    {text}"));
    }
    let defines = content.border.as_ref().map_err(Clone::clone)?;
    let power = power(
        content, frame, viewer_raw, links, state, &states, &countries,
    )?;
    let share = 100_000_i64.wrapping_sub(enemy_ratio(power));
    let mut text = format!("    {}", localise(loc, "BORDER_WAR_HEADER", &[])?);
    let mut block = String::new();
    // 140DEBC90 appends 140DE8E30's indented block directly after the header.
    // 1409C8D50 leaves a blank line before the ratio and the final hint.
    for (key, value, blank) in [
        ("BORDER_WAR_POWER", power[0], false),
        ("BORDER_WAR_POWER_ENEMY", power[1], true),
        ("BORDER_WAR_POWER_RATIO", share, false),
        ("BORDER_WAR_WIN", defines.victory, false),
        (
            "BORDER_WAR_LOSS",
            100_000_i64.wrapping_sub(defines.victory),
            true,
        ),
    ] {
        block.push_str(&localise(loc, key, &[("VAL", Arg::Fixed5(value))])?);
        block.push('\n');
        if blank {
            block.push('\n');
        }
    }
    block.push_str(&localise(loc, "BORDER_WAR_HINT", &[])?);
    block.push('\n');
    for (index, part) in block.split('\n').enumerate() {
        if index != 0 {
            text.push('\n');
        }
        if !part.is_empty() {
            text.push_str("        "); // 140DE8D90: level 2 is eight spaces.
            text.push_str(part);
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests;
