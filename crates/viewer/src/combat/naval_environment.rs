//! Native naval environment selection and displayed gun/depth totals.
use super::{
    Values, Widget,
    binding::{Bindings, text},
};
use crate::{
    document::Document,
    map::{
        perspective::{NAVAL_COMBATS_NOT_WHOLE, NAVAL_DETAILS_ROW_MISSING, SharingUnknown, Vision},
        tables::rows,
    },
};
use engine::modifier_keys as keys;
use replay::{
    frame::DecodedFrame,
    rows::{
        NavalCombatShipRow, NavalCombatSideDetails, NavalShipModifierSource,
        NavalShipWeaponContext, WeatherRegionRow, tables,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

const ONE: i64 = 100_000;
fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}
fn add(a: i64, b: i64) -> i64 {
    a.wrapping_add(b)
}

// 140551820: missing entries return zero; bit4 clamps each individual
// source before composition. Feature eligibility is captured in scope26.
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

/// 140C29020, including doctrine weighting, weather adjustment, critical
/// count/severity and maximum (not sum) over independent HQ influences.
pub(super) fn modifier(context: &NavalShipWeaponContext, key: u32) -> Option<i64> {
    let mut sum = 0;
    let mut influences = BTreeMap::<u32, i64>::new();
    for source in &context.sources {
        let value = match source.scope {
            0..=3 | 5 | 7..=10 => term(context, source, key)?,
            4 => {
                // 140D5B220 -> 141956CB0/141956D40. weights are signed
                // achieved/possible counts, not fixed-point fractions.
                let [achieved, possible] = source.weights;
                let progress = add(ONE, source_term(context, 0, source.dynamic_keys[0])?)
                    .wrapping_mul(achieved);
                let weight = if possible == 0 {
                    0
                } else {
                    (progress / possible).clamp(0, ONE)
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
                    add(ONE, modifier(context, keys::NAVAL_CRITICAL_EFFECT_FACTOR)?)
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
            // Raw weapon adjusters, relation operands and default-leader
            // sources have distinct callers; they are not additive here.
            16 | 20..=27 | 31 | 32 => 0,
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
        // 141AC36A0/3700/3760/3780 read the convoy equipment definition.
        // The format22 convoy reader records its bars, not these weapon bases.
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
    // 140C25310 uses the cached formation flags at ship+628, without
    // substituting ship category or adding a strength multiplier.
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
        // Native instructions140C254B9/140C25527 add positive C4/C6.
        // The decompiler sign-extends these immediates incorrectly.
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

/// The largest light/heavy gun, torpedo or depth-charge attack any convoy
/// equipment declares, in fixed5. A live convoy's weapon getters
/// (141AC36A0/3700/3760/3780) read its equipment definition's fields
/// unmodified, and format 22 does not record which convoy equipment a slot
/// carries nor which field each getter reads, so a total is bounded by this
/// value per convoy. `None` when a convoy definition is not a plain literal.
/// Shipped content: `convoy` declares lg_attack 0.25 and torpedo_attack 0.
pub(super) fn convoy_weapon_bound(layers: &installed_content::ResolvedLayers) -> io::Result<Option<i64>> {
    use installed_content::parser::Item;
    let mut definitions: BTreeMap<String, Vec<Item>> = BTreeMap::new();
    let mut paths: Vec<_> = layers
        .paths()
        .filter(|p| p.starts_with("common/units/equipment/") && p.ends_with(".txt"))
        .collect();
    paths.sort_unstable();
    for path in paths {
        for item in installed_content::parser::parse(&layers.read(path)?)?.items {
            if item.key.as_deref() != Some(b"equipments") {
                continue;
            }
            for entry in item.value.items().unwrap_or_default() {
                if let (Some(key), Some(items)) = (entry.key.as_deref(), entry.value.items()) {
                    definitions.insert(String::from_utf8_lossy(key).into_owned(), items.to_vec());
                }
            }
        }
    }
    let scalar = |items: &[Item], key: &str| {
        items
            .iter()
            .rev()
            .find(|i| i.key.as_deref() == Some(key.as_bytes()))
            .and_then(|i| i.value.scalar_str())
            .map(str::to_owned)
    };
    // A value is the definition's own, else its archetype's or parent's.
    let lookup = |name: &str, key: &str| -> Option<String> {
        let mut current = name.to_owned();
        for _ in 0..8 {
            let items = definitions.get(&current)?;
            if let Some(value) = scalar(items, key) {
                return Some(value);
            }
            current = scalar(items, "archetype").or_else(|| scalar(items, "parent"))?;
        }
        None
    };
    let mut bound = 0_i64;
    for name in definitions.keys() {
        if lookup(name, "type").as_deref() != Some("convoy") {
            continue;
        }
        for key in ["lg_attack", "hg_attack", "torpedo_attack", "sub_attack"] {
            let Some(value) = lookup(name, key) else {
                continue;
            };
            let Ok(value) = value.parse::<f64>() else {
                return Ok(None);
            };
            if !(0.0..1e9).contains(&value) {
                return Ok(None);
            }
            bound = bound.max((value * ONE as f64).ceil() as i64);
        }
    }
    Ok(Some(bound))
}

/// The displayed whole number of `ships` plus `convoys` live convoys, when
/// every convoy value in `[0, bound]` truncates to the same integer.
fn bounded_total(ships: i64, convoys: i64, bound: Option<i64>) -> Option<i64> {
    let high = ships.checked_add(convoys.checked_mul(bound?)?)?;
    (ships / ONE == high / ONE).then_some(ships / ONE)
}

// 140B387D0 has different precedence from map weather-particle selection.
fn weather_suffix(weather: &WeatherRegionRow) -> &'static str {
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
fn picture_names(terrain: Option<&str>, night: bool, weather: &str, ongoing: bool) -> Vec<String> {
    let time = if night { "night" } else { "day" };
    let prefix = if ongoing {
        "GFX_terrain_ongoing_sea_battle_"
    } else {
        "GFX_terrain_"
    };
    let mut names = Vec::new();
    // 140B387D0 uses the raw terrain token (already water_deep_ocean,
    // for example). Its generic water fallback retains the battle prefix.
    for terrain in terrain.into_iter().chain(std::iter::once("water")) {
        let stem = format!("{prefix}{terrain}");
        if !weather.is_empty() {
            names.push(format!("{stem}_{time}_{weather}"));
        }
        names.push(format!("{stem}_{time}"));
    }
    names
}

fn no_carrier_penalty(side: &NavalCombatSideDetails) -> bool {
    // 14160C430 walks source0 plane groups and accepts carrier state&0xB.
    // The reader records state0 for a null carrier, preserving this test.
    let planes = side
        .planes
        .iter()
        .filter(|plane| plane.source == 0 && plane.carrier_state & 0xb != 0)
        .fold(0_i32, |sum, plane| sum.wrapping_add(plane.current_planes));
    // Native threshold is max(free allowance, opposing weighted ship count).
    // This branch needs no assumptions about the latter unported evaluator.
    planes <= side.summary.integer_defines[2]
}

/// 140E69830: whether the battle province shows the `ongoing_sea_battle_`
/// terrain prefix, over the hour's tables.
fn ongoing_battle(
    frame: &DecodedFrame,
    province: u32,
    aliases: &BTreeMap<u32, u32>,
    vision: &Vision,
) -> io::Result<Result<bool, SharingUnknown>> {
    let details = rows::<tables::NavalCombatDetails>(frame)?;
    // 140E69830: any intel-visible combat at this province enables the
    // ongoing_sea_battle_ prefix. 1413CD880 uses either side's countries.
    // A proven seen combat decides it; otherwise an undecided one leaves it
    // unknown, with the first missing input. The province's combats are
    // every NavalCombats identity there (and every detail row there): one
    // without a detail row has no country list, and a NavalCombats table
    // that is not whole may lack one.
    let combats = rows::<tables::NavalCombats>(frame)?;
    let at_province: BTreeSet<u64> = combats
        .rows()
        .iter()
        .filter(|r| r.province == province)
        .map(|r| r.id)
        .chain(
            details
                .rows()
                .iter()
                .filter(|r| r.province == province)
                .map(|r| r.combat),
        )
        .collect();
    let mut ongoing = Ok(false);
    for combat in at_province {
        let row = details.rows().iter().find(|r| r.combat == combat);
        match vision.sees_combat(
            row.into_iter()
                .flat_map(|r| &r.sides)
                .flat_map(|side| &side.summary.countries)
                .map(|country| aliases.get(country).copied()),
            row.is_none().then_some(NAVAL_DETAILS_ROW_MISSING),
        ) {
            Ok(true) => {
                ongoing = Ok(true);
                break;
            }
            Ok(false) => {}
            Err(cause) => {
                if ongoing == Ok(false) {
                    ongoing = Err(cause);
                }
            }
        }
    }
    if ongoing == Ok(false) && !combats.is_complete() {
        ongoing = Err(SharingUnknown::Census(NAVAL_COMBATS_NOT_WHOLE));
    }
    Ok(ongoing)
}

pub(super) fn bind(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    id: u64,
    vision: &Vision,
) -> io::Result<()> {
    let details = rows::<tables::NavalCombatDetails>(frame)?;
    let Some(detail) = details.rows().iter().find(|r| r.combat == id) else {
        return Ok(());
    };
    let weapons_admitted = match replay::fixed_layout::validate_recording(
        frame,
        replay::fixed_layout::NAVAL_WEAPONS,
    ) {
        Ok(()) => true,
        Err(reason) => {
            out.warnings.push(reason);
            false
        }
    };
    let aliases: BTreeMap<_, _> = rows::<tables::CountryLinks>(frame)?
        .rows()
        .iter()
        .map(|r| (r.id, r.canonical_slot))
        .collect();
    let ongoing = ongoing_battle(frame, detail.province, &aliases, vision)?;
    for (side, name) in detail.sides.iter().zip(["attacker", "defender"]) {
        if no_carrier_penalty(side) {
            let widget = out
                .values
                .entry(format!("header_container/carrier_penalty_{name}"))
                .or_default();
            widget.hidden = false;
            // 1417DA2F0→CIcon+308→1422E6A10→CSprite+8→1422E9810.
            // Sprite+18C multiplies only output alpha in14222D5A0.
            widget.opacity = Some(0.5);
        } else {
            out.warnings.push(format!("Naval {name} carrier penalty: UNVERIFIED opposing weighted ship-count evaluator141C557D0 is unported; laurel remains unavailable."));
        }
    }
    let ships = rows::<tables::NavalCombatShips>(frame)?;
    let ships_at: Vec<_> = ships.rows().iter().filter(|r| r.combat == id).collect();
    let tasks: BTreeMap<_, _> = rows::<tables::TaskForces>(frame)?
        .rows()
        .iter()
        .map(|r| (r.id, r.province))
        .collect();
    if let Some(region) = document.counters.region(detail.province) {
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
                let terrain = document
                    .strategic_regions
                    .get(&region)
                    .and_then(|r| r.naval_terrain.as_deref());
                let night = *nights.first().expect("one night value");
                let picture = |ongoing| {
                    picture_names(terrain, night, weather_suffix(weather), ongoing)
                        .into_iter()
                        .find(|n| document.icons.combat.has_sprite(n))
                };
                // An undecided prefix decides nothing unless both pictures agree.
                let name = match ongoing {
                    Ok(ongoing) => picture(ongoing),
                    Err(cause) => match (picture(true), picture(false)) {
                        (with, without) if with == without => with,
                        _ => {
                            out.warnings.push(format!("Naval environment: whether the viewer sees a combat at the battle province (1406F3A90, for the ongoing_sea_battle_ prefix) is unknown, {}; terrain image remains empty.", cause.describe()));
                            None
                        }
                    },
                };
                if let Some(name) = name {
                    out.values.insert(
                        "header_container/terrain_background".into(),
                        Widget {
                            sprite: Some(name),
                            ..Widget::default()
                        },
                    );
                }
            } else {
                out.warnings.push("Naval environment: no consistent recorded province-night operand joins a live task force at the battle province; terrain image remains empty.".into());
            }
            weather_icons(out, document, weather);
        } else {
            out.warnings.push("Naval environment: WeatherRegions has no row for this battle; weather widgets remain empty.".into());
        }
    }
    if ships.is_complete() {
        let prides: BTreeSet<_> = ships_at
            .iter()
            .filter(|r| r.live_ship != 0 && r.pride != 0)
            .map(|r| r.owner)
            .collect();
        for side in 0..2_u8 {
            // 1417D9C60 walks active source groups and accepts exactly2/8.
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
                    .filter(|r| r.live_ship != 0 || r.live_convoy == 0)
                    .try_fold(0, |sum, r| {
                        Some(add(sum, weapon(r, case, prides.contains(&r.owner))?))
                    })
                    .and_then(|ships| {
                        bounded_total(ships, convoys, document.icons.combat.convoy_weapon_bound)
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
    out.warnings.push("Naval environment/summary: weather picture uses the native heavy-rain/light-rain/blizzard/snow priority and recorded task-force province-night join; totals use active source0 state2/8 members and native ordered weapon modifiers, doctrine, weather, critical and HQ arithmetic. UNVERIFIED: high-speed UI night override, weather-icon category eligibility (+64 definition flags), custom sea modifiers, pride-country alias eligibility and zero-decimal tie rounding require their missing operands or a same-hour format22 capture. Convoy weapon values are not recorded: a total is shown only when every convoy equipment value the content allows leaves its whole number unchanged, otherwise dash. AA/positioning remain recorded cached values: UI AA recomputation requires ship+218 base AA, which weapon_bases does not record, plus modifiers88/89 and positioning. Modifier tooltips remain unported.".into());
    Ok(())
}
fn weather_icons(out: &mut Bindings, document: &Document, weather: &WeatherRegionRow) {
    let mut entries = Vec::new();
    for (active, key) in [
        (weather.light_rain, "rain_light_icon"),
        (weather.heavy_rain, "rain_heavy_icon"),
        (weather.snow, "snow_icon"),
        (weather.blizzard, "blizzard_icon"),
        (weather.sandstorm, "sandstorm_icon"),
        (weather.arctic_water, "arctic_water_icon"),
    ] {
        if active == 0 {
            continue;
        }
        let Some(token) = document.names.localisation.get(key) else {
            continue;
        };
        // 141D8A180 resolves localisation to "sprite:one-based-frame".
        let (sprite, frame) = token.split_once(':').map_or((token, 0), |(s, f)| {
            (s, f.parse::<u32>().unwrap_or(1).saturating_sub(1))
        });
        if document.icons.combat.has_sprite(sprite) {
            let mut values = Values::new();
            values.insert(
                "icon".into(),
                Widget {
                    sprite: Some(sprite.into()),
                    frame: Some(frame),
                    ..Widget::default()
                },
            );
            entries.push(values);
        }
    }
    out.values.insert(
        "header_container/modifier_grid".into(),
        Widget {
            entry_template: Some("naval_region_modifier_entry".into()),
            entries,
            ..Widget::default()
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn weather_picture_uses_ui_precedence_and_terrain_fallback() {
        let mut w = WeatherRegionRow {
            light_rain: 1,
            heavy_rain: 1,
            blizzard: 1,
            snow: 1,
            ..Default::default()
        };
        assert_eq!(weather_suffix(&w), "storm");
        w.heavy_rain = 0;
        assert_eq!(weather_suffix(&w), "rain");
        w.light_rain = 0;
        assert_eq!(weather_suffix(&w), "snow_storm");
        assert_eq!(
            picture_names(Some("water_deep_ocean"), true, "rain", true),
            vec![
                "GFX_terrain_ongoing_sea_battle_water_deep_ocean_night_rain",
                "GFX_terrain_ongoing_sea_battle_water_deep_ocean_night",
                "GFX_terrain_ongoing_sea_battle_water_night_rain",
                "GFX_terrain_ongoing_sea_battle_water_night"
            ]
        );
    }
    #[test]
    fn a_convoy_total_is_shown_only_when_no_allowed_value_changes_the_whole_number() {
        // pairJ_naval_5.png: one Romanian convoy, content bound 0.25, shows 0.
        assert_eq!(bounded_total(0, 1, Some(25_000)), Some(0));
        assert_eq!(bounded_total(8_190_000, 0, Some(25_000)), Some(81));
        assert_eq!(bounded_total(8_190_000, 1, Some(25_000)), None);
        assert_eq!(bounded_total(0, 4, Some(25_000)), None);
        assert_eq!(bounded_total(0, 1, None), None);
    }

    #[test]
    fn carrier_penalty_free_allowance_counts_only_eligible_active_planes() {
        use replay::rows::NavalCombatPlane;
        let mut side = NavalCombatSideDetails::default();
        side.summary.integer_defines[2] = 80;
        assert!(no_carrier_penalty(&side));
        side.planes = vec![
            NavalCombatPlane {
                source: 0,
                carrier_state: 2,
                current_planes: 80,
                ..Default::default()
            },
            NavalCombatPlane {
                source: 1,
                carrier_state: 2,
                current_planes: 200,
                ..Default::default()
            },
            NavalCombatPlane {
                source: 0,
                carrier_state: 32,
                current_planes: 200,
                ..Default::default()
            },
        ];
        assert!(no_carrier_penalty(&side));
        side.planes[0].current_planes += 1;
        assert!(!no_carrier_penalty(&side));
    }
    #[test]
    fn doctrine_weight_and_hq_maximum_are_not_flat_sums() {
        use replay::rows::NavalShipModifierTerm;
        let value = |raw| {
            vec![NavalShipModifierTerm {
                key: 7,
                raw,
                ..Default::default()
            }]
        };
        let context = NavalShipWeaponContext {
            sources: vec![
                NavalShipModifierSource {
                    scope: 4,
                    weights: [1, 2],
                    terms: value(20_000),
                    ..Default::default()
                },
                NavalShipModifierSource {
                    scope: 12,
                    index: 0,
                    terms: value(3_000),
                    ..Default::default()
                },
                NavalShipModifierSource {
                    scope: 13,
                    index: 0,
                    terms: value(4_000),
                    ..Default::default()
                },
                NavalShipModifierSource {
                    scope: 12,
                    index: 1,
                    terms: value(5_000),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(modifier(&context, 7), Some(17_000));
    }

    /// 140E69830 takes the ongoing prefix from any seen combat at the battle
    /// province. Every NavalCombats identity there counts, so two combats in
    /// one province: one without a detail row, or a NavalCombats table that
    /// is not whole, keeps a proven positive and refuses a negative.
    #[test]
    fn the_ongoing_prefix_needs_every_combat_at_the_province() {
        use replay::{
            frame::TableView,
            rows::{NavalCombatDetailsRow, NavalCombatRow, TableId, TableRows},
        };
        // GER (100 -> 0) views and ITA (102 -> 2) shares; SOV (101 -> 1) does not.
        let aliases = BTreeMap::from([(100, 0), (101, 1), (102, 2)]);
        let vision = Vision::sharing(0, [2]);
        let details = |rows: &[(u64, u32)]| {
            TableRows::NavalCombatDetails(
                rows.iter()
                    .map(|&(combat, country)| {
                        let mut row = NavalCombatDetailsRow {
                            combat,
                            province: 500,
                            ..Default::default()
                        };
                        row.sides[0].summary.countries = vec![country];
                        row
                    })
                    .collect(),
            )
        };
        let combats = || {
            TableRows::NavalCombats(
                [(1, 500), (2, 500), (3, 600)]
                    .map(|(id, province)| NavalCombatRow {
                        id,
                        province,
                        ..Default::default()
                    })
                    .to_vec(),
            )
        };
        let ongoing = |combats: TableView, details: TableView| {
            let mut frame = DecodedFrame {
                branch: 1,
                hour: 5,
                tables: std::array::from_fn(|_| {
                    TableView::Unavailable {
                        reason: "not captured".into(),
                    }
                    .into()
                }),
                metrics: Default::default(),
            };
            *frame.tables[TableId::NavalCombats.index()] = combats;
            *frame.tables[TableId::NavalCombatDetails.index()] = details;
            ongoing_battle(&frame, 500, &aliases, &vision).unwrap()
        };
        let truncated = |rows| TableView::Truncated { rows, missing: 1 };
        // Both combats at 500 are SOV's, and every table is whole: unseen.
        assert_eq!(
            ongoing(
                TableView::Complete(combats()),
                TableView::Complete(details(&[(1, 101), (2, 101)]))
            ),
            Ok(false)
        );
        // Combat 2 has no detail row, the table whole or not: unknown.
        for rows in [
            TableView::Complete(details(&[(1, 101)])),
            truncated(details(&[(1, 101)])),
        ] {
            assert_eq!(
                ongoing(TableView::Complete(combats()), rows),
                Err(SharingUnknown::Census(NAVAL_DETAILS_ROW_MISSING))
            );
        }
        // A proven seen combat decides it whatever else is missing.
        assert_eq!(
            ongoing(
                TableView::Unavailable {
                    reason: "combats read failed".into()
                },
                truncated(details(&[(2, 102)]))
            ),
            Ok(true)
        );
        // NavalCombats not whole: another combat may be at the province.
        assert_eq!(
            ongoing(
                truncated(combats()),
                TableView::Complete(details(&[(1, 101), (2, 101)]))
            ),
            Err(SharingUnknown::Census(NAVAL_COMBATS_NOT_WHOLE))
        );
    }
}
