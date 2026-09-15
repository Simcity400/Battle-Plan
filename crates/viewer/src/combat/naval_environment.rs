//! Native naval environment selection and displayed gun/depth totals.
use super::{
    Values, Widget,
    binding::{Bindings, text},
};
use crate::{
    document::Document,
    map::{perspective::Vision, tables::rows},
};
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
                let weather = add(ONE, source_term(context, 0, 0x248)?).clamp(0, ONE);
                mul(
                    term(context, source, key)?,
                    add(context.weather_factor - ONE, weather),
                )
            }
            11 => {
                let severity = if key == 0x1bb {
                    ONE
                } else {
                    add(ONE, modifier(context, 0x1bb)?)
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
    let mut base = context.weapon_bases[case];
    let mut factor = 0;
    // 140C25310 uses the cached formation flags at ship+628, without
    // substituting ship category or adding a strength multiplier.
    if context.formation_flags & (1 << 7) != 0 {
        if !depth {
            base = add(base, modifier(context, 0x38)?);
        }
        factor = add(
            modifier(context, 0xc2 + u32::from(depth))?,
            source_term(context, 16, 0x1cc + u32::from(depth))?,
        );
    }
    for (mask, key, relation) in [
        // Native instructions140C254B9/140C25527 add positive C4/C6.
        // The decompiler sign-extends these immediates incorrectly.
        (1 << 6, 0xc4_u32, 0x1c8),
        (1 << 8, 0xc6_u32, 0x1ca),
    ] {
        if context.formation_flags & mask != 0 {
            factor = add(
                factor,
                add(
                    modifier(context, key + u32::from(depth))?,
                    source_term(context, 16, relation + u32::from(depth))?,
                ),
            );
        }
    }
    if base == 0 {
        return Some(0);
    }
    if !depth {
        factor = add(factor, mul(modifier(context, 0x107)?, context.night));
    }
    factor = add(factor, modifier(context, 0x1b7 + u32::from(depth))?);
    let lane = usize::from(depth);
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
        factor = add(factor, source_term(context, 9, 0x1d1 + u32::from(depth))?);
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
    let aliases: BTreeMap<_, _> = rows::<tables::CountryLinks>(frame)?
        .rows()
        .iter()
        .map(|r| (r.id, r.canonical_slot))
        .collect();
    // 140E69830: any intel-visible combat at this province enables the
    // ongoing_sea_battle_ prefix. 1413CD880 uses either side's countries.
    let ongoing = details
        .rows()
        .iter()
        .filter(|r| r.province == detail.province)
        .any(|r| {
            vision.sees_combat(
                r.sides
                    .iter()
                    .flat_map(|side| &side.summary.countries)
                    .filter_map(|country| aliases.get(country).copied()),
            )
        });
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
                if let Some(name) = picture_names(
                    terrain,
                    *nights.first().expect("one night value"),
                    weather_suffix(weather),
                    ongoing,
                )
                .into_iter()
                .find(|n| document.icons.combat.sprites.contains_key(n))
                {
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
                let sum = members.iter().try_fold(0, |sum, r| {
                    Some(add(sum, weapon(r, case, prides.contains(&r.owner))?))
                });
                let wnd = if side == 0 { "left" } else { "right" };
                if let Some(sum) = sum {
                    text(
                        &mut out.values,
                        &format!("header_container/{wnd}_wnd/{widget}"),
                        (sum / ONE).to_string(),
                    );
                } else {
                    out.warnings.push(format!("Naval {wnd} {widget}: convoy equipment weapons or an unsupported modifier operand is missing; total remains dash."));
                }
            }
        }
    }
    out.warnings.push("Naval environment/summary: weather picture uses the native heavy-rain/light-rain/blizzard/snow priority and recorded task-force province-night join; totals use active source0 state2/8 members and native ordered weapon modifiers, doctrine, weather, critical and HQ arithmetic. UNVERIFIED: high-speed UI night override, weather-icon category eligibility (+64 definition flags), custom sea modifiers, pride-country alias eligibility and zero-decimal tie rounding require their missing operands or a same-hour format22 capture. Convoy weapon values are not recorded; affected totals stay dash. AA/positioning remain recorded cached values: UI AA recomputation requires ship+218 base AA, which weapon_bases does not record, plus modifiers88/89 and positioning. Modifier tooltips remain unported.".into());
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
        if document.icons.combat.sprites.contains_key(sprite) {
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
}
