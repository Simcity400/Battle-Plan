use engine_executable::modifier_keys as keys;
use content_reader_localisation::localisation::Argument;
use content_reader_localisation::localisation::Localisation;
use replay_data::{
    rows::CountryCounterInputRow, rows::NavalCombatShipRow, rows::NavalCombatSideDetails,
    rows::NavalShipModifierSource, rows::NavalShipWeaponContext, rows::ShipModelRow, rows::ShipRow,
    rows::Recorded, rows::TaskForceRow, rows::WeatherRegionRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
use viewer_engagements::engaged::Engaged;
use viewer_engagements::engagement_history::CombatFlags;
use viewer_foundation::perspective::Vision;
use viewer_foundation::tooltip_text::ensure_newlines;
use viewer_foundation::tooltip_text::tooltip_keys;
use viewer_foundation::tooltip_text::viewing_tag as tooltip_text_viewing_tag;
use super::content::Content as StaticContent;
use viewer_foundation::tables::rows;
use {
    super::{
        Values, Widget,
        binding::{Bindings, text},
        keyed::Keyed,
        modifier_text::{Block, Definitions},
        tooltips::{hover_text, set_hover},
    },
    viewer_foundation::tooltip_text::{TextIcons, builtin, token},
};

const ONE: i64 = 100_000;
fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}
fn add(a: i64, b: i64) -> i64 {
    a.wrapping_add(b)
}

fn term(
    context: &NavalShipWeaponContext,
    source: &NavalShipModifierSource,
    key: u32,
) -> Option<i64> {
    let Some(term) = source.terms.iter().find(|t| t.key == key) else {
        return Some(0);
    };
    if term.feature_selector != 0 {
        let eligibility = context.sources.iter().find(|s| s.scope == 26)?;
        match eligibility.weights[1] {
            0 => return Some(0),
            1 if term.feature_selector & eligibility.weights[0] as u32 == 0 => return Some(0),
            1 => (),
            _ => return None,
        }
    }
    Some(if term.metadata_flags & 0x10 != 0 {
        term.raw.clamp(0, ONE)
    } else {
        term.raw
    })
}
fn source_term(context: &NavalShipWeaponContext, scope: u8, key: u32) -> Option<i64> {
    context
        .sources
        .iter()
        .filter(|s| s.scope == scope)
        .try_fold(0, |sum, source| Some(add(sum, term(context, source, key)?)))
}

pub(super) fn modifier(context: &NavalShipWeaponContext, key: u32) -> Option<i64> {
    modifier_with_leader(context, key, false)
}

pub(super) fn modifier_default_leader(context: &NavalShipWeaponContext, key: u32) -> Option<i64> {
    modifier_with_leader(context, key, true)
}

pub(super) fn breakdown_scale(
    context: &NavalShipWeaponContext,
    scope: u8,
    index: u32,
) -> Option<i64> {
    if scope == 11 {
        let source = context
            .sources
            .iter()
            .find(|s| s.scope == scope && s.index == index)?;
        return Some(source.weights[0].wrapping_mul(ONE));
    }
    if scope != 4 {
        return Some(ONE);
    }
    let source = context
        .sources
        .iter()
        .find(|s| s.scope == scope && s.index == index)?;
    let [achieved, possible] = source.weights;
    let progress =
        add(ONE, source_term(context, 0, source.dynamic_keys[0])?).wrapping_mul(achieved);
    Some(if possible == 0 {
        0
    } else {
        progress.checked_div(possible)?.clamp(0, ONE)
    })
}

pub(super) fn breakdown_visible(
    context: &NavalShipWeaponContext,
    key: u32,
    scope: u8,
    index: u32,
) -> Option<bool> {
    if !matches!(scope, 12..=15) {
        return Some(true);
    }
    let mut totals = BTreeMap::<u32, i64>::new();
    for source in context
        .sources
        .iter()
        .filter(|s| matches!(s.scope, 12..=15))
    {
        let total = totals.entry(source.index).or_default();
        *total = total.wrapping_add(term(context, source, key)?);
    }
    let mut best = 0;
    let mut winner = None;
    for (candidate, value) in totals {
        if value > best {
            best = value;
            winner = Some(candidate);
        }
    }
    Some(winner == Some(index))
}

fn modifier_with_leader(
    context: &NavalShipWeaponContext,
    key: u32,
    default_leader: bool,
) -> Option<i64> {
    let mut sum = 0;
    let mut influences = BTreeMap::<u32, i64>::new();
    for source in &context.sources {
        let value = match source.scope {
            0..=3 | 5 | 7..=8 => term(context, source, key)?,
            9 | 10 if !default_leader => term(context, source, key)?,
            31 | 32 if default_leader => term(context, source, key)?,
            4 => {
                let [achieved, possible] = source.weights;
                let progress = add(ONE, source_term(context, 0, source.dynamic_keys[0])?)
                    .wrapping_mul(achieved);
                let weight = if possible == 0 {
                    0
                } else {
                    progress.checked_div(possible)?.clamp(0, ONE)
                };
                mul(
                    mul(
                        term(context, source, key)?,
                        add(ONE, source_term(context, 0, source.dynamic_keys[1])?),
                    ),
                    weight,
                )
            }
            6 => {
                let weather =
                    add(ONE, source_term(context, 0, keys::NAVY_WEATHER_PENALTY)?).clamp(0, ONE);
                mul(
                    term(context, source, key)?,
                    add(context.weather_factor - ONE, weather),
                )
            }
            11 => {
                let severity = if key == keys::NAVAL_CRITICAL_EFFECT_FACTOR {
                    ONE
                } else {
                    add(
                        ONE,
                        modifier_with_leader(
                            context,
                            keys::NAVAL_CRITICAL_EFFECT_FACTOR,
                            default_leader,
                        )?,
                    )
                };
                mul(
                    term(context, source, key)?.wrapping_mul(source.weights[0]),
                    severity,
                )
            }
            12..=15 => {
                let value = term(context, source, key)?;
                let sum = influences.entry(source.index).or_default();
                *sum = add(*sum, value);
                0
            }
            9 | 10 | 16 | 20..=27 | 31 | 32 => 0,
            _ => return None,
        };
        sum = add(sum, value);
    }
    Some(add(
        sum,
        influences.values().copied().max().unwrap_or(0).max(0),
    ))
}
fn adjuster(context: &NavalShipWeaponContext, scope: u8, lane: usize) -> i64 {
    context
        .sources
        .iter()
        .filter(|s| s.scope == scope)
        .fold(0, |sum, s| add(sum, s.weights[lane]))
}

fn weapon(row: &NavalCombatShipRow, case: usize, pride_in_battle: bool) -> Option<i64> {
    if row.live_ship == 0 {
        return (row.live_convoy == 0).then_some(0);
    }
    let context = &row.weapon_context;
    if !context.sources.iter().any(|s| s.scope == 26) {
        return None;
    }
    let depth = case == 4;
    let lane = usize::from(depth);
    let mut base = context.weapon_bases[case];
    let mut factor = 0;
    if context.formation_flags & (1 << 7) != 0 {
        if !depth {
            base = add(base, modifier(context, keys::SUBMARINE_ATTACK)?);
        }
        factor = add(
            modifier(
                context,
                [
                    keys::NAVY_SUBMARINE_ATTACK_FACTOR,
                    keys::NAVY_SUBMARINE_DEFENCE_FACTOR,
                ][lane],
            )?,
            source_term(
                context,
                16,
                [
                    keys::NAVY_SUBMARINE_ATTACK_FACTOR_AGAINST,
                    keys::NAVY_SUBMARINE_DEFENCE_FACTOR_AGAINST,
                ][lane],
            )?,
        );
    }
    for (mask, key, relation) in [
        (
            1 << 6,
            [
                keys::NAVY_CAPITAL_SHIP_ATTACK_FACTOR,
                keys::NAVY_CAPITAL_SHIP_DEFENCE_FACTOR,
            ],
            [
                keys::NAVY_CAPITAL_SHIP_ATTACK_FACTOR_AGAINST,
                keys::NAVY_CAPITAL_SHIP_DEFENCE_FACTOR_AGAINST,
            ],
        ),
        (
            1 << 8,
            [
                keys::NAVY_SCREEN_ATTACK_FACTOR,
                keys::NAVY_SCREEN_DEFENCE_FACTOR,
            ],
            [
                keys::NAVY_SCREEN_ATTACK_FACTOR_AGAINST,
                keys::NAVY_SCREEN_DEFENCE_FACTOR_AGAINST,
            ],
        ),
    ] {
        if context.formation_flags & mask != 0 {
            factor = add(
                factor,
                add(
                    modifier(context, key[lane])?,
                    source_term(context, 16, relation[lane])?,
                ),
            );
        }
    }
    if base == 0 {
        return Some(0);
    }
    if !depth {
        factor = add(
            factor,
            mul(modifier(context, keys::NAVAL_NIGHT_ATTACK)?, context.night),
        );
    }
    factor = add(
        factor,
        modifier(
            context,
            [keys::NAVAL_DAMAGE_FACTOR, keys::NAVAL_DEFENSE_FACTOR][lane],
        )?,
    );
    if (context.ship_class as i32) < 0 {
        let fleet: Vec<_> = context.sources.iter().filter(|s| s.scope == 25).collect();
        if !fleet.is_empty() {
            factor = add(
                factor,
                fleet.iter().fold(0_i64, |sum, s| add(sum, s.weights[lane])) / fleet.len() as i64,
            );
        }
    } else {
        factor = add(factor, adjuster(context, 20, lane));
    }
    factor = add(factor, adjuster(context, 21, lane));
    factor = add(factor, mul(adjuster(context, 23, lane), context.night));
    factor = add(factor, adjuster(context, 22, lane));
    if pride_in_battle {
        factor = add(
            factor,
            source_term(
                context,
                9,
                [
                    keys::NAVAL_HAS_POTF_IN_COMBAT_ATTACK,
                    keys::NAVAL_HAS_POTF_IN_COMBAT_DEFENSE,
                ][lane],
            )?,
        );
    }
    if !depth {
        let fuel = if context.fuel[1] < 1 {
            ONE
        } else {
            (context.fuel[0].wrapping_mul(ONE) / context.fuel[1]).clamp(0, ONE)
        };
        if fuel < ONE {
            factor = add(
                factor,
                mul(ONE - fuel, adjuster(context, 24, usize::from(case == 3))),
            );
        }
    }
    Some(mul(add(ONE, factor), base).max(0))
}

pub(super) type ConvoyBound = (Result<[i64; 2], String>, Vec<String>);

const CONVOY_WEAPON_STATS: [&str; 4] = ["lg_attack", "hg_attack", "torpedo_attack", "sub_attack"];

#[cfg(test)]
fn convoy_weapon_bound(layers: &content_reader_layers::ResolvedLayers) -> io::Result<ConvoyBound> {
    Ok(convoy_weapon_bound_from_database(
        &content_reader_key_names::equipment::Database::load(layers)?,
    ))
}

