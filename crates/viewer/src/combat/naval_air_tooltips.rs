//! Air-group icon hovers assembled from each CFEXAir's recorded operands.
use super::binding::{TextIcons, builtin, displayable};
use installed_content::localisation::{Argument, Localisation};
use replay::rows::{ModifierDefinitionRow, NavalCombatPlane, NavalCombatShipRow};

// Executable keys in 1417D2430 and 1415B46D0. Category names are selected
// by 140F734A0 and then localised by 140F742C0.
super::text::tooltip_keys! {
    NAVAL_COMBAT_AIRWING_TOOLTIP_HEADER = "NAVAL_COMBAT_AIRWING_TOOLTIP_HEADER",
    NAVAL_COMBAT_AIRWING_TOOLTIP_HEADER_EXTERNAL = "NAVAL_COMBAT_AIRWING_TOOLTIP_HEADER_EXTERNAL",
    NAVAL_COMBAT_AIRWING_TOOLTIP_KILLS_HEADER = "NAVAL_COMBAT_AIRWING_TOOLTIP_KILLS_HEADER",
    NAVAL_COMBAT_AIRWING_TOOLTIP_KILLS = "NAVAL_COMBAT_AIRWING_TOOLTIP_KILLS",
    NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED = "NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED",
    NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED_ENTRY = "NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED_ENTRY",
    NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED_LAST_HIT = "NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED_LAST_HIT",
    CARRIER_EFFICIENCY = "CARRIER_EFFICIENCY",
    CARRIER_EFFICIENCY_BAD = "CARRIER_EFFICIENCY_BAD",
    CARRIER_EFFICIENCY_CROWD = "CARRIER_EFFICIENCY_CROWD",
    CARRIER_EFFICIENCY_WEATHER = "CARRIER_EFFICIENCY_WEATHER",
    CARRIER_EFFICIENCY_NIGHT = "CARRIER_EFFICIENCY_NIGHT",
    CARRIER_EFFICIENCY_DESC = "CARRIER_EFFICIENCY_DESC",
    CARRIER_EFFICIENCY_ORG = "CARRIER_EFFICIENCY_ORG",
    STAT_BASE_VALUE = "STAT_BASE_VALUE",
    STAT_ADJUSTER_BASE = "STAT_ADJUSTER_BASE",
    MODIFIER_CARRIER_NIGHT_TRAFFIC = "MODIFIER_CARRIER_NIGHT_TRAFFIC",
    SCREENING_VALUE = "SCREENING_VALUE",
}

fn stored(loc: &Localisation, recorded: &str, value: &[u8], what: &str) -> Result<String, String> {
    dashboard::scripted_text::stored_text(loc, recorded, value, what)
}

/// 14160C430 plane-stacking multiplier, but returns zero for an empty array.
fn wings_striking(total: usize, penalty: i64) -> i32 {
    if total == 0 {
        return 0;
    }
    let raw = (total as i64).wrapping_mul(penalty);
    let rounded = if raw < 0 {
        raw.wrapping_sub(50_000) / 100_000
    } else {
        raw.wrapping_add(50_000) / 100_000
    };
    rounded.max(1) as i32
}

