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
    let mut text = String::new();
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
/// ordered country/weather/terrain/commander child lines are pending
/// capture-on-change. 1417D4980 prepends the heading and one newline.
fn positioning_description(
    terms: super::naval_positioning::Terms,
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

fn positioning_hover(
    out: &mut Bindings,
    id: u64,
    side: &str,
    terms: super::naval_positioning::Terms,
    defines: &installed_content::naval_combat_defines::Defines,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<String, String> {
    let text = positioning_description(terms, defines, icons, loc)?;
    // 141609460 -> 1415B4BB0: numerical positioning terms are recorded;
    // ordered modifier children await native writer notifications.
    out.refuse(format!("Naval combat {id} {side} positioning modifier breakdown is pending capture-on-change (141609460, key 0x1B2, 1415B4BB0, 14054D0F0)."));
    if terms.early_defender {
        out.refuse(format!("Naval combat {id} {side} early-defender positioning modifier breakdown is pending capture-on-change (141609460, key 0x5A)."));
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
                positioning_hover(out, id, name, terms, defines, icons, loc)
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
                    if !carrier && correction_branch {
                        out.refuse(format!("Naval combat {id} {name} capital screening country modifier breakdown is pending capture-on-change (14160AD8B, key 0x1B3, 14054D0F0)."));
                    }
                    screening_description(
                        carrier,
                        census,
                        screening,
                        defines,
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
mod tests {
    use super::*;
    use crate::combat::binding::NoIcons;
    use replay::rows::{LexicalTokenRow, NavalDamageSource};

    #[test]
    fn flag_damage_summary_uses_native_sections_and_sorted_source_types() {
        let loc: Localisation = [
            ("total_damage_dealt", "T $NUM$ $RATIO$;"),
            ("damage_dealt_light_guns", "L $NUM$ $RATIO$;"),
            ("damage_dealt_torpedoes", "Torp $NUM$ $RATIO$;"),
            ("damage_dealt_by_box", "BOX|"),
            ("damage_dealt_screen", "S $NUM$ $RATIO$;"),
            ("damage_dealt_convoys", "C $NUM$ $RATIO$;"),
            ("damage_dealt_by_ship_types", "CLASS|"),
            ("damage_dealt_by_type", "$TYPE$ $NUM$ $RATIO$;"),
            ("AAA_destroyer", "Escorts"),
            ("AAA_carrier", "Carriers"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        let mut side = NavalCombatSideDetails {
            total_damage: 400_000,
            damage_by_type: vec![
                NavalDamageSource {
                    token: 2,
                    damage: 300_000,
                },
                NavalDamageSource {
                    token: 1,
                    damage: 100_000,
                },
            ],
            ..Default::default()
        };
        side.damage_history[0][0] = 100_000;
        side.damage_history[3][2] = 200_000;
        let opponent = NavalCombatSideDetails {
            original_health: 800_000,
            ..Default::default()
        };
        let rows = [
            LexicalTokenRow {
                token: 1,
                literal: b"destroyer".to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: b"carrier".to_vec(),
            },
        ];
        let tokens = TokenNames::from_rows(&rows, true);
        let mut sub_units = SubUnits::default();
        sub_units.read(
            b"sub_units = { destroyer = {} carrier = {} }",
            "common/units/test.txt",
        );
        assert_eq!(
            flag_description(
                &side,
                &opponent,
                &tokens,
                &sub_units,
                Ok("AAA"),
                &NoIcons,
                &loc
            )
            .unwrap(),
            "T 4.00 0.50;L 1.00 0.25;Torp 2.00 0.50;\nBOX|S 1.00 0.25;C 2.00 0.50;\nCLASS|Escorts 1.00 0.25;Carriers 3.00 0.75;\n"
        );
        assert_eq!(
            flag_description(
                &side,
                &NavalCombatSideDetails {
                    original_health: -1,
                    ..Default::default()
                },
                &tokens,
                &sub_units,
                Ok("AAA"),
                &NoIcons,
                &loc
            )
            .unwrap(),
            ""
        );
    }

    #[test]
    fn weapon_damage_summary_includes_piercing_in_native_order() {
        let loc: Localisation = [
            ("damage_done_by_weapon_type", "Total $NUM$"),
            ("relative_damage_ratio_to_enemy_health", "Enemy $NUM$"),
            ("damage_dealt_screen", "Screen $NUM$ $RATIO$"),
            ("NAVY_PIERCING_TOOLTIP_INFO", "Info"),
            (
                "NAVAL_PIERCING_THRESHOLD_TO_DAMAGE",
                "T$THRESHOLD$ D$DAMAGE$;",
            ),
            (
                "NAVAL_PIERCING_THRESHOLD_TO_DAMAGE_LOWEST",
                "L$THRESHOLD$ D$DAMAGE$;",
            ),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        let mut side = NavalCombatSideDetails::default();
        side.damage_history[0][0] = 100_000;
        let opponent = NavalCombatSideDetails {
            original_health: 400_000,
            ..Default::default()
        };
        let defines = installed_content::naval_combat_defines::Defines {
            summary_fixed: [0; 14],
            summary_integer: [0; 3],
            silhouette_planes: [0; 6],
            screen_cap_reduction_factor: 0,
            plane_stacking_penalty_effect: 0,
            sortie_stance_ratios: vec![100_000],
            base_carrier_sortie_efficiency: 0,
            selected_sortie_initial_time: 0,
            piercing_thresholds: vec![40_000, 60_000],
            piercing_critical_values: vec![10_000, 20_000],
            positioning_impacts: [0; 3],
        };
        assert_eq!(
            weapon_description(&side, &opponent, 0, Some(&defines), &NoIcons, &loc).unwrap(),
            "Total 1.00\nEnemy 0.25\nScreen 1.00 1.00\n\nInfo\nT0.40 D1.10;\nL0.40 D1.20;"
        );
        let mut invalid = defines.clone();
        invalid.piercing_critical_values.truncate(1);
        assert_eq!(
            weapon_description(&side, &opponent, 0, Some(&invalid), &NoIcons, &loc),
            Err("NNavy piercing threshold arrays differ or have fewer than two entries".into())
        );
    }

    #[test]
    fn positioning_hover_has_native_heading_terms_and_low_positioning_consequences() {
        let loc: Localisation = [
            ("POSITIONING_INFO", "Positioning"),
            ("MODIFIER_BASE_VALUE_PERCENTAGE", "Base $VALUE$"),
            ("CONVOY_ESCORT_POSITIONING_BONUS", "Escort $VALUE$"),
            (
                "POSITIONING_SURFACE_DETECTION_SUPERIORITY",
                "Detection $NUM$",
            ),
            ("POSITIONING_SHIP_RATIO_PENALTY", "Ship $NUM$"),
            ("POSITIONING_CARRIER_RATIO_PENALTY", "Carrier $NUM$"),
            ("POSITIONING_RECENTLY_JOINED_SHIPS", "Recent $NUM$"),
            ("POSITIONING_PENALTY_INFO", "Penalty"),
            ("BM_ATTACK_VALUE", "Attack $VALUE$"),
            ("SCREENING_VALUE", "Screen $VALUE$"),
            ("AA_MODIFIER", "AA $VALUE$"),
            ("STAT_NAVY_SUB_VISIBILITY", "Sub "),
            ("STAT_NAVY_SUB_VISIBILITY_MOD_DIFF", "$VALUE$"),
        ]
        .into_iter()
        .map(|(key, text)| (key.to_owned(), text.to_owned()))
        .collect();
        let terms = super::super::naval_positioning::Terms {
            value: 50_000,
            base: 40_000,
            escort: 10_000,
            detection: 20_000,
            ship_ratio: 10_000,
            carrier_ratio: 5_000,
            recent_join: 5_000,
            early_defender: true,
        };
        let defines = installed_content::naval_combat_defines::Defines {
            summary_fixed: {
                let mut values = [0; 14];
                values[7] = 20_000;
                values
            },
            summary_integer: [0; 3],
            silhouette_planes: [0; 6],
            screen_cap_reduction_factor: 0,
            plane_stacking_penalty_effect: 0,
            sortie_stance_ratios: vec![],
            base_carrier_sortie_efficiency: 0,
            selected_sortie_initial_time: 0,
            piercing_thresholds: vec![],
            piercing_critical_values: vec![],
            positioning_impacts: [40_000, 60_000, 30_000],
        };
        let mut out = Bindings::default();
        let text =
            positioning_hover(&mut out, 42, "right", terms, &defines, &NoIcons, &loc).unwrap();
        assert_eq!(
            text,
            "Positioning\nBase 0.40\nEscort 0.10\nDetection 0.20\nShip -0.10\nCarrier -0.05\nRecent -0.05\n\nPenalty\nAttack -0.20\nScreen -0.10\nAA -0.30\nSub 0.15\n"
        );
        assert_eq!(out.refused().len(), 2);
        assert!(
            out.refused()
                .iter()
                .all(|(reason, &count)| reason.contains("pending capture-on-change") && count == 1)
        );
        let mut later = terms;
        later.early_defender = false;
        assert_eq!(
            positioning_hover(&mut out, 42, "right", later, &defines, &NoIcons, &loc).unwrap(),
            text
        );
        assert_eq!(out.refused().values().copied().sum::<usize>(), 3);
        assert_eq!(
            out.refused()
                .iter()
                .find(|(reason, _)| reason.contains("key 0x5A"))
                .unwrap()
                .1,
            &1
        );
    }

    #[test]
    fn screen_hover_keeps_supported_lines_without_a_country_tree() {
        let loc: Localisation = [
            ("SCREENING_TOOLTIP_SCREEN_BOX", "Screens $NUM$"),
            ("SCREENING_TOOLTIP_NO_SCREENS", "No screens"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        let census = Census::default();
        let screening = Screening {
            capital: 0,
            carrier: 0,
            country_bonus_applied: false,
        };
        let defines = installed_content::naval_combat_defines::Defines {
            summary_fixed: [0; 14],
            summary_integer: [0; 3],
            silhouette_planes: [0; 6],
            screen_cap_reduction_factor: 0,
            plane_stacking_penalty_effect: 0,
            sortie_stance_ratios: vec![],
            base_carrier_sortie_efficiency: 0,
            selected_sortie_initial_time: 0,
            piercing_thresholds: vec![],
            piercing_critical_values: vec![],
            positioning_impacts: [0; 3],
        };
        assert_eq!(
            screening_description(
                false,
                &census,
                &screening,
                &defines,
                Err("not read".into()),
                Err("not read".into()),
                &NoIcons,
                &loc
            )
            .unwrap()
            .0,
            "Screens 0.00\n\nNo screens"
        );
    }

    #[test]
    fn native_stat_headers_pair_text_and_background_with_delayed_damage() {
        let loc: Localisation = [
            (
                "combat_stat_light_guns_info".to_owned(),
                "Light guns".to_owned(),
            ),
            (
                "screening_efficiency_info".to_owned(),
                "Screening".to_owned(),
            ),
            ("combat_stat_heavy_guns_info".into(), "Heavy guns".into()),
            ("combat_stat_torpedoes_info".into(), "Torpedoes".into()),
            (
                "combat_stat_depth_charges_info".into(),
                "Depth charges".into(),
            ),
            ("damage_done_by_weapon_type".into(), "Total $NUM$".into()),
            (
                "relative_damage_ratio_to_enemy_health".into(),
                "Enemy $NUM$".into(),
            ),
            ("damage_dealt_screen".into(), "Screens $NUM$ $RATIO$".into()),
        ]
        .into_iter()
        .collect();
        let mut out = Bindings::default();
        let mut side = NavalCombatSideDetails::default();
        side.damage_history[0][2] = 250_000;
        let opponent = NavalCombatSideDetails {
            original_health: 1_000_000,
            ..Default::default()
        };
        side_hovers(&mut out, &NoIcons, &loc, 42, "left", &side, &opponent, None);
        let text = &out.values["header_container/left_wnd/stat_light_gun"];
        let background = &out.values["header_container/left_wnd/stat_light_gun_bg"];
        assert_eq!(text.tooltip.as_deref(), Some("Light guns"));
        assert_eq!(background.tooltip, text.tooltip);
        assert!(background.hover_owned);
        assert!(background.tooltip_description.is_none());
        assert_eq!(
            out.values["header_container/left_wnd/stat_torpedo"]
                .tooltip_description
                .as_deref(),
            Some("Total 2.50\nEnemy 0.25\nScreens 2.50 1.00")
        );
        assert_eq!(
            out.values["left_wnd_capital_screening_efficiency"]
                .tooltip
                .as_deref(),
            Some("Screening")
        );
        assert_eq!(out.refused().len(), 2);
        assert!(
            out.refused()
                .keys()
                .any(|reason| reason.contains("NNavy piercing defines"))
        );
    }
}