pub(super) fn convoy_weapon_bound_from_database(
    database: &content_reader_key_names::equipment::Database,
) -> ConvoyBound {
    use content_reader_key_names::equipment::{StatGroup, StatValue};
    let misreads = database.misreads.clone();
    if let Some(lost) = &database.lost {
        return (Err(lost.clone()), misreads);
    }
    let stats: Vec<usize> = CONVOY_WEAPON_STATS
        .iter()
        .filter_map(|name| content_reader_script::equipment_stats::stat_index(name.as_bytes()))
        .collect();
    let group_touches_weapons = |group: &StatGroup| {
        group
            .add
            .iter()
            .chain(&group.average)
            .chain(&group.multiply)
            .any(|term| stats.contains(&term.index))
            || group.unread.iter().any(|(index, _)| stats.contains(index))
    };
    let stat_term = |values: &[StatValue], index: usize| {
        values
            .iter()
            .find(|entry| entry.index == index)
            .map_or(0_i64, |entry| entry.value)
    };
    let mut range = [0_i64; 2];
    for equipment in database
        .equipment
        .iter()
        .filter(|equipment| database.category(equipment) & 1 != 0)
    {
        let mut upgrade_touches_weapons = false;
        let mut cursor = Some(equipment);
        let mut seen_archetypes = BTreeSet::new();
        while let Some(current) = cursor {
            if current.upgrades.iter().any(|upgrade| {
                database
                    .upgrade_stats(upgrade)
                    .is_some_and(group_touches_weapons)
            }) {
                upgrade_touches_weapons = true;
                break;
            }
            cursor = current
                .archetype
                .filter(|&at| seen_archetypes.insert(at))
                .map(|at| &database.equipment[at]);
        }
        let feeding = bound_inputs(&stats, equipment);
        let unread_default = equipment
            .default_modules
            .iter()
            .filter_map(|(_, module)| database.module_stats(module))
            .flat_map(|group| &group.unread);
        let lineage = match database.lineage(equipment) {
            Ok(lineage) => lineage,
            Err(why) => return (Err(why), misreads),
        };
        let unread_upgrades = lineage
            .into_iter()
            .flat_map(|held| &held.upgrades)
            .filter_map(|upgrade| database.upgrade_stats(upgrade))
            .flat_map(|group| &group.unread);
        let unread_scaling = equipment
            .default_modules
            .iter()
            .filter_map(|(_, module)| database.module_stats(module))
            .flat_map(|group| equipment.unread_multipliers_for(group));
        let unread = equipment.unread_terms.iter().chain(unread_default).chain(unread_upgrades).map(|(index, why)| (*index, why.as_str())).chain(unread_scaling);
        if let Some((_, why)) = unread.into_iter().find(|(index, _)| feeding.contains(index)) {
            return (
                Err(format!("convoy equipment {}: {why}", String::from_utf8_lossy(&equipment.name))),
                misreads,
            );
        }
        if upgrade_touches_weapons {
            return (
                Err(format!(
                    "convoy equipment {} admits weapon-modifying upgrades, so convoy weapon bounds cannot be determined from equipment types alone",
                    String::from_utf8_lossy(&equipment.name)
                )),
                misreads,
            );
        }
        let has_dynamic_terms = !equipment.module_add_stats_multiplier.is_empty()
            || !equipment.module_multiply_stats_multiplier.is_empty()
            || !equipment.scaled_by_reliability.is_empty();
        let default_weapon_modules: Vec<&StatGroup> = equipment
            .default_modules
            .iter()
            .filter_map(|(_, module)| database.module_stats(module))
            .filter(|group| group_touches_weapons(group))
            .collect();
        let mut slot_allows_weapon_module = false;
        if !equipment.module_slots.is_empty() {
            let mut slot_categories: BTreeMap<Vec<u8>, BTreeSet<Vec<u8>>> = BTreeMap::new();
            for (slot_name, rule) in equipment
                .module_slots
                .iter()
                .zip(&equipment.module_slot_rules)
            {
                slot_categories
                    .entry(slot_name.to_ascii_lowercase())
                    .or_default()
                    .extend(rule.categories.iter().cloned());
            }
            let mut reachable: BTreeSet<Vec<u8>> = equipment
                .default_modules
                .iter()
                .filter_map(|(_, module)| {
                    database.module(module).map(|name| name.to_ascii_lowercase())
                })
                .collect();
            loop {
                let mut changed = false;
                for (name, design, group) in database.all_module_designs() {
                    let allowed = reachable.contains(name)
                        || slot_categories
                            .values()
                            .any(|categories| categories.contains(&design.category));
                    if !allowed {
                        continue;
                    }
                    if group.is_some_and(group_touches_weapons) {
                        slot_allows_weapon_module = true;
                        break;
                    }
                    if reachable.insert(name.to_vec()) {
                        changed = true;
                    }
                    for (slot, category) in &design.allowed_module_categories {
                        if let Some(categories) = slot_categories.get_mut(slot)
                            && categories.insert(category.clone())
                        {
                            changed = true;
                        }
                    }
                }
                if slot_allows_weapon_module || !changed {
                    break;
                }
            }
        }
        if !equipment.module_slots.is_empty()
            && (has_dynamic_terms || !default_weapon_modules.is_empty() || slot_allows_weapon_module)
        {
            return (
                Err(format!(
                    "convoy equipment {} defines module slots whose modules or dynamic stat terms can change convoy weapon stats, so convoy weapon bounds cannot be determined from equipment types alone",
                    String::from_utf8_lossy(&equipment.name)
                )),
                misreads,
            );
        }
        for &stat in &stats {
            let stat_range = (|| -> Option<[i64; 2]> {
                let add_factor =
                    100_000_i64.checked_add(stat_term(&equipment.module_add_stats_multiplier, stat))?;
                let mul_factor = 100_000_i64
                    .checked_add(stat_term(&equipment.module_multiply_stats_multiplier, stat))?;
                let mut pure_add_range = [equipment.stats[stat], equipment.stats[stat]];
                let mut pure_mul_range = [0_i64, 0_i64];
                let mut pure_avg_by_ratio: BTreeMap<(i64, i64), (i64, i64)> = BTreeMap::new();
                let mut pure_avg_exact_by_key: BTreeMap<i64, usize> = BTreeMap::new();
                let mut pure_avg_pos_sum = 0_i64;
                let mut pure_avg_neg_sum = 0_i64;
                struct CorrelatedGroup {
                    add: i64,
                    mul: i64,
                    avg_sum: i64,
                    avg_cnt: i64,
                    count: usize,
                    avg_pos_sum: i64,
                    avg_neg_sum: i64,
                }
                let mut correlated_by_key: BTreeMap<(i64, i64, i64, i64), CorrelatedGroup> =
                    BTreeMap::new();
                for (_, module) in &equipment.default_modules {
                    let Some(group) = database.module_stats(module) else {
                        continue;
                    };
                    let mut mod_add = 0_i64;
                    for term in group.add.iter().filter(|term| term.index == stat) {
                        let delta = add_factor.checked_mul(term.value)? / 100_000;
                        mod_add = mod_add.checked_add(delta)?;
                    }
                    let mut mod_mul = 0_i64;
                    for term in group.multiply.iter().filter(|term| term.index == stat) {
                        let delta = mul_factor.checked_mul(term.value)? / 100_000;
                        mod_mul = mod_mul.checked_add(delta)?;
                    }
                    let mut mod_avg = Vec::new();
                    for term in group.average.iter().filter(|term| term.index == stat) {
                        let delta = add_factor.checked_mul(term.value)? / 100_000;
                        mod_avg.push(delta);
                    }
                    if mod_add == 0 && mod_mul == 0 && mod_avg.is_empty() {
                        continue;
                    }
                    if mod_add == 0 && mod_mul == 0 {
                        let mut mod_avg_sum = 0_i64;
                        for &delta in &mod_avg {
                            mod_avg_sum = mod_avg_sum.checked_add(delta)?;
                            if delta > 0 {
                                pure_avg_pos_sum = pure_avg_pos_sum.checked_add(delta)?;
                                pure_avg_pos_sum.checked_mul(100_000)?;
                            } else if delta < 0 {
                                pure_avg_neg_sum = pure_avg_neg_sum.checked_add(delta)?;
                                pure_avg_neg_sum.checked_mul(100_000)?;
                            }
                        }
                        let mod_avg_cnt = i64::try_from(mod_avg.len()).ok()?;
                        mod_avg_sum.checked_mul(100_000)?;
                        mod_avg_cnt.checked_mul(100_000)?;
                        let mut a = mod_avg_sum.unsigned_abs();
                        let mut b = mod_avg_cnt as u64;
                        while b != 0 {
                            let r = a % b;
                            a = b;
                            b = r;
                        }
                        let g = i64::try_from(a.max(1)).ok()?;
                        let entry = pure_avg_by_ratio
                            .entry((mod_avg_sum / g, mod_avg_cnt / g))
                            .or_insert((0, 0));
                        entry.0 = entry.0.checked_add(mod_avg_sum)?;
                        entry.1 = entry.1.checked_add(mod_avg_cnt)?;
                        entry.0.checked_mul(100_000)?;
                        entry.1.checked_mul(100_000)?;
                        let exact = pure_avg_exact_by_key.entry(mod_avg_sum).or_insert(0);
                        *exact = exact.checked_add(1)?;
                        continue;
                    }
                    if mod_mul == 0 && mod_avg.is_empty() {
                        pure_add_range = [
                            pure_add_range[0].checked_add(mod_add.min(0))?,
                            pure_add_range[1].checked_add(mod_add.max(0))?,
                        ];
                    } else if mod_add == 0 && mod_avg.is_empty() {
                        pure_mul_range = [
                            pure_mul_range[0].checked_add(mod_mul.min(0))?,
                            pure_mul_range[1].checked_add(mod_mul.max(0))?,
                        ];
                    } else {
                        let mut mod_avg_sum = 0_i64;
                        let mut mod_avg_pos = 0_i64;
                        let mut mod_avg_neg = 0_i64;
                        for &delta in &mod_avg {
                            mod_avg_sum = mod_avg_sum.checked_add(delta)?;
                            if delta > 0 {
                                mod_avg_pos = mod_avg_pos.checked_add(delta)?;
                                mod_avg_pos.checked_mul(100_000)?;
                            } else if delta < 0 {
                                mod_avg_neg = mod_avg_neg.checked_add(delta)?;
                                mod_avg_neg.checked_mul(100_000)?;
                            }
                        }
                        let mod_avg_cnt = i64::try_from(mod_avg.len()).ok()?;
                        mod_avg_sum.checked_mul(100_000)?;
                        mod_avg_cnt.checked_mul(100_000)?;
                        let entry = correlated_by_key
                            .entry((mod_add, mod_mul, mod_avg_sum, mod_avg_cnt))
                            .or_insert(CorrelatedGroup {
                                add: mod_add,
                                mul: mod_mul,
                                avg_sum: mod_avg_sum,
                                avg_cnt: mod_avg_cnt,
                                count: 0,
                                avg_pos_sum: 0,
                                avg_neg_sum: 0,
                            });
                        entry.count = entry.count.checked_add(1)?;
                        entry.avg_pos_sum = entry.avg_pos_sum.checked_add(mod_avg_pos)?;
                        entry.avg_pos_sum.checked_mul(100_000)?;
                        entry.avg_neg_sum = entry.avg_neg_sum.checked_add(mod_avg_neg)?;
                        entry.avg_neg_sum.checked_mul(100_000)?;
                    }
                }
                let mut pure_avg_groups: Vec<(i64, i64)> = pure_avg_by_ratio.into_values().collect();
                pure_avg_groups.sort_by(|&(a_sum, a_cnt), &(b_sum, b_cnt)| {
                    (i128::from(a_sum) * i128::from(b_cnt))
                        .cmp(&(i128::from(b_sum) * i128::from(a_cnt)))
                });
                let correlated_groups: Vec<CorrelatedGroup> =
                    correlated_by_key.into_values().collect();
                for term in equipment
                    .scaled_by_reliability
                    .iter()
                    .filter(|term| term.index == stat)
                {
                    term.value.checked_mul(100_000)?;
                    pure_add_range = [
                        pure_add_range[0].checked_add(term.value.min(0))?,
                        pure_add_range[1].checked_add(term.value.max(0))?,
                    ];
                }
                let total_states = correlated_groups
                    .iter()
                    .try_fold(1_usize, |acc, group| acc.checked_mul(group.count.checked_add(1)?));
                if let Some(states) = total_states
                    && states <= 4096
                {
                    let mut best: Option<[i64; 2]> = None;
                    for state in 0..states {
                        let mut rem = state;
                        let mut corr_add = 0_i64;
                        let mut corr_mul = 0_i64;
                        let mut avg_sum = 0_i64;
                        let mut avg_count = 0_i64;
                        for group in &correlated_groups {
                            let radix = group.count + 1;
                            let k = i64::try_from(rem % radix).ok()?;
                            rem /= radix;
                            if k == 0 {
                                continue;
                            }
                            corr_add = corr_add.checked_add(group.add.checked_mul(k)?)?;
                            corr_mul = corr_mul.checked_add(group.mul.checked_mul(k)?)?;
                            avg_sum = avg_sum.checked_add(group.avg_sum.checked_mul(k)?)?;
                            avg_count = avg_count.checked_add(group.avg_cnt.checked_mul(k)?)?;
                        }
                        let base_avg = if avg_count > 0 {
                            avg_sum.checked_mul(100_000)? / avg_count.checked_mul(100_000)?
                        } else {
                            0
                        };
                        let mut avg_bounds = [base_avg, base_avg];
                        let mut pref_sum = avg_sum;
                        let mut pref_cnt = avg_count;
                        for &(g_sum, g_cnt) in &pure_avg_groups {
                            pref_sum = pref_sum.checked_add(g_sum)?;
                            pref_cnt = pref_cnt.checked_add(g_cnt)?;
                            let val =
                                pref_sum.checked_mul(100_000)? / pref_cnt.checked_mul(100_000)?;
                            avg_bounds = [avg_bounds[0].min(val), avg_bounds[1].max(val)];
                        }
                        let mut suff_sum = avg_sum;
                        let mut suff_cnt = avg_count;
                        for &(g_sum, g_cnt) in pure_avg_groups.iter().rev() {
                            suff_sum = suff_sum.checked_add(g_sum)?;
                            suff_cnt = suff_cnt.checked_add(g_cnt)?;
                            let val =
                                suff_sum.checked_mul(100_000)? / suff_cnt.checked_mul(100_000)?;
                            avg_bounds = [avg_bounds[0].min(val), avg_bounds[1].max(val)];
                        }
                        let value_range = [
                            pure_add_range[0]
                                .checked_add(corr_add)?
                                .checked_add(avg_bounds[0])?,
                            pure_add_range[1]
                                .checked_add(corr_add)?
                                .checked_add(avg_bounds[1])?,
                        ];
                        let factor_range = [
                            pure_mul_range[0].checked_add(corr_mul)?.checked_add(100_000)?,
                            pure_mul_range[1].checked_add(corr_mul)?.checked_add(100_000)?,
                        ];
                        let corners = [
                            value_range[0].checked_mul(factor_range[0])? / 100_000,
                            value_range[0].checked_mul(factor_range[1])? / 100_000,
                            value_range[1].checked_mul(factor_range[0])? / 100_000,
                            value_range[1].checked_mul(factor_range[1])? / 100_000,
                        ];
                        let low = *corners.iter().min().unwrap();
                        let high = *corners.iter().max().unwrap();
                        best = Some(match best {
                            Some([lo, hi]) => [lo.min(low), hi.max(high)],
                            None => [low, high],
                        });
                    }
                    return best;
                }
                let (mul_coupled, non_mul): (Vec<_>, Vec<_>) = correlated_groups
                    .into_iter()
                    .partition(|group| group.mul != 0);
                let mut avg_pos_sum = pure_avg_pos_sum;
                let mut avg_neg_sum = pure_avg_neg_sum;
                let mut total_avg_cnt = 0_i64;
                let mut pure_avg_ratio_range: Option<[i64; 2]> = None;
                for &(g_sum, g_cnt) in &pure_avg_groups {
                    total_avg_cnt = total_avg_cnt.checked_add(g_cnt)?;
                    total_avg_cnt.checked_mul(100_000)?;
                    let ratio = g_sum.checked_mul(100_000)? / g_cnt.checked_mul(100_000)?;
                    pure_avg_ratio_range = Some(match pure_avg_ratio_range {
                        Some([lo, hi]) => [lo.min(ratio), hi.max(ratio)],
                        None => [ratio, ratio],
                    });
                }
                let mut avg_add_groups: Vec<AvgAddGroup> =
                    Vec::with_capacity(non_mul.len() + pure_avg_exact_by_key.len());
                for (avg, count) in pure_avg_exact_by_key {
                    avg_add_groups.push(AvgAddGroup {
                        add: 0,
                        avg,
                        count,
                    });
                }
                let mut non_mul_cases: Vec<([i64; 2], Option<[i64; 2]>)> =
                    vec![([0, 0], None)];
                if !non_mul.is_empty() {
                    let mut neg_sum = 0_i64;
                    let mut pos_sum = 0_i64;
                    let mut has_neg = false;
                    let mut has_pos = false;
                    let mut min_single = i64::MAX;
                    let mut max_single = i64::MIN;
                    let mut nm_avg_range: Option<[i64; 2]> = None;
                    for group in &non_mul {
                        let count = i64::try_from(group.count).ok()?;
                        let total_add = group.add.checked_mul(count)?;
                        if group.add < 0 {
                            has_neg = true;
                            neg_sum = neg_sum.checked_add(total_add)?;
                        } else if group.add > 0 {
                            has_pos = true;
                            pos_sum = pos_sum.checked_add(total_add)?;
                        }
                        min_single = min_single.min(group.add);
                        max_single = max_single.max(group.add);
                        avg_pos_sum = avg_pos_sum.checked_add(group.avg_pos_sum)?;
                        avg_pos_sum.checked_mul(100_000)?;
                        avg_neg_sum = avg_neg_sum.checked_add(group.avg_neg_sum)?;
                        avg_neg_sum.checked_mul(100_000)?;
                        total_avg_cnt =
                            total_avg_cnt.checked_add(group.avg_cnt.checked_mul(count)?)?;
                        total_avg_cnt.checked_mul(100_000)?;
                        let ratio =
                            group.avg_sum.checked_mul(100_000)? / group.avg_cnt.checked_mul(100_000)?;
                        nm_avg_range = Some(match nm_avg_range {
                            Some([lo, hi]) => [lo.min(ratio), hi.max(ratio)],
                            None => [ratio, ratio],
                        });
                        avg_add_groups.push(AvgAddGroup {
                            add: group.add,
                            avg: group.avg_sum,
                            count: group.count,
                        });
                    }
                    let min_nonempty_add = if has_neg { neg_sum } else { min_single };
                    let max_nonempty_add = if has_pos { pos_sum } else { max_single };
                    non_mul_cases.push(([min_nonempty_add, max_nonempty_add], nm_avg_range));
                }
                let mul_states = mul_coupled
                    .iter()
                    .try_fold(1_usize, |acc, group| acc.checked_mul(group.count.checked_add(1)?));
                if let Some(states) = mul_states
                    && states <= 4096
                {
                    let mut coupled_cache: BTreeMap<(i64, i64), [i64; 2]> = BTreeMap::new();
                    let mut best: Option<[i64; 2]> = None;
                    for state in 0..states {
                        let mut rem = state;
                        let mut corr_add = 0_i64;
                        let mut corr_mul = 0_i64;
                        let mut corr_avg_sum = 0_i64;
                        let mut corr_avg_cnt = 0_i64;
                        let mut mask_pos_sum = avg_pos_sum;
                        let mut mask_neg_sum = avg_neg_sum;
                        let mut mask_total_cnt = total_avg_cnt;
                        for group in &mul_coupled {
                            let radix = group.count + 1;
                            let k = i64::try_from(rem % radix).ok()?;
                            rem /= radix;
                            if k == 0 {
                                continue;
                            }
                            corr_add = corr_add.checked_add(group.add.checked_mul(k)?)?;
                            corr_mul = corr_mul.checked_add(group.mul.checked_mul(k)?)?;
                            corr_avg_sum =
                                corr_avg_sum.checked_add(group.avg_sum.checked_mul(k)?)?;
                            corr_avg_cnt =
                                corr_avg_cnt.checked_add(group.avg_cnt.checked_mul(k)?)?;
                            mask_pos_sum = mask_pos_sum.checked_add(group.avg_pos_sum)?;
                            mask_pos_sum.checked_mul(100_000)?;
                            mask_neg_sum = mask_neg_sum.checked_add(group.avg_neg_sum)?;
                            mask_neg_sum.checked_mul(100_000)?;
                            mask_total_cnt =
                                mask_total_cnt.checked_add(group.avg_cnt.checked_mul(k)?)?;
                            mask_total_cnt.checked_mul(100_000)?;
                        }
                        let coupled = if let Some(&cached) =
                            coupled_cache.get(&(corr_avg_sum, corr_avg_cnt))
                        {
                            cached
                        } else {
                            let allow_heavy = coupled_cache.len() < 16;
                            let computed = coupled_non_mul_bound(
                                &avg_add_groups,
                                corr_avg_sum,
                                corr_avg_cnt,
                                allow_heavy,
                            )?;
                            coupled_cache.insert((corr_avg_sum, corr_avg_cnt), computed);
                            computed
                        };
                        let factor_range = [
                            pure_mul_range[0].checked_add(corr_mul)?.checked_add(100_000)?,
                            pure_mul_range[1].checked_add(corr_mul)?.checked_add(100_000)?,
                        ];
                        let value_range = [
                            pure_add_range[0]
                                .checked_add(corr_add)?
                                .checked_add(coupled[0])?,
                            pure_add_range[1]
                                .checked_add(corr_add)?
                                .checked_add(coupled[1])?,
                        ];
                        let corners = [
                            value_range[0].checked_mul(factor_range[0])? / 100_000,
                            value_range[0].checked_mul(factor_range[1])? / 100_000,
                            value_range[1].checked_mul(factor_range[0])? / 100_000,
                            value_range[1].checked_mul(factor_range[1])? / 100_000,
                        ];
                        let low = *corners.iter().min().unwrap();
                        let high = *corners.iter().max().unwrap();
                        best = Some(match best {
                            Some([lo, hi]) => [lo.min(low), hi.max(high)],
                            None => [low, high],
                        });
                    }
                    return best;
                }
                let mut mul_add_range = [0_i64, 0_i64];
                let mut mul_range = pure_mul_range;
                let mut mul_avg_ratio_range: Option<[i64; 2]> = None;
                for group in &mul_coupled {
                    let count = i64::try_from(group.count).ok()?;
                    let total_add = group.add.checked_mul(count)?;
                    let total_mul = group.mul.checked_mul(count)?;
                    mul_add_range = [
                        mul_add_range[0].checked_add(total_add.min(0))?,
                        mul_add_range[1].checked_add(total_add.max(0))?,
                    ];
                    mul_range = [
                        mul_range[0].checked_add(total_mul.min(0))?,
                        mul_range[1].checked_add(total_mul.max(0))?,
                    ];
                    avg_pos_sum = avg_pos_sum.checked_add(group.avg_pos_sum)?;
                    avg_pos_sum.checked_mul(100_000)?;
                    avg_neg_sum = avg_neg_sum.checked_add(group.avg_neg_sum)?;
                    avg_neg_sum.checked_mul(100_000)?;
                    total_avg_cnt =
                        total_avg_cnt.checked_add(group.avg_cnt.checked_mul(count)?)?;
                    total_avg_cnt.checked_mul(100_000)?;
                    if group.avg_cnt > 0 {
                        let ratio =
                            group.avg_sum.checked_mul(100_000)? / group.avg_cnt.checked_mul(100_000)?;
                        mul_avg_ratio_range = Some(match mul_avg_ratio_range {
                            Some([lo, hi]) => [lo.min(ratio), hi.max(ratio)],
                            None => [ratio, ratio],
                        });
                    }
                }
                let factor_range = [
                    mul_range[0].checked_add(100_000)?,
                    mul_range[1].checked_add(100_000)?,
                ];
                let coupled_zero = coupled_non_mul_bound(&avg_add_groups, 0, 0, true)?;
                let mut value_ranges: Vec<[i64; 2]> = vec![[
                    pure_add_range[0]
                        .checked_add(coupled_zero[0])?
                        .checked_add(mul_add_range[0])?,
                    pure_add_range[1]
                        .checked_add(coupled_zero[1])?
                        .checked_add(mul_add_range[1])?,
                ]];
                if let Some([m_lo, m_hi]) = mul_avg_ratio_range {
                    for &(nm_add, nm_avg) in &non_mul_cases {
                        let mut avg_lo = m_lo;
                        let mut avg_hi = m_hi;
                        if let Some([n_lo, n_hi]) = nm_avg {
                            avg_lo = avg_lo.min(n_lo);
                            avg_hi = avg_hi.max(n_hi);
                        }
                        if let Some([p_lo, p_hi]) = pure_avg_ratio_range {
                            avg_lo = avg_lo.min(p_lo);
                            avg_hi = avg_hi.max(p_hi);
                        }
                        value_ranges.push([
                            pure_add_range[0]
                                .checked_add(nm_add[0])?
                                .checked_add(mul_add_range[0])?
                                .checked_add(avg_lo)?,
                            pure_add_range[1]
                                .checked_add(nm_add[1])?
                                .checked_add(mul_add_range[1])?
                                .checked_add(avg_hi)?,
                        ]);
                    }
                }
                let mut best: Option<[i64; 2]> = None;
                for value_range in value_ranges {
                    let corners = [
                        value_range[0].checked_mul(factor_range[0])? / 100_000,
                        value_range[0].checked_mul(factor_range[1])? / 100_000,
                        value_range[1].checked_mul(factor_range[0])? / 100_000,
                        value_range[1].checked_mul(factor_range[1])? / 100_000,
                    ];
                    let low = *corners.iter().min().unwrap();
                    let high = *corners.iter().max().unwrap();
                    best = Some(match best {
                        Some([lo, hi]) => [lo.min(low), hi.max(high)],
                        None => [low, high],
                    });
                }
                best
            })();
            let Some([low, high]) = stat_range else {
                return (
                    Err(format!(
                        "convoy equipment {} overflows fixed-point weapon stat bounds, so convoy weapon bounds cannot be determined safely",
                        String::from_utf8_lossy(&equipment.name)
                    )),
                    misreads,
                );
            };
            range = [range[0].min(low), range[1].max(high)];
        }
    }
    (Ok(range), misreads)
}

