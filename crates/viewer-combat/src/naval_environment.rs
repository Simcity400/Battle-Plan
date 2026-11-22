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
use viewer_foundation::engaged::Engaged;
use viewer_foundation::engagement_history::CombatFlags;
use viewer_foundation::map::perspective::Vision;
use viewer_foundation::tooltip_text::ensure_newlines;
use viewer_foundation::tooltip_text::tooltip_keys;
use viewer_foundation::tooltip_text::viewing_tag as tooltip_text_viewing_tag;
use {super::content::Content as StaticContent, viewer_foundation::map::tables::rows};
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
fn convoy_weapon_bound(layers: &content_reader_core::ResolvedLayers) -> io::Result<ConvoyBound> {
    Ok(convoy_weapon_bound_from_database(
        &content_reader_core::equipment::Database::load(layers)?,
    ))
}

pub(super) fn convoy_weapon_bound_from_database(
    database: &content_reader_core::equipment::Database,
) -> ConvoyBound {
    use content_reader_core::equipment::{StatComponents, StatGroup};
    let misreads = database.misreads.clone();
    if let Some(lost) = &database.lost {
        return (Err(lost.clone()), misreads);
    }
    let stats: Vec<usize> = CONVOY_WEAPON_STATS
        .iter()
        .filter_map(|name| content_reader_core::equipment::stat_index(name.as_bytes()))
        .collect();
    let group_touches_weapons = |group: &StatGroup| {
        group
            .add
            .iter()
            .chain(&group.average)
            .chain(&group.multiply)
            .any(|term| stats.contains(&term.index))
    };
    let mut range = [0_i64; 2];
    for equipment in database
        .equipment
        .iter()
        .filter(|equipment| database.category(equipment) & 1 != 0)
    {
        let has_dynamic_terms = !equipment.module_add_stats_multiplier.is_empty()
            || !equipment.module_multiply_stats_multiplier.is_empty()
            || !equipment.scaled_by_reliability.is_empty();
        let default_weapon_modules: Vec<&StatGroup> = equipment
            .default_modules
            .iter()
            .filter_map(|(_, module)| database.module_stats(module))
            .filter(|group| group_touches_weapons(group))
            .collect();
        let slot_allows_weapon_module = !equipment.module_slots.is_empty()
            && database.all_module_designs().any(|(_, design, group)| {
                group.is_some_and(group_touches_weapons)
                    && equipment.module_slot_rules.iter().any(|slot| {
                        slot.categories
                            .iter()
                            .any(|category| category == &design.category)
                    })
            });
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
        if default_weapon_modules.len() > 8 {
            return (
                Err(format!(
                    "convoy equipment {} defines {} weapon-modifying default modules, so convoy weapon bounds cannot be enumerated from equipment types alone",
                    String::from_utf8_lossy(&equipment.name),
                    default_weapon_modules.len()
                )),
                misreads,
            );
        }
        for mask in 0..(1_usize << default_weapon_modules.len()) {
            let mut components = StatComponents::base(&equipment.stats);
            for (bit, group) in default_weapon_modules.iter().enumerate() {
                if mask & (1_usize << bit) != 0
                    && let Err(reason) = components.apply_module(group, equipment)
                {
                    return (Err(reason.to_owned()), misreads);
                }
            }
            let projected = match components.finish_for(equipment) {
                Ok(values) => values,
                Err(reason) => return (Err(reason.to_owned()), misreads),
            };
            let at_zero_rel = components.finish();
            let mut full_rel = components.clone();
            for term in &equipment.scaled_by_reliability {
                if let Some(target) = full_rel.values.get_mut(term.index) {
                    target.additive = target.additive.wrapping_add(term.value);
                }
            }
            let at_full_rel = full_rel.finish();
            for &stat in &stats {
                for value in [projected[stat], at_zero_rel[stat], at_full_rel[stat]] {
                    range = [range[0].min(value), range[1].max(value)];
                }
            }
        }
    }
    (Ok(range), misreads)
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

#[allow(clippy::too_many_arguments)]
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
            for (widget, case) in [
                ("stat_light_gun", 0),
                ("stat_heavy_gun", 1),
                ("stat_torpedo", 3),
                ("stat_depth_charges", 2),
            ] {
                if !weapons_admitted {
                    continue;
                }
                let convoys = members
                    .iter()
                    .filter(|r| r.live_ship == 0 && r.live_convoy != 0)
                    .count() as i64;
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
                    .and_then(|ships| {
                        bounded_total(
                            ships,
                            convoys,
                            static_content
                                .assets
                                .convoy_weapon_bound
                                .as_ref()
                                .copied()
                                .map_err(String::as_str),
                        )
                    });
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
        content_reader_core::fixed5::decimal_i32(text.as_bytes())
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