/// 140F734A0's fixed executable key switch. A category bit set the switch
/// does not recognise uses its EQUIPMENT_UNKNOWN default.
fn category_key(mask: u64) -> &'static str {
    match mask {
        1 => "EQUIPMENT_CONVOY",
        4 => "EQUIPMENT_ARMOR",
        8 => "EQUIPMENT_MOTORIZED",
        0x10 => "EQUIPMENT_MECHANIZED",
        0x20 => "EQUIPMENT_INFANTRY",
        0x40 => "EQUIPMENT_CAPITAL_SHIP",
        0x80 => "EQUIPMENT_SUBMARINE",
        0x100 => "EQUIPMENT_SCREEN",
        0x200 => "EQUIPMENT_FLOATING_HARBOR",
        0x201 => "EQUIPMENT_NON_TASKFORCE_NAVAL",
        0x400 => "EQUIPMENT_SMALL_FIGHTER",
        0x800 => "EQUIPMENT_INTERCEPTOR",
        0x1000 => "EQUIPMENT_TACTICAL_BOMBER",
        0x2000 => "EQUIPMENT_STRATEGIC_BOMBER",
        0x4000 => "EQUIPMENT_CAS",
        0x8000 => "EQUIPMENT_NAVAL_BOMBER",
        0x1_0000 => "EQUIPMENT_MISSILE",
        0x2_0000 => "EQUIPMENT_SUICIDE",
        0x4_0000 => "EQUIPMENT_SCOUT_PLANE",
        0x8_0000 => "EQUIPMENT_RAILWAY_GUN",
        0x10_0000 => "EQUIPMENT_AIR_TRANSPORT",
        0x20_0000 => "EQUIPMENT_MARITIME_PATROL_PLANE",
        0x20_b000 => "EQUIPMENT_BOMBER",
        0x40_0000 => "EQUIPMENT_CARRIER",
        0x80_0000 => "EQUIPMENT_PARACHUTE",
        0x100_0000 => "EQUIPMENT_SUPPORT",
        0x1d0_0000 => "EQUIPMENT_SPECIAL",
        0x200_0000 => "EQUIPMENT_FLAME",
        0x400_0000 => "EQUIPMENT_AMPHIBIOUS",
        0x800_0000 => "EQUIPMENT_ANTI_AIR",
        0x1000_0000 => "EQUIPMENT_ARTILLERY",
        0x2000_0000 => "EQUIPMENT_ANTI_TANK",
        0x4000_0000 => "EQUIPMENT_ROCKET",
        0x7e00_0000 => "EQUIPMENT_WEAPONS",
        0x8000_0000 => "EQUIPMENT_TRAIN",
        0x1_0000_0000 => "EQUIPMENT_HEAVY_FIGHTER",
        0x1_0000_0400 => "EQUIPMENT_FIGHTER",
        0x1_0036_fc00 => "EQUIPMENT_PLANE",
        0x1f_0037_fc00 => "EQUIPMENT_AIR",
        0x2_0000_0000 => "EQUIPMENT_EMPLACEMENT_GUN_AMMO",
        0x4_0000_0000 => "EQUIPMENT_BALLISTIC_MISSILE",
        0x8_0000_0000 => "EQUIPMENT_NUCLEAR_MISSILE",
        0x10_0000_0000 => "EQUIPMENT_SAM_MISSILE",
        0x1c_0001_0000 => "EQUIPMENT_MISSILE_CATEGORY",
        0x20_0000_0000 => "EQUIPMENT_MISSILE_LAUNCHER",
        0x40_0000_0000 => "EQUIPMENT_LAND_CRUISER",
        0x40_8000_003c => "EQUIPMENT_LAND",
        0x80_0000_0000 => "EQUIPMENT_SUPPORT_SHIP",
        0x0080_0040_03c1 => "EQUIPMENT_NAVAL",
        _ => "EQUIPMENT_UNKNOWN",
    }
}

/// LocalizeShortNameForCountry@140BBAA50 checks TAG_key_short, key_short,
/// then LocalizeForCountry@140BBA780 for the ordinary key.
pub(super) fn wing_short_name(loc: &Localisation, tag: &str, key: &str) -> Result<String, String> {
    let short = format!("{key}_short");
    let qualified = format!("{tag}_{short}");
    let selected = if loc.get(&qualified).is_some() {
        qualified
    } else if loc.get(&short).is_some() {
        short
    } else {
        return super::text::country_localised(loc, Ok(tag), key);
    };
    super::text::localise(loc, &selected, &[])
}