#[derive(Clone, Copy)]
struct AvgAddGroup {
    add: i64,
    avg: i64,
    count: usize,
}

fn floor_div_i128(n: i128, d: i128) -> i128 {
    let q = n / d;
    let r = n % d;
    if r < 0 { q - 1 } else { q }
}

fn ceil_div_i128(n: i128, d: i128) -> i128 {
    let q = n / d;
    let r = n % d;
    if r > 0 { q + 1 } else { q }
}

fn gcd_u128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a.max(1)
}

#[derive(Clone, Copy)]
struct CoupledItem {
    r: i128,
    u: i128,
}

#[derive(Clone, Copy)]
struct NodeQuery {
    need: usize,
    r_req: i128,
    u_cap: i128,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Feasibility {
    Feasible,
    Infeasible,
    Unknown,
}

enum NodeOutcome {
    Feasible,
    Infeasible,
    Unknown,
    Branch { idx: usize, split: usize },
}

struct SearchScratch {
    scored: Vec<(i128, i128, usize)>,
    x_l: Vec<usize>,
    x_r: Vec<usize>,
    x_m: Vec<usize>,
}

fn extremal_selection(
    items: &[CoupledItem],
    caps: &[usize],
    need: usize,
    p: i128,
    q: i128,
    scored: &mut Vec<(i128, i128, usize)>,
    chosen: &mut [usize],
) -> Option<(i128, i128)> {
    scored.clear();
    chosen.fill(0);
    for (idx, (&item, &cap)) in items.iter().zip(caps).enumerate() {
        if cap == 0 {
            continue;
        }
        let key = if q == 0 {
            (-item.u, item.r, idx)
        } else {
            let s = q
                .checked_mul(item.r)?
                .checked_sub(p.checked_mul(item.u)?)?;
            (s, -item.u, idx)
        };
        scored.push(key);
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let mut rem = need;
    let mut sum_r = 0_i128;
    let mut sum_u = 0_i128;
    for &(_, _, idx) in scored.iter().rev() {
        if rem == 0 {
            break;
        }
        let take = rem.min(caps[idx]);
        chosen[idx] = take;
        let take_i = take as i128;
        sum_r = sum_r.checked_add(items[idx].r.checked_mul(take_i)?)?;
        sum_u = sum_u.checked_add(items[idx].u.checked_mul(take_i)?)?;
        rem -= take;
    }
    Some((sum_r, sum_u))
}

fn check_node(
    items: &[CoupledItem],
    caps: &[usize],
    query: NodeQuery,
    scratch: &mut SearchScratch,
) -> NodeOutcome {
    if query.need == 0 {
        return if query.r_req <= 0 && query.u_cap >= 0 {
            NodeOutcome::Feasible
        } else {
            NodeOutcome::Infeasible
        };
    }
    let total_cap: usize = caps.iter().sum();
    if total_cap < query.need {
        return NodeOutcome::Infeasible;
    }
    if total_cap == query.need {
        let mut sum_r = 0_i128;
        let mut sum_u = 0_i128;
        for (&item, &cap) in items.iter().zip(caps) {
            if cap == 0 {
                continue;
            }
            let c = cap as i128;
            let Some(next_r) = item.r.checked_mul(c).and_then(|v| sum_r.checked_add(v)) else {
                return NodeOutcome::Unknown;
            };
            let Some(next_u) = item.u.checked_mul(c).and_then(|v| sum_u.checked_add(v)) else {
                return NodeOutcome::Unknown;
            };
            sum_r = next_r;
            sum_u = next_u;
        }
        return if sum_r >= query.r_req && sum_u <= query.u_cap {
            NodeOutcome::Feasible
        } else {
            NodeOutcome::Infeasible
        };
    }
    let Some((mut r_l, mut u_l)) = extremal_selection(
        items,
        caps,
        query.need,
        0,
        1,
        &mut scratch.scored,
        &mut scratch.x_l,
    ) else {
        return NodeOutcome::Unknown;
    };
    if r_l < query.r_req {
        return NodeOutcome::Infeasible;
    }
    if u_l <= query.u_cap {
        return NodeOutcome::Feasible;
    }
    let Some((mut r_r, mut u_r)) = extremal_selection(
        items,
        caps,
        query.need,
        1,
        0,
        &mut scratch.scored,
        &mut scratch.x_r,
    ) else {
        return NodeOutcome::Unknown;
    };
    if u_r > query.u_cap {
        return NodeOutcome::Infeasible;
    }
    if r_r >= query.r_req {
        return NodeOutcome::Feasible;
    }
    for _ in 0..32 {
        let (Some(p_raw), Some(q_raw)) = (r_l.checked_sub(r_r), u_l.checked_sub(u_r)) else {
            return NodeOutcome::Unknown;
        };
        if p_raw <= 0 || q_raw <= 0 {
            break;
        }
        let g = gcd_u128(p_raw as u128, q_raw as u128) as i128;
        let p = p_raw / g;
        let q = q_raw / g;
        let Some((r_m, u_m)) = extremal_selection(
            items,
            caps,
            query.need,
            p,
            q,
            &mut scratch.scored,
            &mut scratch.x_m,
        ) else {
            return NodeOutcome::Unknown;
        };
        let Some(cert) = r_m
            .checked_sub(query.r_req)
            .and_then(|dr| q.checked_mul(dr))
            .and_then(|qdr| {
                u_m.checked_sub(query.u_cap)
                    .and_then(|du| p.checked_mul(du))
                    .and_then(|pdu| qdr.checked_sub(pdu))
            })
        else {
            return NodeOutcome::Unknown;
        };
        if cert < 0 {
            return NodeOutcome::Infeasible;
        }
        if r_m >= query.r_req && u_m <= query.u_cap {
            return NodeOutcome::Feasible;
        }
        let Some(diff) = r_m
            .checked_sub(r_l)
            .and_then(|dr| q.checked_mul(dr))
            .and_then(|qdr| {
                u_m.checked_sub(u_l)
                    .and_then(|du| p.checked_mul(du))
                    .and_then(|pdu| qdr.checked_sub(pdu))
            })
        else {
            return NodeOutcome::Unknown;
        };
        if diff <= 0 {
            break;
        }
        if u_m > query.u_cap {
            r_l = r_m;
            u_l = u_m;
            std::mem::swap(&mut scratch.x_l, &mut scratch.x_m);
        } else {
            r_r = r_m;
            u_r = u_m;
            std::mem::swap(&mut scratch.x_r, &mut scratch.x_m);
        }
    }
    let mut best_branch: Option<(usize, (u128, u128))> = None;
    for idx in 0..items.len() {
        if caps[idx] == 0 || scratch.x_l[idx] == scratch.x_r[idx] {
            continue;
        }
        let score = (items[idx].u.unsigned_abs(), items[idx].r.unsigned_abs());
        if best_branch.is_none_or(|(_, prev)| score > prev) {
            best_branch = Some((idx, score));
        }
    }
    let Some((idx, _)) = best_branch else {
        return NodeOutcome::Unknown;
    };
    let split = (scratch.x_l[idx] + scratch.x_r[idx]) / 2;
    NodeOutcome::Branch { idx, split }
}

fn search_feasible(
    items: &[CoupledItem],
    caps: &mut [usize],
    query: NodeQuery,
    budget: &mut usize,
    scratch: &mut SearchScratch,
) -> Feasibility {
    if *budget == 0 {
        return Feasibility::Unknown;
    }
    *budget -= 1;
    match check_node(items, caps, query, scratch) {
        NodeOutcome::Feasible => Feasibility::Feasible,
        NodeOutcome::Infeasible => Feasibility::Infeasible,
        NodeOutcome::Unknown => Feasibility::Unknown,
        NodeOutcome::Branch { idx, split } => {
            let orig_cap = caps[idx];
            caps[idx] = split;
            let left = search_feasible(items, caps, query, budget, scratch);
            caps[idx] = orig_cap;
            if left != Feasibility::Infeasible {
                return left;
            }
            let forced = split + 1;
            if forced <= query.need {
                let forced_i = forced as i128;
                let Some(next_r_req) = items[idx]
                    .r
                    .checked_mul(forced_i)
                    .and_then(|v| query.r_req.checked_sub(v))
                else {
                    return Feasibility::Unknown;
                };
                let Some(next_u_cap) = items[idx]
                    .u
                    .checked_mul(forced_i)
                    .and_then(|v| query.u_cap.checked_sub(v))
                else {
                    return Feasibility::Unknown;
                };
                caps[idx] = orig_cap - forced;
                let right = search_feasible(
                    items,
                    caps,
                    NodeQuery {
                        need: query.need - forced,
                        r_req: next_r_req,
                        u_cap: next_u_cap,
                    },
                    budget,
                    scratch,
                );
                caps[idx] = orig_cap;
                return right;
            }
            Feasibility::Infeasible
        }
    }
}

fn prove_sign_split_infeasible(
    groups: &[AvgAddGroup],
    k: usize,
    d: i128,
    base_sum: i128,
    target: i128,
    upper: bool,
) -> bool {
    let Some(limit) = d.checked_mul(target) else {
        return false;
    };
    let (r_req, u_cap) = if upper {
        let Some(r_req) = limit.checked_add(1).and_then(|v| v.checked_sub(base_sum)) else {
            return false;
        };
        let Some(u_cap) = (-1_i128).checked_sub(base_sum) else {
            return false;
        };
        (r_req, u_cap)
    } else {
        let Some(r_req) = base_sum.checked_sub(limit).and_then(|v| v.checked_add(1)) else {
            return false;
        };
        let Some(u_cap) = base_sum.checked_sub(1) else {
            return false;
        };
        (r_req, u_cap)
    };
    let mut items = Vec::with_capacity(groups.len());
    let mut caps = Vec::with_capacity(groups.len());
    for group in groups {
        let Some(r_pos) = d
            .checked_mul(i128::from(group.add))
            .and_then(|v| v.checked_add(i128::from(group.avg)))
        else {
            return false;
        };
        let u_pos = i128::from(group.avg);
        let (r, u) = if upper {
            (r_pos, u_pos)
        } else {
            let (Some(neg_r), Some(neg_u)) = (r_pos.checked_neg(), u_pos.checked_neg()) else {
                return false;
            };
            (neg_r, neg_u)
        };
        items.push(CoupledItem { r, u });
        caps.push(group.count);
    }
    let mut scratch = SearchScratch {
        scored: Vec::with_capacity(groups.len()),
        x_l: vec![0; groups.len()],
        x_r: vec![0; groups.len()],
        x_m: vec![0; groups.len()],
    };
    let mut budget = 4096_usize;
    search_feasible(
        &items,
        &mut caps,
        NodeQuery {
            need: k,
            r_req,
            u_cap,
        },
        &mut budget,
        &mut scratch,
    ) == Feasibility::Infeasible
}

fn coupled_non_mul_bound(
    groups: &[AvgAddGroup],
    base_sum: i64,
    base_cnt: i64,
    allow_heavy: bool,
) -> Option<[i64; 2]> {
    let base_avg = if base_cnt > 0 {
        base_sum.checked_mul(100_000)? / base_cnt.checked_mul(100_000)?
    } else {
        0
    };
    let mut best = [base_avg, base_avg];
    if groups.is_empty() {
        return Some(best);
    }
    let total_cnt = groups
        .iter()
        .try_fold(0_usize, |acc, group| acc.checked_add(group.count))?;
    if total_cnt <= 256 && (allow_heavy || total_cnt.saturating_mul(groups.len()) <= 4096) {
        let mut by_avg: Vec<(i64, usize)> =
            groups.iter().map(|group| (group.avg, group.count)).collect();
        by_avg.sort_by_key(|&(avg, _)| avg);
        let mut weighted: Vec<(i128, usize)> = Vec::with_capacity(groups.len());
        for k in 1..=total_cnt {
            let d = i128::from(base_cnt).checked_add(k as i128)?;
            let mut rem = k;
            let mut u_min = i128::from(base_sum);
            for &(avg, count) in &by_avg {
                if rem == 0 {
                    break;
                }
                let take = rem.min(count);
                u_min = u_min.checked_add(i128::from(avg).checked_mul(take as i128)?)?;
                rem -= take;
            }
            let mut rem = k;
            let mut u_max = i128::from(base_sum);
            for &(avg, count) in by_avg.iter().rev() {
                if rem == 0 {
                    break;
                }
                let take = rem.min(count);
                u_max = u_max.checked_add(i128::from(avg).checked_mul(take as i128)?)?;
                rem -= take;
            }
            weighted.clear();
            for group in groups {
                let w = d
                    .checked_mul(i128::from(group.add))?
                    .checked_add(i128::from(group.avg))?;
                weighted.push((w, group.count));
            }
            weighted.sort_by_key(|&(w, _)| w);
            let mut rem = k;
            let mut r_min = i128::from(base_sum);
            for &(w, count) in &weighted {
                if rem == 0 {
                    break;
                }
                let take = rem.min(count);
                r_min = r_min.checked_add(w.checked_mul(take as i128)?)?;
                rem -= take;
            }
            let mut rem = k;
            let mut r_max = i128::from(base_sum);
            for &(w, count) in weighted.iter().rev() {
                if rem == 0 {
                    break;
                }
                let take = rem.min(count);
                r_max = r_max.checked_add(w.checked_mul(take as i128)?)?;
                rem -= take;
            }
            let lo_floor = floor_div_i128(r_min, d);
            let lo_ceil = ceil_div_i128(r_min, d);
            let lo_k = if u_max <= 0 || lo_floor == lo_ceil {
                lo_ceil
            } else if u_min >= 0 {
                lo_floor
            } else if prove_sign_split_infeasible(
                groups,
                k,
                d,
                i128::from(base_sum),
                lo_ceil,
                false,
            ) {
                lo_ceil
            } else {
                lo_floor
            };
            let hi_floor = floor_div_i128(r_max, d);
            let hi_ceil = ceil_div_i128(r_max, d);
            let hi_k = if u_min >= 0 || hi_floor == hi_ceil {
                hi_floor
            } else if u_max <= 0 {
                hi_ceil
            } else if prove_sign_split_infeasible(
                groups,
                k,
                d,
                i128::from(base_sum),
                hi_floor,
                true,
            ) {
                hi_floor
            } else {
                hi_ceil
            };
            best = [
                best[0].min(i64::try_from(lo_k).ok()?),
                best[1].max(i64::try_from(hi_k).ok()?),
            ];
        }
        return Some(best);
    }
    let mut neg_sum = 0_i64;
    let mut pos_sum = 0_i64;
    let mut has_neg = false;
    let mut has_pos = false;
    let mut min_single = i64::MAX;
    let mut max_single = i64::MIN;
    let mut avg_range: Option<[i64; 2]> = if base_cnt > 0 {
        Some([base_avg, base_avg])
    } else {
        None
    };
    for group in groups {
        let count = i64::try_from(group.count).ok()?;
        let total_add = group.add.checked_mul(count)?;
        if group.add < 0 {
            has_neg = true;
            neg_sum = neg_sum.checked_add(total_add)?;
        } else if group.add > 0 {
            has_pos = true;
            pos_sum = pos_sum.checked_add(total_add)?;
        }
        min_single = min_single.min(group.add);
        max_single = max_single.max(group.add);
        avg_range = Some(match avg_range {
            Some([lo, hi]) => [lo.min(group.avg), hi.max(group.avg)],
            None => [group.avg, group.avg],
        });
    }
    let min_nonempty_add = if has_neg { neg_sum } else { min_single };
    let max_nonempty_add = if has_pos { pos_sum } else { max_single };
    let [avg_lo, avg_hi] = avg_range.unwrap_or([0, 0]);
    Some([
        best[0].min(min_nonempty_add.checked_add(avg_lo)?),
        best[1].max(max_nonempty_add.checked_add(avg_hi)?),
    ])
}

fn convoy_weapon_bound_for_frame(
    static_bound: Result<[i64; 2], &str>,
    database: Option<&content_reader_key_names::equipment::Database>,
    frame: &DecodedFrame,
) -> io::Result<Result<[i64; 2], String>> {
    use content_reader_script::equipment_stats::{STAT_COUNT, stat_index};
    use content_reader_key_names::equipment::{StatComponents, StatGroup};
    use content_reader_parser::parser::c_string;

    let mut range = match static_bound {
        Ok(bound) => bound,
        Err(reason) => return Ok(Err(reason.to_owned())),
    };
    let Some(database) = database else {
        return Ok(Err("equipment database is unavailable".to_owned()));
    };
    let definitions = rows::<tables::VariantDefinitions>(frame)?;
    if !definitions.is_complete() {
        return Ok(Err(
            "VariantDefinitions is incomplete, so convoy variant bonuses and upgrades are unknown"
                .to_owned(),
        ));
    }
    let lexical = rows::<tables::LexicalTokens>(frame)?;
    if !lexical.is_complete() {
        return Ok(Err(
            "LexicalTokens is incomplete, so convoy variant tokens are unknown".to_owned(),
        ));
    }
    let tokens: BTreeMap<u32, &[u8]> = lexical
        .rows()
        .iter()
        .map(|row| (row.token, row.literal.as_slice()))
        .collect();
    let stats: Vec<usize> = CONVOY_WEAPON_STATS
        .iter()
        .filter_map(|name| stat_index(name.as_bytes()))
        .collect();
    let mut upgrades_by_variant: Option<BTreeMap<u64, Vec<&replay_data::rows::VariantUpgradeRow>>> =
        None;
    let mut modules_by_variant: Option<BTreeMap<u64, Vec<&replay_data::rows::VariantModuleRow>>> =
        None;
    let mut bonus_snapshots: Option<BTreeMap<u64, &replay_data::rows::VariantBonusSource>> = None;
    for definition in definitions.rows() {
        let Some(&equipment_name) = tokens.get(&definition.equipment_token) else {
            return Ok(Err(format!(
                "variant {} equipment token {} has no recorded spelling",
                definition.id, definition.equipment_token
            )));
        };
        let Some(kind) = database.named(equipment_name) else {
            return Ok(Err(format!(
                "variant {} equipment {} is absent from recorded content",
                definition.id,
                String::from_utf8_lossy(equipment_name)
            )));
        };
        let is_convoy = database.category(kind) & 1 != 0 || definition.classifier_flags & 1 != 0;
        if !is_convoy {
            continue;
        }
        let feeding = bound_inputs(&stats, kind);
        let unread_weapon_term = |group: &StatGroup| {
            group.unread.iter().find(|(index, _)| feeding.contains(index)).map(|(_, why)| why.clone())
        };
        let state = match definition.design.require("VariantDefinitions.design") {
            Ok(state) => state,
            Err(error) => {
                return Ok(Err(format!(
                    "convoy variant {} design is unavailable: {error}",
                    definition.id
                )));
            }
        };
        if !state.named_bonuses.is_empty() {
            let mut cached = [0_i64; STAT_COUNT];
            let mut seen = [false; STAT_COUNT];
            for entry in &state.unresolved_stats {
                let index = entry.index as usize;
                if index >= STAT_COUNT {
                    return Ok(Err(format!(
                        "convoy variant {} named bonus residual index {index} outside native metadata",
                        definition.id
                    )));
                }
                cached[index] = entry.value;
                seen[index] = true;
            }
            for &stat in &stats {
                if !seen[stat] {
                    return Ok(Err(format!(
                        "convoy variant {} named bonus residuals omit weapon stat {stat}",
                        definition.id
                    )));
                }
            }
            for &stat in &stats {
                range = [range[0].min(cached[stat]), range[1].max(cached[stat])];
            }
            continue;
        }
        if let Some((_, why)) = kind.unread_terms.iter().find(|(index, _)| feeding.contains(index)) {
            return Ok(Err(format!("convoy variant {}: {why}", definition.id)));
        }
        if upgrades_by_variant.is_none() {
            let upgrades = rows::<tables::VariantUpgrades>(frame)?;
            if !upgrades.is_complete() {
                return Ok(Err(
                    "VariantUpgrades is incomplete, so convoy variant upgrades are unknown"
                        .to_owned(),
                ));
            }
            let mut map: BTreeMap<u64, Vec<&replay_data::rows::VariantUpgradeRow>> =
                BTreeMap::new();
            for row in upgrades.rows() {
                map.entry(row.variant).or_default().push(row);
            }
            upgrades_by_variant = Some(map);
        }
        if modules_by_variant.is_none() {
            let modules = rows::<tables::VariantModules>(frame)?;
            if !modules.is_complete() {
                return Ok(Err(
                    "VariantModules is incomplete, so convoy variant modules are unknown"
                        .to_owned(),
                ));
            }
            let mut map: BTreeMap<u64, Vec<&replay_data::rows::VariantModuleRow>> =
                BTreeMap::new();
            for row in modules.rows() {
                map.entry(row.variant).or_default().push(row);
            }
            modules_by_variant = Some(map);
        }
        let mut components = StatComponents::base(&kind.stats);
        if !state.bonus_sources.is_empty() || !state.named_sources.is_empty() {
            if bonus_snapshots.is_none() {
                let table = rows::<tables::VariantBonusSnapshots>(frame)?;
                if !table.is_complete() {
                    return Ok(Err(
                        "VariantBonusSnapshots is incomplete, so applied convoy variant bonuses are unknown"
                            .to_owned(),
                    ));
                }
                bonus_snapshots = Some(
                    table
                        .rows()
                        .iter()
                        .map(|row| (row.id, &row.source))
                        .collect(),
                );
            }
            let snapshots = bonus_snapshots.as_ref().unwrap();
            for id in state.bonus_sources.iter().chain(&state.named_sources) {
                let Some(source) = snapshots.get(id) else {
                    return Ok(Err(format!(
                        "convoy variant {} references unrecorded bonus snapshot {id}",
                        definition.id
                    )));
                };
                for entry in &source.stats {
                    if let Err(error) =
                        components.applied(false, entry.index as usize, entry.value)
                    {
                        return Ok(Err(format!(
                            "convoy variant {} bonus snapshot {id}: {error}",
                            definition.id
                        )));
                    }
                }
            }
        }
        for entry in &state.design_team_bonus {
            let Some(&name) = tokens.get(&entry.index) else {
                return Ok(Err(format!(
                    "convoy variant {} design-team bonus token {} has no recorded spelling",
                    definition.id, entry.index
                )));
            };
            let Some(index) = stat_index(&c_string(name).to_ascii_lowercase()) else {
                return Ok(Err(format!(
                    "convoy variant {} design-team bonus {} is not an equipment stat",
                    definition.id,
                    String::from_utf8_lossy(name)
                )));
            };
            if let Err(error) =
                components.applied([0x25, 0x26, 0x27].contains(&index), index, entry.value)
            {
                return Ok(Err(format!(
                    "convoy variant {} design-team bonus: {error}",
                    definition.id
                )));
            }
        }
        if let Some(variant_upgrades) = upgrades_by_variant
            .as_ref()
            .unwrap()
            .get(&definition.id)
        {
            for upgrade in variant_upgrades {
                let Ok(token) = u32::try_from(upgrade.definition_token) else {
                    return Ok(Err(format!(
                        "convoy variant {} has negative upgrade token {}",
                        definition.id, upgrade.definition_token
                    )));
                };
                let Some(&name) = tokens.get(&token) else {
                    return Ok(Err(format!(
                        "convoy variant {} upgrade token {token} has no recorded spelling",
                        definition.id
                    )));
                };
                let Some(group) = database.upgrade_stats(name) else {
                    return Ok(Err(format!(
                        "convoy variant {} upgrade {} has no content stat group",
                        definition.id,
                        String::from_utf8_lossy(name)
                    )));
                };
                if let Some(why) = unread_weapon_term(group) {
                    return Ok(Err(format!(
                        "convoy variant {} upgrade {}: {why}",
                        definition.id,
                        String::from_utf8_lossy(name)
                    )));
                }
                if let Err(error) = components.apply(group, i64::from(upgrade.level) * ONE) {
                    return Ok(Err(format!(
                        "convoy variant {} upgrade {}: {error}",
                        definition.id,
                        String::from_utf8_lossy(name)
                    )));
                }
            }
        }
        if let Some(variant_modules) = modules_by_variant
            .as_ref()
            .unwrap()
            .get(&definition.id)
        {
            for module in variant_modules {
                if module.definition_active == 0 {
                    continue;
                }
                let Some(&name) = tokens.get(&module.definition_token) else {
                    return Ok(Err(format!(
                        "convoy variant {} module token {} has no recorded spelling",
                        definition.id, module.definition_token
                    )));
                };
                let Some(group) = database.module_stats(name) else {
                    return Ok(Err(format!(
                        "convoy variant {} module {} has no content stat group",
                        definition.id,
                        String::from_utf8_lossy(name)
                    )));
                };
                if let Some(why) = unread_weapon_term(group).or_else(|| kind.unread_multipliers_for(group).find(|(index, _)| feeding.contains(index)).map(|(_, why)| why.to_owned())) {
                    return Ok(Err(format!(
                        "convoy variant {} module {}: {why}",
                        definition.id,
                        String::from_utf8_lossy(name)
                    )));
                }
                if let Err(error) = components.apply_module(group, kind) {
                    return Ok(Err(format!(
                        "convoy variant {} module {}: {error}",
                        definition.id,
                        String::from_utf8_lossy(name)
                    )));
                }
            }
        }
        let values = match components.finish_for(kind) {
            Ok(values) => values,
            Err(error) => {
                return Ok(Err(format!(
                    "convoy variant {} stat finish failed: {error}",
                    definition.id
                )));
            }
        };
        for &stat in &stats {
            range = [range[0].min(values[stat]), range[1].max(values[stat])];
        }
    }
    Ok(Ok(range))
}

fn bound_inputs(stats: &[usize], equipment: &content_reader_key_names::equipment::Equipment) -> Vec<usize> {
    let mut inputs = stats.to_vec();
    if equipment.scaled_by_reliability.iter().any(|term| stats.contains(&term.index)) {
        inputs.push(content_reader_script::equipment_stats::RELIABILITY_STAT);
    }
    inputs
}

fn bounded_total(ships: i64, convoys: i64, bound: Result<[i64; 2], &str>) -> Option<i64> {
    if convoys == 0 {
        return Some(ships / ONE);
    }
    let [low, high] = bound.ok()?;
    let least = ships.checked_add(convoys.checked_mul(low)?)?;
    let most = ships.checked_add(convoys.checked_mul(high)?)?;
    (least / ONE == most / ONE).then_some(least / ONE)
}

pub(super) fn weather_suffix(weather: &WeatherRegionRow) -> &'static str {
    if weather.heavy_rain != 0 {
        "storm"
    } else if weather.light_rain != 0 {
        "rain"
    } else if weather.blizzard != 0 {
        "snow_storm"
    } else if weather.snow != 0 {
        "snow"
    } else {
        ""
    }
}
pub(super) fn picture_names(terrain: Option<&str>, night: bool, weather: &str, ongoing: bool) -> Vec<String> {
    let time = if night { "night" } else { "day" };
    let prefix = if ongoing {
        "GFX_terrain_ongoing_sea_battle_"
    } else {
        "GFX_terrain_"
    };
    let mut names = Vec::new();
    for terrain in terrain.into_iter().chain(std::iter::once("water")) {
        let stem = format!("{prefix}{terrain}");
        if !weather.is_empty() {
            names.push(format!("{stem}_{time}_{weather}"));
        }
        names.push(format!("{stem}_{time}"));
    }
    names
}

#[expect(clippy::too_many_arguments, reason = "the weather, the weapon context, the terrain and its definitions, the text resources and the dropped list")]
fn terrain_hover(
    weather: &WeatherRegionRow,
    context: &NavalShipWeaponContext,
    terrain: &super::naval_terrain::Content,
    definitions: &Definitions<'_>,
    icons: &dyn TextIcons,
    loc: &Localisation,
    viewing_tag: Result<&str, &str>,
    dropped: &mut Vec<String>,
) -> Result<String, String> {
    let mut output = terrain.prefix(
        &context.terrain_name,
        definitions,
        icons,
        loc,
        viewing_tag,
        dropped,
    )?;
    let block = |scope| -> Result<Block, String> {
        let source = context
            .sources
            .iter()
            .find(|source| source.scope == scope)
            .ok_or_else(|| format!("recorded naval ship modifier scope {scope} is absent"))?;
        Ok(Block {
            values: source
                .terms
                .iter()
                .map(|term| (term.key, term.raw))
                .collect(),
            ..Block::default()
        })
    };
    let region = block(6)?
        .describe(definitions, icons, loc, ONE, "  - ", 1)
        .map_err(|reason| reason.to_string())?;
    let province = block(7)?
        .describe(definitions, icons, loc, ONE, "  - ", 1)
        .map_err(|reason| reason.to_string())?;
    let active: Result<Vec<_>, _> = active_weather_events(weather)
        .map(|(_, name, _)| token(icons, loc, name.as_bytes()))
        .collect();
    let active = active?;
    let weather_name = if active.is_empty() {
        token(icons, loc, WEATHER_NONE.as_bytes())?
    } else {
        active.join(", ")
    };
    ensure_newlines(&mut output, 2);
    let heading = builtin(
        icons,
        loc,
        WEATHER_MODIFIERS_HEADING,
        &[("WEA", Argument::Text(&weather_name))],
    )?;
    output.push_str(&format!("\u{11}H{heading}\u{11}!\n"));
    if !region.is_empty() {
        output.push_str("  - ");
        output.push_str(region.trim_start_matches("  - "));
    }
    if !province.is_empty() {
        ensure_newlines(&mut output, 2);
        output.push('\n');
        let heading = builtin(icons, loc, NIGHT_MODIFIERS, &[])?;
        output.push_str(&format!("\u{11}H{heading}\u{11}!\n  - "));
        output.push_str(province.trim_start_matches("  - "));
    }
    Ok(output)
}

fn joined_weather_context<'a>(
    ships: &[&'a NavalCombatShipRow],
    task_provinces: &BTreeMap<u64, u32>,
    battle_province: u32,
) -> Result<&'a NavalShipWeaponContext, String> {
    let joined: Vec<_> = ships
        .iter()
        .filter(|row| {
            row.live_ship != 0
                && task_provinces.get(&row.weapon_context.task_force) == Some(&battle_province)
        })
        .collect();
    let first = joined.first().ok_or_else(|| {
        format!("no live ship's task force joins battle province {battle_province}")
    })?;
    if joined.iter().any(|row| {
        let sources = |scope| {
            row.weapon_context
                .sources
                .iter()
                .find(|source| source.scope == scope)
        };
        let initial = |scope| {
            first
                .weapon_context
                .sources
                .iter()
                .find(|source| source.scope == scope)
        };
        sources(6) != initial(6)
            || sources(7) != initial(7)
            || row.weapon_context.night != first.weapon_context.night
    }) {
        return Err("ship-at-battle-province weather and night inputs disagree".to_owned());
    }
    Ok(&first.weapon_context)
}

