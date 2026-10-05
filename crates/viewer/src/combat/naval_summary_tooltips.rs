//! Dynamic hovers of the active naval combat summary (1417D4980).

use super::{
    binding::{Bindings, TextIcons, builtin},
    modifier_text::{Block, Definitions},
    naval_screening::{Census, Screening},
    tooltips::{hover_text, set_hover},
};
use crate::table_access::TokenNames;
use crate::{document::Document, map::tables::rows};
use installed_content::localisation::{Argument, Localisation};
use installed_content::unit_leader::{SubUnit, SubUnits};
use replay::{
    frame::DecodedFrame,
    rows::{NavalCombatSideDetails, tables},
};
use std::io;

/// 140C29020 -> 140C230A0 -> 14054D0F0: each ship modifier source has an
/// ordered cloned-child tree. The labels at CModifier+0x58 are stored text,
/// with no surviving source key (1405FFDE0), so another display language
/// cannot reconstruct them. 1415B4BB0 and 140C230A0 pass depth -1, so
/// 14054C770 prints every nonzero leaf regardless of hierarchy depth.
#[allow(clippy::too_many_arguments)] // The recorded tree, its formatting and text resources.
pub(super) fn ship_modifier_breakdown(
    nodes: &[replay::rows::ModifierBreakdownNode],
    definition: &replay::rows::ModifierDefinitionRow,
    scale: i64,
    prefix: &str,
    recorded_language: &str,
    icons: &dyn TextIcons,
    loc: &Localisation,
    show_names: bool,
) -> Result<String, String> {
    dashboard::modifier_text::breakdown(
        nodes,
        dashboard::modifier_text::Breakdown {
            depth: -1,
            prefix,
            suffix: "",
        },
        |node| {
            let mut text = if show_names {
                dashboard::scripted_text::stored_text(
                    loc,
                    recorded_language,
                    &node.name,
                    "naval ship modifier source",
                )?
            } else {
                String::new()
            };
            super::binding::validate(icons, &text)
                .map_err(|reason| format!("naval ship modifier source holds {reason}"))?;
            if !text.is_empty() {
                text.push_str(": ");
            }
            text.push_str(
                &super::modifier_text::value_text(
                    definition,
                    super::fixed::mul(node.value, scale),
                    2,
                    false,
                    false,
                    icons,
                    loc,
                )
                .map_err(|reason| reason.to_string())?,
            );
            Ok(text)
        },
    )
}

/// Table 270 shares each stable accumulator/key tree among its native source
/// occurrences. The join borrows nodes while restoring 1415B4BB0/140C230A0
/// source order; no tree is copied for another combat or ship.
#[derive(Clone, Copy)]
pub(super) struct NavalTreeUse<'a> {
    pub tree: &'a replay::rows::NavalModifierBreakdownRow,
    pub use_: &'a replay::rows::NavalModifierUse,
}

impl std::ops::Deref for NavalTreeUse<'_> {
    type Target = replay::rows::NavalModifierBreakdownRow;
    fn deref(&self) -> &Self::Target {
        self.tree
    }
}

pub(super) fn expanded_trees(
    rows: &[replay::rows::NavalModifierBreakdownRow],
) -> Vec<NavalTreeUse<'_>> {
    let mut expanded = Vec::new();
    for tree in rows {
        for use_ in &tree.uses {
            expanded.push(NavalTreeUse { tree, use_ });
        }
    }
    expanded.sort_unstable_by_key(|row| row.use_.order);
    expanded
}

#[cfg(test)]
pub(super) fn test_tree_use(tree: &replay::rows::NavalModifierBreakdownRow) -> NavalTreeUse<'_> {
    NavalTreeUse {
        tree,
        use_: &tree.uses[0],
    }
}