/// 141963700 -> 14195EEC0 -> 140C29020 appends each linked carrier source's
/// cloned child tree in the Table 270 source order. The winning HQ influence
/// and doctrine/critical display scales are the same as the ship hovers.
pub(super) fn carrier_key_breakdown(
    member: &NavalCombatShipRow,
    rows: &[&super::naval_summary_tooltips::NavalTreeUse<'_>],
    definition: Option<&ModifierDefinitionRow>,
    key: u32,
    recorded_language: &str,
    loc: &Localisation,
    icons: &dyn TextIcons,
) -> Result<String, String> {
    let mut text = String::new();
    for row in rows {
        if row.key != key {
            return Err("carrier modifier tree key differs from requested sortie key".into());
        }
        if !super::naval_environment::breakdown_visible(
            &member.weapon_context,
            key,
            row.use_.source_scope,
            row.use_.source_index,
        )
        .ok_or("carrier sortie HQ influence selection is unavailable")?
        {
            continue;
        }
        let definition = definition.ok_or("carrier sortie modifier has no recorded definition")?;
        let scale = super::naval_environment::breakdown_scale(
            &member.weapon_context,
            row.use_.source_scope,
            row.use_.source_index,
        )
        .ok_or("carrier sortie modifier source scale is unavailable")?;
        text.push_str(&super::naval_summary_tooltips::ship_modifier_breakdown(
            &row.nodes,
            definition,
            scale,
            "  ",
            recorded_language,
            icons,
            loc,
            !matches!(row.use_.source_scope, 12..=15),
        )?);
    }
    Ok(text)
}

/// 1417D2430 joins 141963460's distinct wing short names with comma-space,
/// then formats its two native header branches and the recorded hit vectors.
#[allow(clippy::too_many_arguments)] // The plane's recorded row, its native texts and text resources.
pub(super) fn plane_hover(
    plane: &NavalCombatPlane,
    name: &str,
    names: &str,
    efficiency_suffix: &str,
    hour: i32,
    stacking_penalty: i64,
    recorded_language: &str,
    loc: &Localisation,
    icons: &dyn TextIcons,
) -> Result<String, String> {
    let total_wings = i32::try_from(plane.wings[0].len())
        .map_err(|_| "CFEXAir+0x28 wing count exceeds i32".to_owned())?;
    let striking = wings_striking(plane.wings[0].len(), stacking_penalty);
    let mut text = if plane.flags[1] == 0 {
        builtin(
            icons,
            loc,
            NAVAL_COMBAT_AIRWING_TOOLTIP_HEADER,
            &[
                (
                    "NUM",
                    Argument::Int32(plane.initial_planes.wrapping_sub(plane.casualties)),
                ),
                ("TOT", Argument::Int32(plane.initial_planes)),
                ("MOD", Argument::Text(names)),
                ("FROM", Argument::Text(name)),
                ("DEAD", Argument::Int32(plane.casualties)),
                ("WINGS_STRIKING", Argument::Int32(striking)),
                ("TOTAL_WINGS", Argument::Int32(total_wings)),
            ],
        )?
    } else {
        builtin(
            icons,
            loc,
            NAVAL_COMBAT_AIRWING_TOOLTIP_HEADER_EXTERNAL,
            &[
                (
                    "NUM",
                    Argument::Int32(plane.current_planes.max(plane.initial_planes)),
                ),
                ("MOD", Argument::Text(names)),
                ("FROM", Argument::Text(name)),
                ("HOUR", Argument::Int32(hour.wrapping_sub(plane.date))),
                ("DEAD", Argument::Int32(plane.casualties)),
                ("WINGS_STRIKING", Argument::Int32(striking)),
                ("TOTAL_WINGS", Argument::Int32(total_wings)),
            ],
        )?
    };
    // 1417D2430 folds equal +9F8 wing categories in first occurrence order.
    let mut kills: Vec<(u64, i32)> = Vec::new();
    for hit in &plane.kills {
        if let Some((_, total)) = kills
            .iter_mut()
            .find(|(category, _)| *category == hit.category)
        {
            *total = total.wrapping_add(hit.count);
        } else {
            kills.push((hit.category, hit.count));
        }
    }
    if !kills.is_empty() {
        text.push_str(&builtin(
            icons,
            loc,
            NAVAL_COMBAT_AIRWING_TOOLTIP_KILLS_HEADER,
            &[],
        )?);
        for (category, count) in kills {
            let category = builtin(icons, loc, category_key(category), &[])?;
            text.push_str(&builtin(
                icons,
                loc,
                NAVAL_COMBAT_AIRWING_TOOLTIP_KILLS,
                &[
                    ("NUM", Argument::Int32(count)),
                    ("TYPE", Argument::Text(&category)),
                ],
            )?);
        }
    }
    if !plane.naval_hits.is_empty() {
        text.push_str(&builtin(
            icons,
            loc,
            NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED,
            &[],
        )?);
        for hit in &plane.naval_hits {
            let target = stored(
                loc,
                recorded_language,
                &hit.target_name,
                "CFEXAir+0x100 damaged ship name",
            )?;
            let ratio = if hit.denominator < 1 {
                hit.damage
            } else if (hit.damage.wrapping_add(0x7fff_ffff_ffff_fffe) as u64)
                < 0xffff_ffff_ffff_fffd
            {
                hit.damage.wrapping_mul(100_000) / hit.denominator
            } else {
                0xffff_ffff
            };
            text.push_str(&builtin(
                icons,
                loc,
                NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED_ENTRY,
                &[
                    ("NAVY", Argument::Text(&target)),
                    ("PERC", Argument::Fixed5(ratio)),
                ],
            )?);
            if hit.last_hit != 0 {
                text.push_str(&builtin(
                    icons,
                    loc,
                    NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED_LAST_HIT,
                    &[],
                )?);
            }
        }
    }
    // 1417D2430 appends 141965010's first eligible wing text after the
    // damaged-ship block, inserting one newline only when one is needed.
    if !efficiency_suffix.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(efficiency_suffix);
    }
    displayable(icons, text)
}