fn opposing_silhouette(
    ships: &[&NavalCombatShipRow],
    models: Keyed<'_, ShipModelRow>,
    side: u8,
    defines: &content_reader_naval_combat_defines::naval_combat_defines::Defines,
) -> Result<i32, String> {
    let groups: BTreeSet<_> = ships
        .iter()
        .filter(|row| row.side == side && row.member_source == 0 && row.group_index != u32::MAX)
        .map(|row| row.group_index)
        .collect();
    groups.into_iter().try_fold(0_i32, |sum, group| {
        silhouette_group(ships, models, side, group, defines).map(|value| sum.wrapping_add(value))
    })
}

fn silhouette_group(
    ships: &[&NavalCombatShipRow],
    models: Keyed<'_, ShipModelRow>,
    side: u8,
    group: u32,
    defines: &content_reader_naval_combat_defines::naval_combat_defines::Defines,
) -> Result<i32, String> {
    let [capital, screen, carrier, support, convoy, submarine] = defines.silhouette_planes;
    let mut capitals = 0_i32;
    let mut screens = 0_i32;
    let mut others = 0_i32;
    for row in ships.iter().copied().filter(|row| {
        row.side == side
            && row.member_source == 0
            && row.group_index == group
            && row.detection_state != 0
            && row.state & 0xb != 0
    }) {
        if row.live_ship != 0 {
            let flags = row.formation_flags;
            if flags & (1 << 6) != 0 {
                capitals = capitals.wrapping_add(capital);
            } else if flags & (1 << 8) != 0 {
                screens = screens.wrapping_add(screen);
            } else if flags & (1 << 7) != 0 {
                others = others.wrapping_add(submarine);
            } else {
                let pool = if flags & ((1_u64 << 22) | (1_u64 << 37)) != 0 {
                    0
                } else {
                    let model = models.get(row.ship).ok_or_else(|| {
                        format!("live combat ship {} has no hourly ShipModels row", row.ship)
                    })?;
                    model
                        .counter
                        .as_ref()
                        .ok_or_else(|| {
                            format!(
                                "live combat ship {} has no positive equipment-pool flags",
                                row.ship
                            )
                        })?
                        .positive_equipment_flags
                };
                let carrier_flags = (1_u64 << 22) | (1_u64 << 37);
                if (flags | pool) & carrier_flags != 0 {
                    others = others.wrapping_add(carrier);
                } else if flags & (1 << 39) != 0 {
                    others = others.wrapping_add(support);
                }
            }
        }
        if row.live_convoy != 0 {
            others = others.wrapping_add(convoy);
        }
    }
    let denominator = i64::from(capitals)
        .wrapping_mul(defines.screen_cap_reduction_factor)
        .wrapping_add(ONE);
    let reciprocal = if denominator == 0 {
        0xffff_ffff
    } else {
        10_000_000_000_i64 / denominator
    };
    let weighted_screens = i64::from(screens).wrapping_mul(reciprocal);
    let rounded_screens =
        weighted_screens / ONE + i64::from(weighted_screens > 0 && weighted_screens % ONE != 0);
    Ok((rounded_screens as i32)
        .wrapping_add(capitals)
        .wrapping_add(others))
}