// `1417D4980` chooses these executable keys for paired stat text and
// background widgets. `1417D1A90` makes the delayed second string.
super::text::tooltip_keys! {
    COMBAT_STAT_LIGHT_GUNS_INFO = "combat_stat_light_guns_info",
    COMBAT_STAT_HEAVY_GUNS_INFO = "combat_stat_heavy_guns_info",
    COMBAT_STAT_TORPEDOES_INFO = "combat_stat_torpedoes_info",
    COMBAT_STAT_DEPTH_CHARGES_INFO = "combat_stat_depth_charges_info",
    SCREENING_EFFICIENCY_INFO = "screening_efficiency_info",
    SCREENING_TOOLTIP_SCREEN_BOX = "SCREENING_TOOLTIP_SCREEN_BOX",
    SCREENING_TOOLTIP_CAPITAL_BOX = "SCREENING_TOOLTIP_CAPITAL_BOX",
    SCREENING_TOOLTIP_CAPITAL_BOX_COMBINED = "SCREENING_TOOLTIP_CAPITAL_BOX_COMBINED",
    SCREENING_TOOLTIP_NO_SCREENS = "SCREENING_TOOLTIP_NO_SCREENS",
    SCREENING_TOOLTIP_FOR_SCREEN_BOX = "SCREENING_TOOLTIP_FOR_SCREEN_BOX",
    SCREENING_TOOLTIP_FOR_CAPITAL_BOX = "SCREENING_TOOLTIP_FOR_CAPITAL_BOX",
    SCREENING_TOOLTIP_BONUSES = "SCREENING_TOOLTIP_BONUSES",
    SCREENING_TOOLTIP_BONUSES_CAPITAL = "SCREENING_TOOLTIP_BONUSES_CAPITAL",
    SCREENING_TOOLTIP_LOW_INFO = "SCREENING_TOOLTIP_LOW_INFO",
    TOTAL_DAMAGE_DEALT = "total_damage_dealt",
    DAMAGE_DEALT_BY_BOX = "damage_dealt_by_box",
    DAMAGE_DEALT_BY_SHIP_TYPES = "damage_dealt_by_ship_types",
    DAMAGE_DEALT_BY_TYPE = "damage_dealt_by_type",
    DAMAGE_DEALT_LIGHT_GUNS = "damage_dealt_light_guns",
    DAMAGE_DEALT_HEAVY_GUNS = "damage_dealt_heavy_guns",
    DAMAGE_DEALT_TORPEDOES = "damage_dealt_torpedoes",
    DAMAGE_DEALT_DEPTH_CHARGES = "damage_dealt_depth_charges",
    DAMAGE_DONE_BY_WEAPON_TYPE = "damage_done_by_weapon_type",
    RELATIVE_DAMAGE_RATIO_TO_ENEMY_HEALTH = "relative_damage_ratio_to_enemy_health",
    DAMAGE_DEALT_SCREEN = "damage_dealt_screen",
    DAMAGE_DEALT_BATTLE_BOX = "damage_dealt_battle_box",
    DAMAGE_DEALT_CARRIERS = "damage_dealt_carriers",
    DAMAGE_DEALT_CONVOYS = "damage_dealt_convoys",
    DAMAGE_DEALT_SUBS = "damage_dealt_subs",
    DAMAGE_DEALT_AIR = "damage_dealt_air",
    NAVY_PIERCING_TOOLTIP_INFO = "NAVY_PIERCING_TOOLTIP_INFO",
    NAVAL_PIERCING_THRESHOLD_TO_DAMAGE = "NAVAL_PIERCING_THRESHOLD_TO_DAMAGE",
    NAVAL_PIERCING_THRESHOLD_TO_DAMAGE_LOWEST = "NAVAL_PIERCING_THRESHOLD_TO_DAMAGE_LOWEST",
    POSITIONING_INFO = "POSITIONING_INFO",
    MODIFIER_BASE_VALUE_PERCENTAGE = "MODIFIER_BASE_VALUE_PERCENTAGE",
    CONVOY_ESCORT_POSITIONING_BONUS = "CONVOY_ESCORT_POSITIONING_BONUS",
    POSITIONING_SURFACE_DETECTION_SUPERIORITY = "POSITIONING_SURFACE_DETECTION_SUPERIORITY",
    POSITIONING_SHIP_RATIO_PENALTY = "POSITIONING_SHIP_RATIO_PENALTY",
    POSITIONING_CARRIER_RATIO_PENALTY = "POSITIONING_CARRIER_RATIO_PENALTY",
    POSITIONING_RECENTLY_JOINED_SHIPS = "POSITIONING_RECENTLY_JOINED_SHIPS",
    POSITIONING_PENALTY_INFO = "POSITIONING_PENALTY_INFO",
    BM_ATTACK_VALUE = "BM_ATTACK_VALUE",
    SCREENING_VALUE = "SCREENING_VALUE",
    AA_MODIFIER = "AA_MODIFIER",
    STAT_NAVY_SUB_VISIBILITY = "STAT_NAVY_SUB_VISIBILITY",
    STAT_NAVY_SUB_VISIBILITY_MOD_DIFF = "STAT_NAVY_SUB_VISIBILITY_MOD_DIFF",
}
const WEAPON_HOVERS: [(&str, &str); 4] = [
    ("stat_light_gun", COMBAT_STAT_LIGHT_GUNS_INFO),
    ("stat_heavy_gun", COMBAT_STAT_HEAVY_GUNS_INFO),
    ("stat_torpedo", COMBAT_STAT_TORPEDOES_INFO),
    ("stat_depth_charges", COMBAT_STAT_DEPTH_CHARGES_INFO),
];
// `1417D1A90` walks the six pairs at 1429ED7E0 in this order.
const DAMAGE_CATEGORIES: [&str; 6] = [
    DAMAGE_DEALT_SCREEN,
    DAMAGE_DEALT_BATTLE_BOX,
    DAMAGE_DEALT_CARRIERS,
    DAMAGE_DEALT_CONVOYS,
    DAMAGE_DEALT_SUBS,
    DAMAGE_DEALT_AIR,
];
// 1417D0EC0 walks the four pairs at 1429EDC00 before the box categories.
const DAMAGE_WEAPONS: [&str; 4] = [
    DAMAGE_DEALT_LIGHT_GUNS,
    DAMAGE_DEALT_HEAVY_GUNS,
    DAMAGE_DEALT_TORPEDOES,
    DAMAGE_DEALT_DEPTH_CHARGES,
];