/// 1417DAA40 starts with the first plane's carrier-efficiency text, then
/// ensures two trailing newlines before every 1417D2430 member description.
#[allow(clippy::too_many_arguments)] // The group's recorded rows, native texts and text resources.
pub(super) fn group_hover(
    planes: &[&NavalCombatPlane],
    presentations: &[(String, String)],
    efficiency_suffixes: &[String],
    prefix: &str,
    hour: i32,
    stacking_penalty: i64,
    recorded_language: &str,
    loc: &Localisation,
    icons: &dyn TextIcons,
) -> Result<String, String> {
    if planes.len() != efficiency_suffixes.len() || planes.len() != presentations.len() {
        return Err("air-group presentation count differs from plane count".to_owned());
    }
    let mut text = prefix.to_owned();
    for ((plane, suffix), (name, names)) in
        planes.iter().zip(efficiency_suffixes).zip(presentations)
    {
        while !text.ends_with("\n\n") {
            text.push('\n');
        }
        text.push_str(&plane_hover(
            plane,
            name,
            names,
            suffix,
            hour,
            stacking_penalty,
            recorded_language,
            loc,
            icons,
        )?);
    }
    Ok(text)
}

/// 141965010 appends the overall efficiency and an optional host-organisation
/// parenthesis for the first wing whose air base has no state at +0x68.
pub(super) fn carrier_org_suffix(
    final_efficiency: i64,
    organisation_ratio: i64,
    loc: &Localisation,
    icons: &dyn TextIcons,
) -> Result<String, String> {
    if final_efficiency == 100_000 {
        return Ok(String::new());
    }
    let mut text = builtin(
        icons,
        loc,
        if final_efficiency < 50_000 {
            CARRIER_EFFICIENCY_BAD
        } else {
            CARRIER_EFFICIENCY
        },
        &[("VAL", Argument::Fixed5(final_efficiency))],
    )?;
    if organisation_ratio != 100_000 {
        let colour = if organisation_ratio < 100_000 {
            'R'
        } else {
            'G'
        };
        let percent = installed_content::fixed5::fixed5_percent(organisation_ratio, 0, true);
        let value = format!("\u{11}{colour}{percent}\u{11}!");
        text.push_str(" (");
        text.push_str(&builtin(
            icons,
            loc,
            CARRIER_EFFICIENCY_ORG,
            &[("VAL", Argument::Text(&value))],
        )?);
        text.push(')');
    }
    text.push('\n');
    Ok(text)
}