fn carrier_penalty_values(
    side: &NavalCombatSideDetails,
    opposing: &[&NavalCombatShipRow],
    models: Keyed<'_, ShipModelRow>,
    opposing_side: u8,
    defines: &content_reader_naval_combat_defines::naval_combat_defines::Defines,
) -> Result<(i64, i32), String> {
    let opposing_count = opposing_silhouette(opposing, models, opposing_side, defines)?;
    let threshold = defines.summary_integer[2].max(opposing_count);
    let planes = side
        .planes
        .iter()
        .filter(|plane| plane.source == 0 && plane.carrier_state & 0xb != 0)
        .fold(0_i32, |sum, plane| sum.wrapping_add(plane.current_planes));
    if planes <= threshold {
        return Ok((ONE, opposing_count));
    }
    let denominator = i64::from(planes.wrapping_sub(threshold))
        .wrapping_mul(defines.plane_stacking_penalty_effect)
        .wrapping_add(ONE);
    let multiplier = if denominator == 0 {
        0xffff_ffff
    } else {
        10_000_000_000_i64 / denominator
    };
    Ok((multiplier, opposing_count))
}

fn bind_carrier_penalty(out: &mut Bindings, side: usize, result: Result<(i64, i32, i32), String>) {
    let name = ["attacker", "defender"][side];
    let widget = out
        .values
        .entry(format!("header_container/carrier_penalty_{name}"))
        .or_default();
    widget.hidden = result.is_err();
    widget.opacity = result.as_ref().ok().map(
        |(multiplier, _, _)| {
            if *multiplier < ONE { 1.0 } else { 0.5 }
        },
    );
    if let Err(reason) = &result {
        out.refuse(format!(
            "Naval {name} carrier-penalty laurel opacity is unavailable: {reason}."
        ));
    }
    out.naval_penalties[side] = Some(result);
}