/// 1417D1A90 saturates a failed or overflowing fixed5 ratio to 0xffffffff.
fn damage_ratio(value: i64, denominator: i64) -> i64 {
    super::fixed::fraction(value, denominator).unwrap_or(super::fixed::EXCEPTIONAL)
}

/// 14160A7B0 renders two screening descriptions from the fresh census. Its
/// static bonus blocks are 140600CF0's named content modifiers; only the
/// rare 0x1B3 country correction needs a live source tree.
#[expect(
    clippy::too_many_arguments,
    reason = "native screening text has separate combat and content operands"
)]
fn screening_description(
    carrier: bool,
    census: &Census,
    screening: &Screening,
    defines: &installed_content::naval_combat_defines::Defines,
    correction: &str,
    static_modifier: Result<&installed_content::modifiers::Modifiers, String>,
    definitions: Result<&Definitions<'_>, String>,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<(String, Vec<String>), String> {
    let combined = super::fixed::ONE.wrapping_sub(super::fixed::mul(
        super::fixed::ONE.wrapping_sub(screening.capital),
        super::fixed::ONE.wrapping_sub(screening.carrier),
    ));
    let (value, bonus_scale) = if carrier {
        (screening.carrier, combined)
    } else {
        (screening.capital, screening.capital)
    };
    let mut text = if carrier {
        String::new()
    } else {
        correction.to_owned()
    };
    text.push_str(&builtin(
        icons,
        loc,
        if carrier {
            SCREENING_TOOLTIP_CAPITAL_BOX
        } else {
            SCREENING_TOOLTIP_SCREEN_BOX
        },
        &[("NUM", Argument::Fixed5(value))],
    )?);
    if carrier {
        text.push('\n');
        text.push_str(&builtin(
            icons,
            loc,
            SCREENING_TOOLTIP_CAPITAL_BOX_COMBINED,
            &[("NUM", Argument::Fixed5(combined))],
        )?);
    }
    text.push_str("\n\n");
    if (!carrier && census.screens == 0) || (carrier && combined == 0) {
        text.push_str(&builtin(icons, loc, SCREENING_TOOLTIP_NO_SCREENS, &[])?);
        return Ok((text, Vec::new()));
    }
    if carrier {
        let need = super::naval_screening::ceil_fixed(
            census
                .convoys
                .wrapping_mul(defines.summary_fixed[11])
                .wrapping_add(census.carriers.wrapping_mul(defines.summary_fixed[10])),
        );
        text.push_str(&builtin(
            icons,
            loc,
            SCREENING_TOOLTIP_FOR_CAPITAL_BOX,
            &[
                ("NUM_CARR", Argument::Int32(census.carriers as i32)),
                ("NUM_CONV", Argument::Int32(census.convoys as i32)),
                ("NUM_CAPI", Argument::Int32(census.capitals as i32)),
                ("NUM_NEED", Argument::Int32(need as i32)),
            ],
        )?);
    } else {
        let need = super::naval_screening::ceil_fixed(
            census
                .convoys
                .wrapping_mul(defines.summary_fixed[9])
                .wrapping_add(
                    census
                        .capitals
                        .wrapping_add(census.carriers)
                        .wrapping_mul(defines.summary_fixed[8]),
                ),
        );
        text.push_str(&builtin(
            icons,
            loc,
            SCREENING_TOOLTIP_FOR_SCREEN_BOX,
            &[
                ("NUM_CAPI", Argument::Int32(census.capitals as i32)),
                ("NUM_CARR", Argument::Int32(census.carriers as i32)),
                ("NUM_CONV", Argument::Int32(census.convoys as i32)),
                ("NUM_SCRE", Argument::Int32(census.screens as i32)),
                ("NUM_NEED", Argument::Int32(need as i32)),
            ],
        )?);
    }
    text.push('\n');
    let static_modifier = static_modifier?;
    let definitions = definitions?;
    let (block, dropped) =
        Block::build(definitions, static_modifier).map_err(|reason| reason.to_string())?;
    if bonus_scale > 0
        && block
            .kept(definitions)
            .map_err(|reason| reason.to_string())?
    {
        super::text::ensure_newlines(&mut text, 2);
        text.push_str(&builtin(
            icons,
            loc,
            if carrier {
                SCREENING_TOOLTIP_BONUSES_CAPITAL
            } else {
                SCREENING_TOOLTIP_BONUSES
            },
            &[],
        )?);
        text.push_str(
            &block
                .describe(definitions, icons, loc, bonus_scale, "", u32::MAX)
                .map_err(|reason| reason.to_string())?,
        );
    }
    if bonus_scale < super::fixed::ONE {
        super::text::ensure_newlines(&mut text, 2);
        text.push_str(&builtin(icons, loc, SCREENING_TOOLTIP_LOW_INFO, &[])?);
    }
    Ok((text, dropped))
}