/// 141963700 appends the efficiency explanation only for an active carrier
/// wing whose sortie efficiency differs from ONE. The factors are the already
/// recorded sortie inputs; its fixed-base line deliberately uses the define
/// even during the initial stance interval, as the native formatter does.
pub(super) fn efficiency_prefix(
    plane: &NavalCombatPlane,
    sortie_efficiency: i64,
    base: i64,
    member_breakdown: &str,
    loc: &Localisation,
    icons: &dyn TextIcons,
) -> Result<String, String> {
    const ONE: i64 = 100_000;
    if plane.wings[1].is_empty() || plane.efficiency.source != 3 || sortie_efficiency == ONE {
        return Ok(String::new());
    }
    let e = &plane.efficiency;
    let value = |index: usize| -> Result<i64, String> {
        let term = &e.modifiers[index];
        if term.present == 0 {
            return Ok(0);
        }
        super::naval_air_groups::metadata_value(e, usize::from(index == 0), term.raw).ok_or_else(
            || format!("CFEXAir carrier efficiency modifier {index} metadata is unavailable"),
        )
    };
    let crowd = if e.container_present != 0 {
        e.base_crowding
    } else {
        0
    };
    let weather = ONE.wrapping_add(value(1)?).clamp(0, ONE);
    let province_night = value(2)?;
    let country_night = value(0)?;
    let night = ONE
        .wrapping_add(province_night)
        .wrapping_add(if e.night > 0 { country_night } else { 0 })
        .clamp(0, ONE);
    let mut text = builtin(
        icons,
        loc,
        if sortie_efficiency < 50_000 {
            CARRIER_EFFICIENCY_BAD
        } else {
            CARRIER_EFFICIENCY
        },
        &[("VAL", Argument::Fixed5(sortie_efficiency))],
    )?;
    text.push('\n');
    // 141963700 formats this base percent at precision zero, green H.
    let base = installed_content::fixed5::fixed5_percent(base, 0, true);
    text.push_str(&builtin(icons, loc, STAT_BASE_VALUE, &[])?);
    text.push_str(&format!("\u{11}H{base}\u{11}!\n"));
    text.push_str(member_breakdown);
    let coloured = |value, factor| {
        let colour = if value < ONE { 'R' } else { 'G' };
        let value = installed_content::fixed5::fixed5_percent(value, 0, true);
        format!(
            "\u{11}{colour}{}{value}\u{11}!",
            if factor { "x" } else { "" }
        )
    };
    for (value, key) in [
        (crowd, CARRIER_EFFICIENCY_CROWD),
        (weather, CARRIER_EFFICIENCY_WEATHER),
        (night, CARRIER_EFFICIENCY_NIGHT),
    ] {
        if value != ONE {
            text.push_str(&builtin(
                icons,
                loc,
                key,
                &[("VAL", Argument::Text(&coloured(value, true)))],
            )?);
            text.push('\n');
        }
        if key == CARRIER_EFFICIENCY_NIGHT && value != ONE {
            if e.modifiers[2].present != 0 {
                text.push_str("    ");
                text.push_str(&builtin(
                    icons,
                    loc,
                    STAT_ADJUSTER_BASE,
                    &[("VALUE", Argument::Text(&coloured(province_night, false)))],
                )?);
                text.push('\n');
            }
            if country_night != 0 {
                text.push_str("    ");
                text.push_str(&builtin(icons, loc, MODIFIER_CARRIER_NIGHT_TRAFFIC, &[])?);
                text.push_str(": ");
                // 141963700 hard-codes green here, including a negative
                // country modifier; the subtotal above retains its own red.
                let country = installed_content::fixed5::fixed5_percent(country_night, 0, true);
                text.push_str(&format!("\u{11}G{country}\u{11}!"));
                text.push('\n');
            }
        }
    }
    text.push_str(&builtin(icons, loc, CARRIER_EFFICIENCY_DESC, &[])?);
    displayable(icons, text)
}