fn pride_bonus_applies(
    row: &NavalCombatShipRow,
    sides: &[NavalCombatSideDetails; 2],
    pride_countries: Keyed<'_, CountryCounterInputRow>,
    live_ships: Keyed<'_, ShipRow>,
    task_forces: Keyed<'_, TaskForceRow>,
    aliases: &BTreeMap<u32, u32>,
) -> Result<bool, String> {
    let task_force = task_forces
        .get(row.weapon_context.task_force)
        .ok_or_else(|| {
            format!(
                "combat ship {} has no TaskForces row for {}",
                row.ship, row.weapon_context.task_force
            )
        })?;
    let roster_owner = task_force.roster_owner;
    let country = pride_countries
        .get(u64::from(roster_owner))
        .ok_or_else(|| {
            format!("roster country {roster_owner} has no hourly CountryCounterInputs row")
        })?;
    if country.pride_of_the_fleet == 0 {
        return Ok(false);
    }
    let Some(pride) = live_ships.get(country.pride_of_the_fleet) else {
        return Ok(false);
    };
    if pride.task_force == 0
        || !sides
            .iter()
            .any(|side| side.units.contains(&pride.task_force))
    {
        return Ok(false);
    }
    let logical_owner = task_force.logical_owner;
    let owner_slot = aliases.get(&logical_owner).copied().ok_or_else(|| {
        format!("logical country {logical_owner} has no CountryLinks canonical slot")
    })?;
    let side =
        super::tactics::viewing_side(&sides[0].summary.countries, "attacker", aliases, owner_slot)?;
    Ok(sides[side]
        .summary
        .countries
        .iter()
        .any(|country| aliases.get(country) == Some(&owner_slot)))
}