/// 141609460 writes the fresh positioning description in this order. Its
/// source text is 1415B4BB0's ordered country/weather/terrain/commander
/// breakdown for keys 0x1B2 and, for an early defender, 0x5A. 1417D4980
/// then prepends the immediate heading and one newline.
fn positioning_description(
    terms: super::naval_positioning::Terms,
    sources: &str,
    defines: &installed_content::naval_combat_defines::Defines,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<String, String> {
    let mut text = builtin(icons, loc, POSITIONING_INFO, &[])?;
    text.push('\n');
    if terms.base != 0 {
        text.push_str(&builtin(
            icons,
            loc,
            MODIFIER_BASE_VALUE_PERCENTAGE,
            &[("VALUE", Argument::Fixed5(terms.base))],
        )?);
        text.push('\n');
    }
    text.push_str(sources);
    if terms.escort > 1_000 {
        text.push_str(&builtin(
            icons,
            loc,
            CONVOY_ESCORT_POSITIONING_BONUS,
            &[("VALUE", Argument::Fixed5(terms.escort))],
        )?);
        if !text.ends_with('\n') {
            text.push('\n');
        }
    }
    for (amount, key) in [
        (terms.detection, POSITIONING_SURFACE_DETECTION_SUPERIORITY),
        (
            terms.ship_ratio.wrapping_neg(),
            POSITIONING_SHIP_RATIO_PENALTY,
        ),
        (
            terms.carrier_ratio.wrapping_neg(),
            POSITIONING_CARRIER_RATIO_PENALTY,
        ),
        (
            terms.recent_join.wrapping_neg(),
            POSITIONING_RECENTLY_JOINED_SHIPS,
        ),
    ] {
        if amount == 0 {
            continue;
        }
        text.push_str(&builtin(
            icons,
            loc,
            key,
            &[("NUM", Argument::Fixed5(amount))],
        )?);
        text.push('\n');
    }
    if terms.value < super::fixed::ONE {
        text.push('\n');
        text.push_str(&builtin(icons, loc, POSITIONING_PENALTY_INFO, &[])?);
        text.push('\n');
        for (impact, key, argument) in [
            (defines.positioning_impacts[0], BM_ATTACK_VALUE, "VALUE"),
            (defines.summary_fixed[7], SCREENING_VALUE, "VALUE"),
            (defines.positioning_impacts[1], AA_MODIFIER, "VALUE"),
        ] {
            let penalty = impact.wrapping_sub(super::fixed::mul(terms.value, impact));
            if penalty > 0 {
                text.push_str(&builtin(
                    icons,
                    loc,
                    key,
                    &[(argument, Argument::Fixed5(penalty.wrapping_neg()))],
                )?);
                text.push('\n');
            }
        }
        let reveal = defines.positioning_impacts[2].wrapping_sub(super::fixed::mul(
            terms.value,
            defines.positioning_impacts[2],
        ));
        if reveal > 0 {
            text.push_str(&builtin(icons, loc, STAT_NAVY_SUB_VISIBILITY, &[])?);
            text.push_str(&builtin(
                icons,
                loc,
                STAT_NAVY_SUB_VISIBILITY_MOD_DIFF,
                &[("VALUE", Argument::Fixed5(reveal))],
            )?);
            text.push('\n');
        }
    }
    Ok(text)
}

#[allow(clippy::too_many_arguments)] // The side's recorded trees and text resources.
fn positioning_sources(
    id: u64,
    side: u8,
    early_defender: bool,
    trees: &[NavalTreeUse<'_>],
    definitions: &[replay::rows::ModifierDefinitionRow],
    recorded_language: &str,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<String, String> {
    let mut text = String::new();
    for key in [Some(0x1b2), early_defender.then_some(0x5a)]
        .into_iter()
        .flatten()
    {
        let definition = definitions
            .iter()
            .find(|row| row.index == key)
            .ok_or_else(|| format!("positioning modifier definition {key:#x} is absent"))?;
        let sources: Vec<_> = trees
            .iter()
            .filter(|row| {
                row.use_.combat == id
                    && row.use_.side == side
                    && row.use_.battle_slot == u32::MAX
                    && row.key == key
            })
            .collect();
        if !sources.iter().any(|row| row.use_.source_scope == 0) {
            return Err(format!(
                "positioning key {key:#x} has no captured country source"
            ));
        }
        for source in sources {
            text.push_str(&ship_modifier_breakdown(
                &source.nodes,
                definition,
                super::fixed::ONE,
                "",
                recorded_language,
                icons,
                loc,
                true,
            )?);
            // 1415B4BB0 does not add a newline after the country source, but
            // ensures one after weather, terrain and commander, even empty.
            if source.use_.source_scope != 0 && !text.ends_with('\n') {
                text.push('\n');
            }
        }
    }
    Ok(text)
}

/// 1417D0EC0: the flag's delayed description starts with the stored total,
/// then weapon and target-category sums, and finally the sorted +0x288 map.
/// A negative opposing baseline returns an empty string before that walk.
pub(super) fn flag_description(
    side: &NavalCombatSideDetails,
    opponent: &NavalCombatSideDetails,
    tokens: &TokenNames<'_>,
    sub_units: &SubUnits,
    viewing_tag: Result<&str, &str>,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<String, String> {
    if opponent.original_health < 0 {
        return Ok(String::new());
    }
    let total = side.total_damage;
    let mut result = builtin(
        icons,
        loc,
        TOTAL_DAMAGE_DEALT,
        &[
            ("NUM", Argument::Fixed5(total)),
            (
                "RATIO",
                Argument::Fixed5(damage_ratio(total, opponent.original_health)),
            ),
        ],
    )?;
    if total <= 0 {
        return Ok(result);
    }
    for (weapon, key) in DAMAGE_WEAPONS.into_iter().enumerate() {
        let damage = side
            .damage_history
            .iter()
            .fold(0_i64, |sum, category| sum.wrapping_add(category[weapon]));
        if damage > 0 {
            result.push_str(&builtin(
                icons,
                loc,
                key,
                &[
                    ("NUM", Argument::Fixed5(damage)),
                    ("RATIO", Argument::Fixed5(damage_ratio(damage, total))),
                ],
            )?);
        }
    }
    result.push('\n');
    result.push_str(&builtin(icons, loc, DAMAGE_DEALT_BY_BOX, &[])?);
    for (category, key) in side.damage_history.iter().zip(DAMAGE_CATEGORIES) {
        let damage = category
            .iter()
            .fold(0_i64, |sum, cell| sum.wrapping_add(*cell));
        if damage > 0 {
            result.push_str(&builtin(
                icons,
                loc,
                key,
                &[
                    ("NUM", Argument::Fixed5(damage)),
                    ("RATIO", Argument::Fixed5(damage_ratio(damage, total))),
                ],
            )?);
        }
    }
    result.push('\n');
    result.push_str(&builtin(icons, loc, DAMAGE_DEALT_BY_SHIP_TYPES, &[])?);
    // sorted list from lowest to highest, retaining map order for ties.
    let mut by_type: Vec<_> = side.damage_by_type.iter().collect();
    by_type.sort_by_key(|source| source.damage);
    for source in by_type {
        let key = tokens
            .name(source.token)
            .map_err(|error| error.to_string())?;
        let index = match sub_units.get(key) {
            SubUnit::Index(index) => index,
            SubUnit::Unknown => {
                return Err(format!(
                    "ship class {key} is absent from the content database"
                ));
            }
            SubUnit::Uncertain(reason) => {
                return Err(format!("ship class {key} is uncertain: {reason}"));
            }
        };
        let name = sub_units.name(index).ok_or_else(|| {
            format!("ship class index {index} is absent from the content database")
        })??;
        let name = super::text::country_localised(loc, viewing_tag, name)?;
        super::binding::validate(icons, &name)?;
        result.push_str(&builtin(
            icons,
            loc,
            DAMAGE_DEALT_BY_TYPE,
            &[
                ("TYPE", Argument::Text(&name)),
                ("NUM", Argument::Fixed5(source.damage)),
                (
                    "RATIO",
                    Argument::Fixed5(damage_ratio(source.damage, total)),
                ),
            ],
        )?);
    }
    result.push('\n');
    Ok(result)
}

/// The delayed weapon body: native 14160C600 sums the weapon's six category
/// cells with signed wrapping addition, then 1417D1A90 visits each positive
/// category cell and optionally adds the light/heavy piercing explanation.
fn weapon_description(
    side: &NavalCombatSideDetails,
    opponent: &NavalCombatSideDetails,
    weapon: usize,
    piercing: Option<&installed_content::naval_combat_defines::Defines>,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<String, String> {
    let total = side
        .damage_history
        .iter()
        .fold(0_i64, |sum, category| sum.wrapping_add(category[weapon]));
    let mut text = builtin(
        icons,
        loc,
        DAMAGE_DONE_BY_WEAPON_TYPE,
        &[("NUM", Argument::Fixed5(total))],
    )?;
    if total > 0 {
        text.push('\n');
        text.push_str(&builtin(
            icons,
            loc,
            RELATIVE_DAMAGE_RATIO_TO_ENEMY_HEALTH,
            &[(
                "NUM",
                Argument::Fixed5(damage_ratio(total, opponent.original_health)),
            )],
        )?);
        for (category, key) in side.damage_history.iter().zip(DAMAGE_CATEGORIES) {
            let damage = category[weapon];
            if damage <= 0 {
                continue;
            }
            text.push('\n');
            text.push_str(&builtin(
                icons,
                loc,
                key,
                &[
                    ("NUM", Argument::Fixed5(damage)),
                    ("RATIO", Argument::Fixed5(damage_ratio(damage, total))),
                ],
            )?);
        }
    }
    // 1417D4980 passes true only for light/heavy guns (asm 1417D4F0C,
    // 1417D4FD1). The arrays are NNavy content loaded at 143320A28/A40.
    if weapon <= 1 {
        let defines = piercing.ok_or("the matching NNavy piercing defines are unavailable")?;
        if defines.piercing_thresholds.len() < 2
            || defines.piercing_thresholds.len() != defines.piercing_critical_values.len()
        {
            return Err(
                "NNavy piercing threshold arrays differ or have fewer than two entries".into(),
            );
        }
        super::text::ensure_newlines(&mut text, 2);
        text.push_str(&builtin(icons, loc, NAVY_PIERCING_TOOLTIP_INFO, &[])?);
        for (index, &critical) in defines.piercing_critical_values.iter().enumerate() {
            if !text.ends_with('\n') {
                text.push('\n');
            }
            let last = index + 1 == defines.piercing_critical_values.len();
            let threshold = defines.piercing_thresholds[index - usize::from(last)];
            text.push_str(&builtin(
                icons,
                loc,
                if last {
                    NAVAL_PIERCING_THRESHOLD_TO_DAMAGE_LOWEST
                } else {
                    NAVAL_PIERCING_THRESHOLD_TO_DAMAGE
                },
                &[
                    ("THRESHOLD", Argument::Fixed5(threshold)),
                    ("DAMAGE", Argument::Fixed5(critical.wrapping_add(100_000))),
                ],
            )?);
        }
    }
    Ok(text)
}

fn header(
    out: &mut Bindings,
    icons: &dyn TextIcons,
    loc: &Localisation,
    id: u64,
    side: &str,
    keys: &[String],
    name: &str,
) {
    let text = match builtin(icons, loc, name, &[]) {
        Ok(text) => hover_text(&text),
        Err(reason) => {
            out.refuse(format!(
                "Naval combat {id} {side} {name} hover is unavailable: {reason}"
            ));
            None
        }
    };
    for key in keys {
        set_hover(&mut out.values, key.clone(), text.clone(), None);
    }
}

#[allow(clippy::too_many_arguments)] // Both sides' recorded details and text resources.
fn side_hovers(
    out: &mut Bindings,
    icons: &dyn TextIcons,
    loc: &Localisation,
    id: u64,
    side: &str,
    value: &NavalCombatSideDetails,
    opponent: &NavalCombatSideDetails,
    defines: Option<&installed_content::naval_combat_defines::Defines>,
) {
    let path = format!("header_container/{side}_wnd");
    for (index, (widget, key)) in WEAPON_HOVERS.into_iter().enumerate() {
        let pair = [format!("{path}/{widget}"), format!("{path}/{widget}_bg")];
        header(out, icons, loc, id, side, &pair, key);
        // Widget order light, heavy, torpedo, depth maps to native weapon
        // indices 0, 1, 2, 3 in 1417D4980.
        let description = weapon_description(value, opponent, index, defines, icons, loc);
        for key in pair {
            out.values.entry(key).or_default().tooltip_description =
                description.as_ref().ok().and_then(|text| hover_text(text));
        }
        if let Err(reason) = description {
            out.refuse(format!(
                "Naval combat {id} {side} {widget} damage breakdown is unavailable: {reason}"
            ));
        }
    }
    // `1417D4980` emits this header for each screening icon; `14160A7B0`
    // builds its delayed explanation below from the current category census.
    for widget in [
        "capital_screening_efficiency",
        "carrier_screening_efficiency",
    ] {
        header(
            out,
            icons,
            loc,
            id,
            side,
            &[format!("{side}_wnd_{widget}")],
            SCREENING_EFFICIENCY_INFO,
        );
    }
}

/// Called after authored GUI hovers, so native dynamic stat callbacks own
/// their widgets even when a historical operand is absent.
pub(super) fn bind(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    id: u64,
) -> io::Result<()> {
    let details = rows::<tables::NavalCombatDetails>(frame)?;
    let Some(detail) = details.rows().iter().find(|row| row.combat == id) else {
        return Ok(());
    };
    let icons = &document.icons.combat;
    let loc = &document.names.localisation;
    let ships = rows::<tables::NavalCombatShips>(frame)?;
    let models = rows::<tables::ShipModels>(frame)?;
    let bases = rows::<tables::AirBases>(frame)?;
    let live_ships = rows::<tables::Ships>(frame)?;
    let dense_rows = rows::<tables::NavalCombatAirBaseWingHandles>(frame)?;
    let wings = rows::<tables::AirWings>(frame)?;
    let trees = rows::<tables::NavalModifierBreakdowns>(frame)?;
    let breakdown_layout =
        replay::fixed_layout::validate_recording(frame, replay::fixed_layout::NAVAL_BREAKDOWNS);
    let expanded = trees.is_complete().then(|| expanded_trees(trees.rows()));
    let raw_definitions = rows::<tables::ModifierDefinitions>(frame)?;
    let definitions = Definitions::read(frame, &document.modifier_names)?;
    let keyed_models = models
        .is_complete()
        .then(|| super::keyed::Keyed::new(models.rows(), |model| model.ship))
        .flatten();
    let dense = (dense_rows.is_complete() && wings.is_complete())
        .then(|| super::naval_air_groups::DenseBases::new(dense_rows.rows(), wings.rows()))
        .flatten();
    let position_terms =
        if replay::fixed_layout::validate_recording(frame, replay::fixed_layout::NAVAL_POSITIONING)
            .is_ok()
        {
            super::naval_positioning::positioning_terms_values(
                detail,
                ships.is_complete().then_some(ships.rows()),
                keyed_models,
                bases.is_complete().then_some(bases.rows()),
                live_ships.is_complete().then_some(live_ships.rows()),
                dense,
                &icons.naval_air_groups,
                icons.naval_defines.as_ref(),
            )
        } else {
            [None; 2]
        };
    for (side, name) in ["left", "right"].into_iter().enumerate() {
        // `1417D4980` returns false before selecting a stat when both
        // CCombatant primary country references are empty.
        if !detail.sides[side].summary.countries.is_empty() {
            side_hovers(
                out,
                icons,
                loc,
                id,
                name,
                &detail.sides[side],
                &detail.sides[1 - side],
                document.icons.combat.naval_defines.as_ref(),
            );
            let position_keys = [
                format!("header_container/{name}_wnd/stat_positioning"),
                format!("header_container/{name}_wnd/stat_positioning_bg"),
            ];
            let positioning = (|| {
                let terms = position_terms[side]
                    .ok_or("fresh positioning operands or fixed modifier layout are unavailable")?;
                let defines = icons
                    .naval_defines
                    .as_ref()
                    .ok_or("matching naval combat defines are unavailable")?;
                breakdown_layout.as_ref().map_err(Clone::clone)?;
                if !trees.is_complete() || !raw_definitions.is_complete() {
                    return Err(
                        "NavalModifierBreakdowns or ModifierDefinitions is incomplete".to_owned(),
                    );
                }
                let source_text = positioning_sources(
                    id,
                    side as u8,
                    terms.early_defender,
                    expanded.as_deref().unwrap_or(&[]),
                    raw_definitions.rows(),
                    &document.recorded_language,
                    icons,
                    loc,
                )?;
                positioning_description(terms, &source_text, defines, icons, loc)
            })();
            for key in position_keys {
                let hover = positioning.as_ref().ok().and_then(|text| hover_text(text));
                set_hover(&mut out.values, key, hover, None);
            }
            if let Err(reason) = positioning {
                out.refuse(format!(
                    "Naval combat {id} {name} positioning hover is unavailable: {reason}"
                ));
            }
            let screening_inputs = (|| {
                let defines = icons
                    .naval_defines
                    .as_ref()
                    .ok_or("matching naval combat defines are unavailable")?;
                if !ships.is_complete() {
                    return Err("NavalCombatShips is incomplete".to_owned());
                }
                let models = keyed_models.ok_or("ShipModels is incomplete or unordered")?;
                let census = super::naval_screening::census(id, side as u8, ships.rows(), &models)
                    .map_err(str::to_owned)?;
                let screening = super::naval_screening::calculate(
                    id,
                    side as u8,
                    &detail.sides[side],
                    ships.rows(),
                    &models,
                    defines,
                )
                .map_err(str::to_owned)?;
                Ok::<_, String>((census, screening, defines))
            })();
            for (carrier, widget) in [
                (false, "capital_screening_efficiency"),
                (true, "carrier_screening_efficiency"),
            ] {
                let delayed = (|| {
                    let (census, screening, defines) =
                        screening_inputs.as_ref().map_err(Clone::clone)?;
                    // 14160AD8B calls 0x1B3 when the uncorrected result is
                    // zero; its source description is still appended when
                    // the numerical country bonus is not positive.
                    let correction_branch = screening.country_bonus_applied
                        || (screening.capital == 0
                            && !detail.sides[side].summary.countries.is_empty()
                            && census.eligible_submarines == 0);
                    let correction = if !carrier && correction_branch {
                        breakdown_layout.as_ref().map_err(Clone::clone)?;
                        if !trees.is_complete() || !raw_definitions.is_complete() {
                            return Err("NavalModifierBreakdowns or ModifierDefinitions is incomplete for the country screening correction".to_owned());
                        }
                        let source = expanded
                            .as_deref()
                            .unwrap_or(&[])
                            .iter()
                            .find(|row| {
                                row.use_.combat == id
                                    && row.use_.side == side as u8
                                    && row.use_.battle_slot == u32::MAX
                                    && row.use_.source_scope == 0
                                    && row.key == 0x1b3
                            })
                            .ok_or("the country screening correction has no captured 0x1B3 tree")?;
                        let definition = raw_definitions
                            .rows()
                            .iter()
                            .find(|row| row.index == 0x1b3)
                            .ok_or("the 0x1B3 modifier definition is absent")?;
                        ship_modifier_breakdown(
                            &source.nodes,
                            definition,
                            super::fixed::ONE,
                            "",
                            &document.recorded_language,
                            icons,
                            loc,
                            true,
                        )?
                    } else {
                        String::new()
                    };
                    screening_description(
                        carrier,
                        census,
                        screening,
                        defines,
                        &correction,
                        icons.naval_screening_modifiers[usize::from(carrier)]
                            .as_ref()
                            .map_err(Clone::clone),
                        definitions.as_ref().map_err(ToString::to_string),
                        icons,
                        loc,
                    )
                })();
                let key = format!("{name}_wnd_{widget}");
                match delayed {
                    Ok((text, dropped)) => {
                        out.values.entry(key).or_default().tooltip_description = hover_text(&text);
                        for dropped in dropped {
                            out.refuse(format!("Naval combat {id} {name} {widget}: static screening modifier {dropped} has no registered definition and is dropped as 1406042A0 drops it."));
                        }
                    }
                    Err(reason) => {
                        out.values.entry(key).or_default().tooltip_description = None;
                        out.refuse(format!(
                            "Naval combat {id} {name} {widget} breakdown is unavailable: {reason}"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