/// 14195EEC0 emits this line after a nonzero box-2/3 screening bonus; its
/// caller in 141963700 supplies the two-space indentation string.
pub(super) fn screening_line(
    value: i64,
    loc: &Localisation,
    icons: &dyn TextIcons,
) -> Result<String, String> {
    if value == 0 {
        return Ok(String::new());
    }
    Ok(format!(
        "  {}\n",
        builtin(
            icons,
            loc,
            SCREENING_VALUE,
            &[("NUM", Argument::Fixed5(value))]
        )?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::binding::NoIcons;
    use replay::rows::{NavalCombatAirWingKill, NavalCombatHit};

    fn loc() -> Localisation {
        let mut loc: Localisation = [
            (
                NAVAL_COMBAT_AIRWING_TOOLTIP_HEADER,
                "$NUM$/$TOT$ $MOD$ from $FROM$; $DEAD$; $WINGS_STRIKING$/$TOTAL_WINGS$",
            ),
            (
                NAVAL_COMBAT_AIRWING_TOOLTIP_HEADER_EXTERNAL,
                "$NUM$ $MOD$ from $FROM$; $HOUR$; $DEAD$; $WINGS_STRIKING$/$TOTAL_WINGS$",
            ),
            (NAVAL_COMBAT_AIRWING_TOOLTIP_KILLS_HEADER, "\nKills:"),
            (NAVAL_COMBAT_AIRWING_TOOLTIP_KILLS, "\n$NUM$ x $TYPE$"),
            (NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED, "\nDamage:"),
            (
                NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED_ENTRY,
                "\n$NAVY$: $PERC$",
            ),
            (NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED_LAST_HIT, " killed"),
            (CARRIER_EFFICIENCY, "Sortie $VAL$"),
            (CARRIER_EFFICIENCY_BAD, "Low $VAL$"),
            (CARRIER_EFFICIENCY_CROWD, "Crowd $VAL$"),
            (CARRIER_EFFICIENCY_WEATHER, "Weather $VAL$"),
            (CARRIER_EFFICIENCY_NIGHT, "Night $VAL$"),
            (CARRIER_EFFICIENCY_DESC, "Why."),
            (CARRIER_EFFICIENCY_ORG, "Org $VAL$"),
            (STAT_BASE_VALUE, "Base: "),
            (STAT_ADJUSTER_BASE, "Night base $VALUE$"),
            (MODIFIER_CARRIER_NIGHT_TRAFFIC, "Traffic"),
            (SCREENING_VALUE, "Screen $NUM$"),
            ("EQUIPMENT_FIGHTER", "Fighter"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        loc.language = "english".into();
        loc
    }

    #[test]
    fn derived_name_and_ordered_strings_build_the_native_header_and_kills() {
        let row = NavalCombatPlane {
            initial_planes: 12,
            casualties: 2,
            wings: [vec![1, 2], vec![]],
            kills: vec![NavalCombatAirWingKill {
                category: 0x1_0000_0400,
                count: 3,
                ..Default::default()
            }],
            naval_hits: vec![NavalCombatHit {
                target_name: b"Destroyer".to_vec(),
                damage: 25_000,
                denominator: 100_000,
                last_hit: 1,
                ..Default::default()
            }],
            ..Default::default()
        };
        let hover = plane_hover(
            &row,
            "Mediterranean",
            "Spitfire, Hurricane",
            "",
            100,
            50_000,
            "english",
            &loc(),
            &NoIcons,
        )
        .unwrap();
        assert!(
            hover.starts_with("10/12 Spitfire, Hurricane from Mediterranean; 2; 1/2"),
            "{hover}"
        );
        assert!(hover.contains("\nKills:\n3 x Fighter"), "{hover}");
        assert!(
            hover.contains("\nDamage:\nDestroyer: 0.25 killed"),
            "{hover}"
        );
        let mut no_stored_hits = row.clone();
        no_stored_hits.naval_hits.clear();
        let german = plane_hover(
            &no_stored_hits,
            "Mittelmeer",
            "Spitfire, Hurricane",
            "",
            100,
            50_000,
            "german",
            &loc(),
            &NoIcons,
        )
        .unwrap();
        assert!(german.contains("from Mittelmeer"), "{german}");
    }

    #[test]
    fn wing_short_name_prefers_country_then_generic_then_ordinary_key() {
        let fixture = |country, generic| {
            let mut entries = vec![("fighter", "Ordinary")];
            if country {
                entries.push(("TAG_fighter_short", "Country short"));
            }
            if generic {
                entries.push(("fighter_short", "Generic short"));
            }
            entries
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect::<Localisation>()
        };
        assert_eq!(
            wing_short_name(&fixture(true, true), "TAG", "fighter").unwrap(),
            "Country short"
        );
        assert_eq!(
            wing_short_name(&fixture(false, true), "TAG", "fighter").unwrap(),
            "Generic short"
        );
        assert_eq!(
            wing_short_name(&fixture(false, false), "TAG", "fighter").unwrap(),
            "Ordinary"
        );
    }

    #[test]
    fn sortie_breakdown_and_organisation_suffix_have_native_order_and_text() {
        let mut plane = NavalCombatPlane {
            wings: [vec![], vec![7]],
            efficiency: replay::rows::NavalCombatAirEfficiency {
                source: 3,
                container_present: 1,
                base_crowding: 75_000,
                night: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        for (modifier, raw) in plane
            .efficiency
            .modifiers
            .iter_mut()
            .zip([-10_000, -20_000, -25_000])
        {
            modifier.present = 1;
            modifier.raw = raw;
        }
        assert_eq!(
            efficiency_prefix(&plane, 40_000, 50_000, "", &loc(), &NoIcons).unwrap(),
            concat!(
                "Low 0.40\n",
                "Base: \u{11}H50%\u{11}!\n",
                "Crowd \u{11}Rx75%\u{11}!\n",
                "Weather \u{11}Rx80%\u{11}!\n",
                "Night \u{11}Rx65%\u{11}!\n",
                "    Night base \u{11}R-25%\u{11}!\n",
                "    Traffic: \u{11}G-10%\u{11}!\n",
                "Why."
            )
        );
        assert_eq!(
            carrier_org_suffix(35_000, 70_000, &loc(), &NoIcons).unwrap(),
            "Low 0.35 (Org \u{11}R70%\u{11}!)\n"
        );
        assert_eq!(
            screening_line(20_000, &loc(), &NoIcons).unwrap(),
            "  Screen 0.20\n"
        );
        assert_eq!(
            carrier_org_suffix(100_000, 70_000, &loc(), &NoIcons).unwrap(),
            ""
        );
    }

    #[test]
    fn linked_carrier_source_trees_keep_capture_order_for_both_sortie_keys() {
        for key in [0x24, 0x1b6] {
            let tree = |name: &[u8], scope| replay::rows::NavalModifierBreakdownRow {
                key,
                uses: vec![replay::rows::NavalModifierUse {
                    source_scope: scope,
                    ..Default::default()
                }],
                nodes: vec![
                    replay::rows::ModifierBreakdownNode {
                        children: 1,
                        ..Default::default()
                    },
                    replay::rows::ModifierBreakdownNode {
                        value: 20_000,
                        name: name.to_vec(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            };
            let first = tree(b"First", 0);
            let second = tree(b"Second", 1);
            let definition = ModifierDefinitionRow {
                index: key,
                precision: 2,
                ..Default::default()
            };
            let first_use = crate::combat::naval_summary_tooltips::test_tree_use(&first);
            let second_use = crate::combat::naval_summary_tooltips::test_tree_use(&second);
            assert_eq!(
                carrier_key_breakdown(
                    &NavalCombatShipRow::default(),
                    &[&first_use, &second_use],
                    Some(&definition),
                    key,
                    "english",
                    &loc(),
                    &NoIcons,
                )
                .unwrap(),
                concat!(
                    "  First: \u{11}R+0.20\u{11}!\n",
                    "  Second: \u{11}R+0.20\u{11}!\n",
                )
            );
            assert!(
                carrier_key_breakdown(
                    &NavalCombatShipRow::default(),
                    &[&first_use],
                    Some(&definition),
                    key + 1,
                    "english",
                    &loc(),
                    &NoIcons,
                )
                .is_err()
            );
        }
    }
}