pub(super) fn bind(
    out: &mut Bindings,
    static_content: &StaticContent,
    frame: &DecodedFrame,
    id: u64,
    flags: &CombatFlags,
    vision: &Vision,
    paused: bool,
) -> io::Result<()> {
    let details = rows::<tables::NavalCombatDetails>(frame)?;
    let Some(detail) = details.rows().iter().find(|r| r.combat == id) else {
        return Ok(());
    };
    let weapons_admitted = match replay_derive::fixed_layout::validate_recording(
        frame,
        replay_derive::fixed_layout::NAVAL_WEAPONS,
    ) {
        Ok(()) => true,
        Err(reason) => {
            out.warnings.push(reason);
            false
        }
    };
    let vectors = rows::<tables::NavalProvinceCombats>(frame)?;
    let vector = vectors
        .rows()
        .iter()
        .find(|row| row.province == detail.province);
    let ongoing = flags.engaged_entries(vector, vectors.is_complete(), 1);
    let undecided = match vector {
        None => "the battle province's combat vector is unknown (the NavalProvinceCombats table is incomplete)".to_owned(),
        Some(row) => row
            .combats
            .iter()
            .zip(&row.kinds)
            .find_map(|(&combat, &kind)| match flags.entry(combat, kind) {
                Engaged::Ambiguous(cause) => Some(cause.reason().to_owned()),
                _ => None,
            })
            .unwrap_or_else(|| "an engaged flag in the battle province's combat vector is unknown".to_owned()),
    };
    let ships = rows::<tables::NavalCombatShips>(frame)?;
    let ships_at: Vec<_> = ships.rows().iter().filter(|r| r.combat == id).collect();
    let ship_models = rows::<tables::ShipModels>(frame)?;
    let models = ship_models
        .is_complete()
        .then(|| Keyed::new(ship_models.rows(), |model| model.ship))
        .flatten();
    for index in 0..2 {
        let penalty = static_content
            .assets
            .naval_defines
            .as_ref()
            .ok_or_else(|| "the matching naval combat defines are unavailable".to_owned())
            .and_then(|defines| {
                let models = models.ok_or_else(|| {
                    "the hourly ShipModels census is incomplete or unordered".to_owned()
                })?;
                if !ships.is_complete() {
                    return Err("the NavalCombatShips census is incomplete".to_owned());
                }
                carrier_penalty_values(
                    &detail.sides[index],
                    &ships_at,
                    models,
                    1 - index as u8,
                    defines,
                )
                .map(|(multiplier, opposing)| (multiplier, opposing, defines.summary_integer[2]))
            });
        bind_carrier_penalty(out, index, penalty);
    }
    let tasks: BTreeMap<_, _> = rows::<tables::TaskForces>(frame)?
        .rows()
        .iter()
        .map(|r| (r.id, r.province))
        .collect();
    let region = static_content.region_of.get(&detail.province).copied();
    if region.is_none() {
        out.warnings.push(format!(
            "Naval environment: province {} is in no content strategic region, so the terrain picture and weather widgets remain empty.",
            detail.province
        ));
    }
    if let Some(region) = region {
        if let Some(weather) = rows::<tables::WeatherRegions>(frame)?
            .rows()
            .iter()
            .find(|r| r.id == region)
        {
            let nights: BTreeSet<_> = ships_at
                .iter()
                .filter(|r| {
                    r.live_ship != 0
                        && tasks.get(&r.weapon_context.task_force) == Some(&detail.province)
                })
                .map(|r| r.weapon_context.night > 50_000)
                .collect();
            if nights.len() == 1 {
                let terrain = static_content
                    .strategic_regions
                    .get(&region)
                    .and_then(|r| r.naval_terrain.as_deref());
                let night = *nights.first().expect("one night value");
                let night = if night {
                    let clock = rows::<tables::Clock>(frame)?;
                    match clock.rows() {
                        [row] if clock.is_complete() => Some(night_picture(row.speed, paused)),
                        _ => {
                            out.refuse("Naval terrain picture is unavailable: the complete single-row Clock speed operand is absent.".into());
                            None
                        }
                    }
                } else {
                    Some(false)
                };
                if let Some(night) = night {
                    let picture = |ongoing| {
                        let names = picture_names(terrain, night, weather_suffix(weather), ongoing);
                        names
                            .iter()
                            .find(|n| static_content.assets.has_sprite(n))
                            .cloned()
                            .ok_or(names)
                    };
                    let name = match ongoing {
                        Some(ongoing) => Some(picture(ongoing)),
                        None => match (picture(true), picture(false)) {
                            (Ok(with), Ok(without)) if with == without => Some(Ok(with)),
                            (Err(mut with), Err(without)) => {
                                with.extend(without);
                                Some(Err(with))
                            }
                            _ => {
                                out.warnings.push(format!("Naval environment: whether a combat in the battle province is engaged for this country is undecided (for the ongoing_sea_battle_ prefix): {undecided}; terrain image remains empty."));
                                None
                            }
                        },
                    };
                    match name {
                    Some(Ok(name)) => {
                        out.values.insert(
                            "header_container/terrain_background".into(),
                            Widget {
                                sprite: Some(name),
                                ..Widget::default()
                            },
                        );
                    }
                    Some(Err(names)) => out.warnings.push(format!(
                        "Naval environment: the content declares none of the terrain pictures {}; the picture remains empty.",
                        names.join(", ")
                    )),
                    None => {}
                }
                }
            } else {
                out.warnings.push("Naval environment: no consistent recorded province-night operand joins a live task force at the battle province; terrain image remains empty.".into());
            }
            let custom_sea = match (detail.custom_sea.as_deref(), detail.custom_sea_error) {
                (Some(bytes), 0) => Ok(custom_sea_words(bytes)),
                _ => Err("guarded native read failed".to_owned()),
            };
            let entries = weather_entries(out, static_content, frame, weather, region, custom_sea.as_deref().map_err(Clone::clone))?;
            out.values.insert(
                "header_container/modifier_grid".into(),
                Widget {
                    entry_template: Some(MODIFIER_ENTRY.into()),
                    entries,
                    ..Widget::default()
                },
            );
        } else {
            out.warnings.push("Naval environment: WeatherRegions has no row for this battle; weather widgets remain empty.".into());
        }
    }
    if ships.is_complete() {
        let country_inputs = rows::<tables::CountryCounterInputs>(frame)?;
        let live_ships = rows::<tables::Ships>(frame)?;
        let task_force_rows = rows::<tables::TaskForces>(frame)?;
        let country_links = rows::<tables::CountryLinks>(frame)?;
        let pride_countries = country_inputs
            .is_complete()
            .then(|| Keyed::new(country_inputs.rows(), |row| u64::from(row.country)))
            .flatten();
        let pride_ships = live_ships
            .is_complete()
            .then(|| Keyed::new(live_ships.rows(), |row| row.id))
            .flatten();
        let pride_tasks = task_force_rows
            .is_complete()
            .then(|| Keyed::new(task_force_rows.rows(), |row| row.id))
            .flatten();
        let aliases = country_links.is_complete().then(|| {
            country_links
                .rows()
                .iter()
                .map(|row| (row.id, row.canonical_slot))
                .collect::<BTreeMap<_, _>>()
        });
        let mut pride_bonus = BTreeMap::new();
        for row in ships_at.iter().copied().filter(|row| row.live_ship != 0) {
            let value = match (pride_countries, pride_ships, pride_tasks, aliases.as_ref()) {
                (Some(countries), Some(ships), Some(tasks), Some(aliases)) => {
                    pride_bonus_applies(row, &detail.sides, countries, ships, tasks, aliases)
                }
                _ => Err(
                    "CountryCounterInputs, Ships, TaskForces or CountryLinks is incomplete or unordered"
                        .to_owned(),
                ),
            };
            if let Err(reason) = &value {
                out.refuse(format!(
                    "Naval ship {} pride modifier is unavailable: {reason}.",
                    row.ship
                ));
            }
            pride_bonus.insert((row.side, row.battle_slot), value.ok());
        }
        let mut frame_convoy_bound: Option<Result<[i64; 2], String>> = None;
        for side in 0..2_u8 {
            let members: Vec<_> = ships_at
                .iter()
                .copied()
                .filter(|r| {
                    r.side == side
                        && r.member_source == 0
                        && r.group_index != u32::MAX
                        && matches!(r.state, 2 | 8)
                })
                .collect();
            let convoys = members
                .iter()
                .filter(|r| r.live_ship == 0 && r.live_convoy != 0)
                .count() as i64;
            if weapons_admitted && convoys > 0 && frame_convoy_bound.is_none() {
                let bound = convoy_weapon_bound_for_frame(
                    static_content
                        .assets
                        .convoy_weapon_bound
                        .as_ref()
                        .copied()
                        .map_err(String::as_str),
                    static_content.assets.naval_air_groups.equipment(),
                    frame,
                )?;
                if let Err(reason) = &bound
                    && static_content.assets.convoy_weapon_bound.is_ok()
                {
                    out.warnings.push(format!(
                        "Naval weapon totals with live convoys stay dash: {reason}."
                    ));
                }
                frame_convoy_bound = Some(bound);
            }
            for (widget, case) in [
                ("stat_light_gun", 0),
                ("stat_heavy_gun", 1),
                ("stat_torpedo", 3),
                ("stat_depth_charges", 2),
            ] {
                if !weapons_admitted {
                    continue;
                }
                let bound = match frame_convoy_bound.as_ref() {
                    Some(bound) => bound.as_ref().copied().map_err(String::as_str),
                    None => Ok([0, 0]),
                };
                let sum = members
                    .iter()
                    .filter(|r| r.live_ship != 0)
                    .try_fold(0, |sum, r| {
                        let pride = pride_bonus
                            .get(&(r.side, r.battle_slot))
                            .copied()
                            .flatten()?;
                        Some(add(sum, weapon(r, case, pride)?))
                    })
                    .and_then(|ships| bounded_total(ships, convoys, bound));
                let wnd = if side == 0 { "left" } else { "right" };
                if let Some(sum) = sum {
                    text(
                        &mut out.values,
                        &format!("header_container/{wnd}_wnd/{widget}"),
                        sum.to_string(),
                    );
                } else {
                    out.warnings.push(format!("Naval {wnd} {widget}: an unsupported modifier operand is missing, or live convoys whose unrecorded equipment values could change the displayed whole number; total remains dash."));
                }
            }
        }
    }
    let mut terrain_dropped = Vec::new();
    let hover = (|| {
        let region = region.ok_or_else(|| {
            format!(
                "battle province {} has no strategic region",
                detail.province
            )
        })?;
        let weather = rows::<tables::WeatherRegions>(frame)
            .map_err(|error| error.to_string())?
            .rows()
            .iter()
            .find(|row| row.id == region)
            .ok_or_else(|| format!("strategic region {region} has no WeatherRegions row"))?;
        let context = joined_weather_context(&ships_at, &tasks, detail.province)?;
        let countries = rows::<tables::Countries>(frame).map_err(|error| error.to_string())?;
        let viewing_tag = tooltip_text_viewing_tag(countries.rows(), vision);
        let definitions = Definitions::read(frame, &static_content.modifier_names)
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        terrain_hover(
            weather,
            context,
            static_content
                .assets
                .naval_terrain
                .as_ref()
                .map_err(Clone::clone)?,
            &definitions,
            static_content.assets.as_ref(),
            &static_content.localisation,
            viewing_tag,
            &mut terrain_dropped,
        )
    })();
    for reason in terrain_dropped {
        out.refuse(reason);
    }
    match hover {
        Ok(text) => set_hover(
            &mut out.values,
            "header_container/terrain_background",
            hover_text(&text),
            None,
        ),
        Err(reason) => {
            set_hover(
                &mut out.values,
                "header_container/terrain_background",
                None,
                None,
            );
            out.refuse(format!(
                "Naval terrain-background hover is withheld: {reason}."
            ));
        }
    }
    Ok(())
}
pub(super) fn night_picture(recorded_speed: i32, viewer_paused: bool) -> bool {
    recorded_speed <= 3 || viewer_paused
}
fn weather_icon_text(text: &str) -> Result<Option<(&str, u32)>, String> {
    if text.is_empty() {
        return Ok(None);
    }
    let mut parts = text.split(':');
    let sprite = parts.next().unwrap_or_default();
    let frame = parts.next().map_or(1, |text| {
        content_reader_parser::fixed5::decimal_i32(text.as_bytes())
    });
    let frame = u32::try_from(frame)
        .ok()
        .filter(|frame| *frame >= 1)
        .ok_or_else(|| {
            format!("its frame {frame} is not positive; the native sprite frame setter's handling of that value is unsupported")
        })?;
    Ok(Some((sprite, frame - 1)))
}

