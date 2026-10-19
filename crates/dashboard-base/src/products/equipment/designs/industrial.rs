//! Industrial conversion for one requested pair, using shared owner operands.
use super::{queries::*, *};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pool {
    pub ic: i64,
    /// Resource tokens in native insertion order, including explicit zeroes.
    pub resources: Vec<(u32, i64)>,
}
impl Pool {
    fn add_resource(&mut self, token: u32, value: i64) {
        if let Some((_, held)) = self.resources.iter_mut().find(|(key, _)| *key == token) {
            *held = held.wrapping_add(value);
        } else {
            self.resources.push((token, value));
        }
    }
    fn add(&mut self, pool: &Self) {
        self.ic = self.ic.wrapping_add(pool.ic);
        for &(token, value) in &pool.resources {
            self.add_resource(token, value);
        }
    }
    fn set_resource(&mut self, token: u32, value: i64) {
        if value == 0 {
            self.resources.retain(|(key, _)| *key != token);
        } else if let Some((_, held)) = self.resources.iter_mut().find(|(key, _)| *key == token) {
            *held = value;
        } else {
            self.resources.push((token, value));
        }
    }
    fn prune(&mut self, maximum: usize, order: &[rows::VariantStatEntry]) {
        while self.resources.len() > maximum {
            let index = self
                .resources
                .iter()
                .enumerate()
                .min_by_key(|(index, (_, value))| (*value, *index))
                .unwrap()
                .0;
            self.resources.remove(index);
        }
        for (_, value) in &mut self.resources {
            *value = i64::from((*value / 100_000) as i32).wrapping_mul(100_000);
        }
        let mut remaining = std::mem::take(&mut self.resources);
        for entry in order {
            if let Some(index) = remaining
                .iter()
                .position(|(token, _)| *token == entry.index)
            {
                self.add_resource(entry.index, remaining.remove(index).1);
            }
        }
        for (token, value) in remaining {
            self.add_resource(token, value);
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndustrialCost {
    pub total: Pool,
    pub conversion: i64,
    pub construction: i64,
    pub dismantling: i64,
    pub multiplier_difference: i64,
    pub construction_factor: i64,
    pub dismantling_factor: i64,
    pub multiplier_delta: i64,
}

fn authored_pool(
    frame: &DecodedFrame,
    ic: i64,
    resources: &[(Vec<u8>, i64)],
) -> Result<Pool, String> {
    let mut pool = Pool {
        ic,
        resources: Vec::new(),
    };
    if resources.is_empty() {
        return Ok(pool);
    }
    let definitions = frame
        .complete::<tables::ResourceDefinitions>()
        .map_err(|e| e.to_string())?;
    for (name, value) in resources {
        let token = definitions
            .iter()
            .find_map(|row| {
                spelling(frame, row.token)
                    .ok()
                    .filter(|token| token.eq_ignore_ascii_case(name))
                    .map(|_| row.token)
            })
            .ok_or_else(|| {
                format!(
                    "industrial resource {} is not recorded (count: 1)",
                    String::from_utf8_lossy(name)
                )
            })?;
        pool.add_resource(token, *value);
    }
    Ok(pool)
}
fn term(terms: &StatGroup, index: usize, additive: bool) -> i64 {
    (if additive {
        &terms.add
    } else {
        &terms.multiply
    })
    .iter()
    .find(|entry| entry.index == index)
    .map_or(0, |entry| entry.value)
}
fn multiplier(
    frame: &DecodedFrame,
    id: u64,
    content: &Database,
    index: usize,
) -> Result<i64, String> {
    let state = definition(frame, id)?
        .design
        .require("VariantDefinitions.design")
        .map_err(|e| e.to_string())?;
    let mut sum = 0_i64;
    if !state.bonus_sources.is_empty() || !state.named_sources.is_empty() {
        let snapshots = frame
            .complete::<tables::VariantBonusSnapshots>()
            .map_err(|e| e.to_string())?;
        for id in state.bonus_sources.iter().chain(&state.named_sources) {
            let source = snapshots
                .iter()
                .find(|row| row.id == *id)
                .ok_or_else(|| format!("industrial source snapshot {id} unavailable (count: 1)"))?;
            if let Some(entry) = source
                .source
                .stats
                .iter()
                .find(|entry| entry.index as usize == index)
            {
                sum = sum.wrapping_add(entry.value);
            }
        }
    }
    for entry in &state.design_team_bonus {
        if installed_content_core::equipment::stat_index(
            &spelling(frame, entry.index)?.to_ascii_lowercase(),
        ) == Some(index)
        {
            sum = sum.wrapping_add(entry.value);
            break;
        }
    }
    for row in frame
        .complete::<tables::VariantUpgrades>()
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|row| row.variant == id)
    {
        let terms = content
            .upgrade_stats(spelling(frame, row.definition_token as u32)?)
            .ok_or_else(|| {
                format!(
                    "industrial upgrade {} terms unavailable (count: 1)",
                    row.definition_token
                )
            })?;
        sum = sum.wrapping_add(term(terms, index, false).wrapping_mul(i64::from(row.level)));
    }
    for row in frame
        .complete::<tables::VariantModules>()
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|row| row.variant == id)
    {
        let terms = content
            .module_stats(spelling(frame, row.definition_token)?)
            .ok_or_else(|| {
                format!(
                    "industrial module {} terms unavailable (count: 1)",
                    row.definition_token
                )
            })?;
        sum = sum.wrapping_add(term(terms, index, false));
    }
    Ok(sum)
}

fn upgrade_resources(
    frame: &DecodedFrame,
    target: u64,
    source: u64,
    content: &Database,
) -> Result<Pool, String> {
    let rows = frame
        .complete::<tables::VariantUpgrades>()
        .map_err(|e| e.to_string())?;
    let a: Vec<_> = rows.iter().filter(|row| row.variant == target).collect();
    let b: Vec<_> = rows.iter().filter(|row| row.variant == source).collect();
    for roster in [&a, &b] {
        if roster
            .windows(2)
            .any(|pair| pair[0].definition_token >= pair[1].definition_token)
        {
            return Err(
                "industrial upgrade roster is not in unique native token order (count: 1)".into(),
            );
        }
    }
    let mut delta = Pool::default();
    let (mut t, mut s) = (0, 0);
    while t < a.len() && s < b.len() {
        let (token, from, to) = match a[t].definition_token.cmp(&b[s].definition_token) {
            std::cmp::Ordering::Less => {
                let row = a[t];
                t += 1;
                (row.definition_token, 0, row.level)
            }
            std::cmp::Ordering::Equal => {
                let next = a[t];
                let old = b[s];
                t += 1;
                s += 1;
                (next.definition_token, old.level, next.level)
            }
            std::cmp::Ordering::Greater => {
                let row = b[s];
                s += 1;
                (row.definition_token, row.level, 0)
            }
        };
        if from == to {
            continue;
        }
        let upgrade = content
            .upgrade_design(spelling(frame, token as u32)?)
            .ok_or_else(|| {
                format!("industrial upgrade {token} resource thresholds unavailable (count: 1)")
            })?;
        let threshold = |level| {
            upgrade
                .resource_thresholds
                .iter()
                .rev()
                .find(|(minimum, _)| *minimum <= level)
        };
        // The resource collector is a definition-indexed array in native
        // resource order. Keep zero/negative deltas until its positive filter.
        for (level, sign) in [(from, -1_i64), (to, 1_i64)] {
            if let Some((_, resources)) = threshold(level) {
                let pool = authored_pool(frame, 0, resources)?;
                for (token, value) in pool.resources {
                    delta.add_resource(token, value.wrapping_mul(sign));
                }
            }
        }
    }
    if !delta.resources.is_empty() {
        let resources = frame
            .complete::<tables::ResourceDefinitions>()
            .map_err(|e| e.to_string())?;
        delta.resources.sort_by_key(|(token, _)| {
            resources
                .iter()
                .find(|row| row.token == *token)
                .map(|row| row.id)
        });
    }
    Ok(delta)
}

/// Native module conversion and designer comparison share this computation.
/// A null organisation is the comparison path. A nonnull organisation requests
/// its current equipment-filtered production conversion bonus from the caller.
pub fn conversion_cost(
    frame: &DecodedFrame,
    target: u64,
    source: u64,
    organisation: Option<u64>,
    content: &Database,
    defines: &installed_content_core::defines::Defines,
    organisation_bonus: impl FnOnce(u64) -> Result<i64, String>,
) -> Result<IndustrialCost, String> {
    if let Some(reason) = &content.lost {
        return Err(reason.clone());
    }
    let target_definition = definition(frame, target)?;
    let source_state = definition(frame, source)?
        .design
        .require("VariantDefinitions.design")
        .map_err(|e| e.to_string())?;
    let target_state = target_definition
        .design
        .require("VariantDefinitions.design")
        .map_err(|e| e.to_string())?;
    let equipment = content
        .named(spelling(frame, target_definition.equipment_token)?)
        .ok_or("industrial target equipment content unavailable (count: 1)")?;
    let build_index = installed_content_core::equipment::stat_index(b"build_cost_ic")
        .ok_or("industrial build-cost metadata unavailable (count: 1)")?;
    let source_multiplier = multiplier(frame, source, content, build_index)?;
    let target_multiplier = multiplier(frame, target, content, build_index)?;
    let mut old_hull = source_state.build_cost;
    let (mut conversion, mut construction, mut dismantling) =
        (Pool::default(), Pool::default(), Pool::default());
    let modules = frame
        .complete::<tables::VariantModules>()
        .map_err(|e| e.to_string())?;
    let module = |token| -> Result<_, String> {
        let name = spelling(frame, token)?;
        Ok((
            name,
            content.module_design(name).ok_or_else(|| {
                format!(
                    "industrial module {} operands unavailable (count: 1)",
                    String::from_utf8_lossy(name)
                )
            })?,
        ))
    };
    for slot in &equipment.module_slots {
        let assignment = |id| -> Result<Option<_>, String> {
            let mut found = None;
            for row in modules.iter().filter(|row| row.variant == id) {
                if spelling(frame, row.slot)?.eq_ignore_ascii_case(slot)
                    && found.replace(row.definition_token).is_some()
                {
                    return Err(
                        "industrial module slot has duplicate assignments (count: 1)".into(),
                    );
                }
            }
            Ok(found)
        };
        let next = assignment(target)?;
        let old = assignment(source)?;
        if next == old {
            continue;
        }
        let next = next.map(module).transpose()?;
        let old = old.map(module).transpose()?;
        if let Some((name, old)) = old {
            let old_terms = content
                .module_stats(name)
                .ok_or("industrial source module stat operands unavailable (count: 1)")?;
            old_hull = old_hull.wrapping_sub(
                source_multiplier.wrapping_add(100_000).wrapping_mul(term(
                    old_terms,
                    build_index,
                    true,
                )) / 100_000,
            );
            if let Some((_, next)) = next
                && let Some(rule) = next.conversion_from(name, old)
            {
                conversion.add(&authored_pool(frame, rule.cost_ic, &rule.resources)?);
                continue;
            }
            dismantling.add(&authored_pool(
                frame,
                old.dismantle_cost_ic.unwrap_or(0),
                &old.dismantle_resources,
            )?);
        }
        if let Some((name, next)) = next {
            construction.add(&authored_pool(
                frame,
                term(
                    content
                        .module_stats(name)
                        .ok_or("industrial target module stat operands unavailable (count: 1)")?,
                    build_index,
                    true,
                ),
                &next.build_resources,
            )?);
        }
    }
    let naval = target_definition.classifier_flags & engine_native::cequipment_type::SHIP_MASK != 0;
    let define = |name| {
        defines
            .fixed5("NProduction", name)
            .map_err(|e| e.to_string())
    };
    let hull_factor = if naval {
        define("BASE_NAVAL_EQUIPMENT_CONVERSION_IC_COST_FACTOR")?
    } else {
        let bonus = match organisation {
            Some(reference) => organisation_bonus(reference)?,
            None => 0,
        };
        let speed = define("CONVERSION_SPEED_BONUS")?
            .wrapping_add(country_modifier(
                frame,
                target_definition.creator_link,
                0xdc,
            )?)
            .wrapping_add(bonus);
        define("BASE_LAND_EQUIPMENT_CONVERSION_IC_COST_FACTOR")?
            .wrapping_sub(speed)
            .max(0)
    };
    conversion.ic = conversion
        .ic
        .wrapping_add(equipment.stats[build_index].wrapping_mul(hull_factor) / 100_000);
    let upgrades = frame
        .complete::<tables::VariantUpgrades>()
        .map_err(|e| e.to_string())?;
    for next in upgrades.iter().filter(|row| row.variant == target) {
        let old = upgrades
            .iter()
            .find(|row| row.variant == source && row.definition_token == next.definition_token)
            .map_or(0, |row| row.level);
        if next.level > old {
            let terms = content
                .upgrade_stats(spelling(frame, next.definition_token as u32)?)
                .ok_or("industrial upgrade stat operands unavailable (count: 1)")?;
            construction.ic =
                construction
                    .ic
                    .wrapping_add(i64::from(next.level - old).wrapping_mul(term(
                        terms,
                        build_index,
                        true,
                    )));
        }
    }
    let refit = if naval {
        country_modifier(frame, target_definition.creator_link, 0x1d6)?
    } else {
        0
    };
    let factor = target_multiplier.wrapping_add(refit);
    conversion.ic = factor
        .wrapping_add(100_000)
        .wrapping_mul(conversion.ic)
        .wrapping_div(100_000)
        .max(0);
    construction.ic = factor
        .wrapping_add(100_000)
        .wrapping_mul(construction.ic)
        .wrapping_div(100_000)
        .max(0);
    dismantling.ic = refit
        .wrapping_add(100_000)
        .wrapping_mul(dismantling.ic)
        .wrapping_div(100_000)
        .max(0);
    let multiplier_delta = target_multiplier.wrapping_sub(source_multiplier).max(0);
    let multiplier_difference = multiplier_delta.wrapping_mul(old_hull) / 100_000;
    let mut total = Pool::default();
    for pool in [&conversion, &construction, &dismantling] {
        total.add(pool);
    }
    total.ic = total.ic.wrapping_add(multiplier_difference);
    for (token, value) in upgrade_resources(frame, target, source, content)?.resources {
        if value > 0 {
            total.add_resource(token, value);
        }
    }
    let floor = define(if naval {
        "MIN_NAVAL_EQUIPMENT_CONVERSION_RESOURCE_COST_FACTOR"
    } else {
        "MIN_LAND_EQUIPMENT_CONVERSION_RESOURCE_COST_FACTOR"
    })?;
    if floor != 0 {
        for entry in &target_state.build_resources {
            let held = total
                .resources
                .iter()
                .find(|(token, _)| *token == entry.index)
                .map_or(0, |(_, value)| *value);
            total.set_resource(
                entry.index,
                held.max(entry.value.wrapping_mul(floor) / 100_000),
            );
        }
    }
    let maximum = defines
        .int32("NProduction", "MAX_EQUIPMENT_RESOURCES_NEED")
        .map_err(|e| e.to_string())?;
    let maximum =
        usize::try_from(maximum).map_err(|_| "negative industrial resource limit (count: 1)")?;
    total.prune(maximum, &target_state.build_resources);
    Ok(IndustrialCost {
        total,
        conversion: conversion.ic,
        construction: construction.ic,
        dismantling: dismantling.ic,
        multiplier_difference,
        construction_factor: factor,
        dismantling_factor: refit,
        multiplier_delta,
    })
}

#[cfg(test)]
mod tests;