fn weather_tooltip(
    name: &str,
    block: &Block,
    definitions: &Definitions<'_>,
    icons: &dyn TextIcons,
    localisation: &content_reader_localisation::localisation::Localisation,
) -> Result<String, String> {
    let name = token(icons, localisation, name.as_bytes())?;
    let description = block
        .describe(definitions, icons, localisation, ONE, "", 1)
        .map_err(|reason| reason.to_string())?;
    Ok(format!("\u{11}H{name}\u{11}!\n\n{description}"))
}

pub(super) const WEATHER_MODIFIERS: [&str; 6] = [
    "weather_rain_light",
    "weather_rain_heavy",
    "weather_snow",
    "weather_blizzard",
    "weather_sandstorm",
    "weather_arctic_water",
];
tooltip_keys! {
    WEATHER_MODIFIERS_HEADING = "WEATHER_MODIFIERS",
    NIGHT_MODIFIERS = "NIGHT_MODIFIERS",
    WEATHER_NONE = "weather_none",
}
const WEATHER_EVENTS: [(&str, &str); 6] = [
    (WEATHER_MODIFIERS[0], "rain_light_icon"),
    (WEATHER_MODIFIERS[1], "rain_heavy_icon"),
    (WEATHER_MODIFIERS[2], "snow_icon"),
    (WEATHER_MODIFIERS[3], "blizzard_icon"),
    (WEATHER_MODIFIERS[4], "sandstorm_icon"),
    (WEATHER_MODIFIERS[5], "arctic_water_icon"),
];

fn active_weather_events(
    weather: &WeatherRegionRow,
) -> impl Iterator<Item = (usize, &'static str, &'static str)> {
    [
        weather.light_rain,
        weather.heavy_rain,
        weather.snow,
        weather.blizzard,
        weather.sandstorm,
        weather.arctic_water,
    ]
    .into_iter()
    .zip(WEATHER_EVENTS)
    .enumerate()
    .filter_map(|(slot, (active, (name, key)))| (active != 0).then_some((slot, name, key)))
}

pub(super) fn active_custom_sea<'a>(
    names: &'a [String],
    values: &'a [i16],
) -> Option<impl Iterator<Item = &'a str>> {
    (values.len() == names.len()).then(|| {
        names
            .iter()
            .zip(values)
            .filter_map(|(name, &value)| (value != 0).then_some(name.as_str()))
    })
}

pub(super) fn custom_sea_words(bytes: &[u8]) -> Vec<i16> {
    bytes.chunks_exact(2).map(|pair| i16::from_le_bytes([pair[0], pair[1]])).collect()
}

pub(super) fn region_custom_sea(
    region_of: &std::collections::BTreeMap<u32, u32>,
    frame: &DecodedFrame,
    weather: &WeatherRegionRow,
    region: u32,
) -> Result<Vec<i16>, String> {
    let details = frame.complete::<tables::NavalCombatDetails>().map_err(|e| e.to_string())?;
    if let Some(bytes) = details
        .iter()
        .filter(|row| region_of.get(&row.province) == Some(&region))
        .find_map(|row| row.custom_sea.as_deref())
    {
        return Ok(custom_sea_words(bytes));
    }
    match &weather.custom_sea {
        Recorded::Value(Some(values)) => Ok(values.clone()),
        Recorded::Value(None) => Err("the region's custom sea values were refused when read".into()),
        Recorded::NotRecorded => Err("the region's custom sea values are not recorded".into()),
    }
}

pub(super) const MODIFIER_ENTRY: &str = "naval_region_modifier_entry";

pub(super) fn weather_entries(
    out: &mut Bindings,
    static_content: &StaticContent,
    frame: &DecodedFrame,
    weather: &WeatherRegionRow,
    region: u32,
    custom_sea: Result<&[i16], String>,
) -> io::Result<Vec<Values>> {
    let mut entries = Vec::new();
    let definitions = Definitions::read(frame, &static_content.modifier_names)?;
    if let Some(template) = static_content.strategic_regions.get(&region) {
        let names = &template.static_modifier_names;
        match &custom_sea {
            Err(reason) if !names.is_empty() => {
                out.refuse(format!("Naval strategic region {region} custom sea grid is unavailable: {reason}."));
            }
            Ok(values) if !names.is_empty() && active_custom_sea(names, values).is_none() => {
                out.refuse(format!("Naval strategic region {region} custom sea grid is withheld: captured values do not match {} content modifier names.", names.len()));
            }
            _ => {}
        }
        if let Some(custom_sea) = custom_sea.ok().filter(|values| active_custom_sea(names, values).is_some()) {
            for name in active_custom_sea(names, custom_sea).expect("length checked") {
                let result = (|| {
                    let modifier = static_content
                        .assets
                        .naval_custom_sea_modifiers
                        .get(name)
                        .ok_or_else(|| format!("static modifier {name} has no stacked block"))?
                        .as_ref()
                        .map_err(Clone::clone)?;
                    let definitions = definitions.as_ref().map_err(ToString::to_string)?;
                    let (block, dropped) =
                        Block::build(definitions, modifier).map_err(|reason| reason.to_string())?;
                    for key in dropped {
                        out.refuse(format!("Naval custom sea modifier {name}: {key} has no recorded definition and is dropped as the game drops it."));
                    }
                    if !block
                        .has_category_one(definitions)
                        .map_err(|reason| reason.to_string())?
                    {
                        return Ok(None);
                    }
                    let icon_key = format!("{name}_icon");
                    let Some((sprite, frame)) =
                        weather_icon(&static_content.localisation, &icon_key, |sprite| {
                            static_content.assets.has_sprite(sprite)
                        })?
                    else {
                        return Ok(None);
                    };
                    let mut values = Values::new();
                    values.insert(
                        "icon".into(),
                        Widget {
                            sprite: Some(sprite),
                            frame: Some(frame),
                            ..Widget::default()
                        },
                    );
                    let tooltip = weather_tooltip(
                        &format!("{name}_name"),
                        &block,
                        definitions,
                        static_content.assets.as_ref(),
                        &static_content.localisation,
                    )?;
                    set_hover(&mut values, "icon", hover_text(&tooltip), None);
                    Ok::<_, String>(Some(values))
                })();
                match result {
                    Ok(Some(values)) => entries.push(values),
                    Ok(None) => (),
                    Err(reason) => out.refuse(format!("Naval strategic region {region} custom sea modifier {name} is unavailable: {reason}.")),
                }
            }
        }
    } else {
        out.refuse(format!(
            "Naval strategic region {region} custom sea entries are unavailable: the recorded content stack has no strategic-region template."
        ));
    }
    for (slot, name, key) in active_weather_events(weather) {
        let modifier = static_content
            .assets
            .naval_weather_modifiers
            .get(slot)
            .ok_or_else(|| format!("the weather static modifier slot {slot} is missing"))
            .and_then(|modifier| modifier.as_ref().map_err(Clone::clone));
        let block = modifier.and_then(|modifier| {
            let definitions = definitions.as_ref().map_err(ToString::to_string)?;
            let (block, dropped) = Block::build(definitions, modifier)
                .map_err(|reason| reason.to_string())?;
            for key in dropped {
                out.refuse(format!("Naval weather event {name}: modifier {key} has no recorded definition and is dropped as the game drops it."));
            }
            Ok::<_, String>(block.has_category_one(definitions)
                .map_err(|reason| reason.to_string())?.then_some(block))
        });
        let block = match block {
            Ok(Some(block)) => block,
            Ok(None) => continue,
            Err(reason) => {
                out.refuse(format!(
                    "Naval weather event {name} is unavailable: {reason}."
                ));
                continue;
            }
        };
        match weather_icon(&static_content.localisation, key, |sprite| {
            static_content.assets.has_sprite(sprite)
        }) {
            Ok(None) => (),
            Ok(Some((sprite, frame))) => {
                let tooltip = weather_tooltip(
                    name,
                    &block,
                    definitions.as_ref().expect("block requires definitions"),
                    static_content.assets.as_ref(),
                    &static_content.localisation,
                );
                let mut values = Values::new();
                values.insert(
                    "icon".into(),
                    Widget {
                        sprite: Some(sprite),
                        frame: Some(frame),
                        ..Widget::default()
                    },
                );
                match tooltip {
                    Ok(tooltip) => set_hover(&mut values, "icon", hover_text(&tooltip), None),
                    Err(reason) => {
                        set_hover(&mut values, "icon", None, None);
                        out.refuse(format!(
                            "Naval weather event {name} tooltip is unavailable: {reason}."
                        ));
                    }
                }
                entries.push(values);
            }
            Err(reason) => out.refuse(format!("Naval weather icon {key} is hidden: {reason}.")),
        }
    }
    Ok(entries)
}
fn weather_icon(
    localisation: &content_reader_localisation::localisation::Localisation,
    key: &str,
    has_sprite: impl Fn(&str) -> bool,
) -> Result<Option<(String, u32)>, String> {
    let token = localisation
        .localise(key.as_bytes())
        .map_err(|refusal| refusal.to_string())?;
    let Some((sprite, frame)) = weather_icon_text(&token)? else {
        return Ok(None);
    };
    if !has_sprite(sprite) {
        return Err(format!("its sprite {sprite} is missing from the content"));
    }
    Ok(Some((sprite.to_owned(), frame)))
}

#[cfg(test)]
mod tests;
