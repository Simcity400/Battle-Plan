
use content_reader_script::sub_units::SubUnits;
use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::{rows::NavalCombatSideDetails, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::io;
use dashboard_text::table_access::{Available, TokenNames};
use viewer_foundation::tooltip_text::country_localised;
use viewer_foundation::tooltip_text::ensure_newlines;
use viewer_foundation::tooltip_text::tooltip_keys;
use viewer_foundation::tooltip_text::validate;
use super::content::Content as StaticContent;
use viewer_foundation::tables::rows;
use {
    super::{
        binding::Bindings,
        modifier_text::{Block, Definitions},
        naval_screening::{Census, Screening},
        tooltips::{hover_text, set_hover},
    },
    viewer_foundation::tooltip_text::{TextIcons, builtin},
};

tooltip_keys! {
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
const DAMAGE_CATEGORIES: [&str; 6] = [
    DAMAGE_DEALT_SCREEN,
    DAMAGE_DEALT_BATTLE_BOX,
    DAMAGE_DEALT_CARRIERS,
    DAMAGE_DEALT_CONVOYS,
    DAMAGE_DEALT_SUBS,
    DAMAGE_DEALT_AIR,
];
const DAMAGE_WEAPONS: [&str; 5] = [
    DAMAGE_DEALT_LIGHT_GUNS,
    DAMAGE_DEALT_HEAVY_GUNS,
    DAMAGE_DEALT_TORPEDOES,
    DAMAGE_DEALT_DEPTH_CHARGES,
    DAMAGE_DEALT_AIR,
];

fn damage_ratio(value: i64, denominator: i64) -> i64 {
    super::fixed::fraction(value, denominator).unwrap_or(super::fixed::EXCEPTIONAL)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the correction branch's census, side and text resources are separate operands"
)]
fn country_correction(
    carrier: bool,
    census: &Census,
    screening: &Screening,
    combat: u64,
    side: &NavalCombatSideDetails,
    opposing: &NavalCombatSideDetails,
    trees: &super::naval_trees::Sources<'_>,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<String, String> {
    let branch = screening.country_bonus_applied
        || (screening.capital == 0
            && !side.summary.countries.is_empty()
            && census.eligible_submarines == 0);
    let mut text = String::new();
    if !carrier && branch {
        trees.side(combat, side, opposing, 0x1b3, "", &mut text, icons, loc)?;
    }
    Ok(text)
}

#[expect(
    clippy::too_many_arguments,
    reason = "native screening text has separate combat and content operands"
)]
fn screening_description(
    carrier: bool,
    census: &Census,
    screening: &Screening,
    defines: &content_reader_naval_combat_defines::naval_combat_defines::Defines,
    correction: &str,
    static_modifier: Result<&content_reader_script::modifiers::Modifiers, String>,
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
        ensure_newlines(&mut text, 2);
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
        ensure_newlines(&mut text, 2);
        text.push_str(&builtin(icons, loc, SCREENING_TOOLTIP_LOW_INFO, &[])?);
    }
    Ok((text, dropped))
}

fn positioning_description(
    terms: super::naval_positioning::Terms,
    sources: impl FnOnce(&mut String) -> Result<(), String>,
    defines: &content_reader_naval_combat_defines::naval_combat_defines::Defines,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<String, String> {
    let mut description = String::new();
    if terms.base != 0 {
        description.push_str(&builtin(
            icons,
            loc,
            MODIFIER_BASE_VALUE_PERCENTAGE,
            &[("VALUE", Argument::Fixed5(terms.base))],
        )?);
        description.push('\n');
    }
    sources(&mut description)?;
    if terms.escort > 1_000 {
        description.push_str(&builtin(
            icons,
            loc,
            CONVOY_ESCORT_POSITIONING_BONUS,
            &[("VALUE", Argument::Fixed5(terms.escort))],
        )?);
        if !description.ends_with('\n') {
            description.push('\n');
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
        description.push_str(&builtin(
            icons,
            loc,
            key,
            &[("NUM", Argument::Fixed5(amount))],
        )?);
        description.push('\n');
    }
    if terms.value < super::fixed::ONE {
        description.push('\n');
        description.push_str(&builtin(icons, loc, POSITIONING_PENALTY_INFO, &[])?);
        description.push('\n');
        for (impact, key, argument) in [
            (defines.positioning_impacts[0], BM_ATTACK_VALUE, "VALUE"),
            (defines.summary_fixed[7], SCREENING_VALUE, "NUM"),
            (defines.positioning_impacts[1], AA_MODIFIER, "NUM"),
        ] {
            let penalty = impact.wrapping_sub(super::fixed::mul(terms.value, impact));
            if penalty > 0 {
                description.push_str(&builtin(
                    icons,
                    loc,
                    key,
                    &[(argument, Argument::Fixed5(penalty.wrapping_neg()))],
                )?);
                description.push('\n');
            }
        }
        let reveal = defines.positioning_impacts[2].wrapping_sub(super::fixed::mul(
            terms.value,
            defines.positioning_impacts[2],
        ));
        if reveal > 0 {
            description.push_str(&builtin(icons, loc, STAT_NAVY_SUB_VISIBILITY, &[])?);
            description.push_str(&builtin(
                icons,
                loc,
                STAT_NAVY_SUB_VISIBILITY_MOD_DIFF,
                &[("VALUE", Argument::Fixed5(reveal))],
            )?);
            description.push('\n');
        }
    }
    let mut text = builtin(icons, loc, POSITIONING_INFO, &[])?;
    text.push('\n');
    text.push_str(&description);
    Ok(text)
}

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
    let mut by_type: Vec<_> = side.damage_by_type.iter().collect();
    by_type.sort_by_key(|source| source.damage);
    for source in by_type {
        let token = if source.token == 0 {
            content_reader_parser::tokens::Token::Rejected
        } else {
            content_reader_parser::tokens::Token::register(
                tokens
                    .name(source.token)
                    .map_err(|error| error.to_string())?
                    .as_bytes(),
            )
        };
        let name = sub_units.token_name(&token)?;
        let name = country_localised(loc, viewing_tag, name)?;
        validate(icons, &name)?;
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

fn weapon_description(
    side: &NavalCombatSideDetails,
    opponent: &NavalCombatSideDetails,
    weapon: usize,
    piercing: Option<&content_reader_naval_combat_defines::naval_combat_defines::Defines>,
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
    if weapon <= 1 {
        let defines = piercing
            .ok_or("the matching NNavy piercing defines are unavailable")?
            .piercing
            .as_ref()
            .map_err(|why| format!("the matching NNavy piercing defines are unavailable: {why}"))?;
        if defines.thresholds.len() < 2 || defines.thresholds.len() != defines.critical_values.len()
        {
            return Err(
                "NNavy piercing threshold arrays differ or have fewer than two entries".into(),
            );
        }
        ensure_newlines(&mut text, 2);
        text.push_str(&builtin(icons, loc, NAVY_PIERCING_TOOLTIP_INFO, &[])?);
        for (index, &critical) in defines.critical_values.iter().enumerate() {
            if !text.ends_with('\n') {
                text.push('\n');
            }
            let last = index + 1 == defines.critical_values.len();
            let threshold = defines.thresholds[index - usize::from(last)];
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

#[expect(clippy::too_many_arguments, reason = "the bindings, the text resources, both sides' details and the defines")]
fn side_hovers(
    out: &mut Bindings,
    icons: &dyn TextIcons,
    loc: &Localisation,
    id: u64,
    side: &str,
    value: &NavalCombatSideDetails,
    opponent: &NavalCombatSideDetails,
    defines: Option<&content_reader_naval_combat_defines::naval_combat_defines::Defines>,
) {
    let path = format!("header_container/{side}_wnd");
    for (index, (widget, key)) in WEAPON_HOVERS.into_iter().enumerate() {
        let pair = [format!("{path}/{widget}"), format!("{path}/{widget}_bg")];
        header(out, icons, loc, id, side, &pair, key);
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

fn screening_inputs<'a>(
    id: u64,
    side: u8,
    detail: &NavalCombatSideDetails,
    ships: Option<&[replay_data::rows::NavalCombatShipRow]>,
    models: &Available<'_, tables::ShipModels>,
    defines: Option<&'a content_reader_naval_combat_defines::naval_combat_defines::Defines>,
) -> Result<
    (
        Census,
        Screening,
        &'a content_reader_naval_combat_defines::naval_combat_defines::Defines,
    ),
    String,
> {
    let defines = defines.ok_or("matching naval combat defines are unavailable")?;
    let ships = ships.ok_or("NavalCombatShips is incomplete")?;
    let model_rows = if models.is_complete() {
        models.rows()
    } else {
        &[]
    };
    let keyed_models = super::keyed::Keyed::new(model_rows, |model| model.ship)
        .ok_or("ShipModels is unordered")?;
    let census =
        super::naval_screening::census(id, side, ships, &keyed_models).map_err(|reason| {
            match models {
                Available::Complete(_) => reason.to_owned(),
                Available::Truncated { missing, .. } => {
                    format!("ShipModels is truncated this hour ({missing} rows not reached)")
                }
                Available::Unavailable { reason } => {
                    format!("ShipModels is unavailable this hour ({reason})")
                }
                Available::NotRecorded { mode } => {
                    format!("ShipModels is not recorded in a {mode} recording")
                }
            }
        })?;
    let screening =
        super::naval_screening::calculate(id, side, detail, ships, &keyed_models, defines)?;
    Ok((census, screening, defines))
}

#[expect(
    clippy::too_many_arguments,
    reason = "the hover's fresh terms, both sides, trees and text resources are separate operands"
)]
fn positioning_hover(
    out: &mut Bindings,
    id: u64,
    name: &str,
    side: usize,
    detail: &replay_data::rows::NavalCombatDetailsRow,
    terms: Option<super::naval_positioning::Terms>,
    trees: &super::naval_trees::Sources<'_>,
    defines: Option<&content_reader_naval_combat_defines::naval_combat_defines::Defines>,
    icons: &dyn TextIcons,
    loc: &Localisation,
) {
    let positioning = (|| {
        let terms =
            terms.ok_or("fresh positioning operands or fixed modifier layout are unavailable")?;
        let defines = defines.ok_or("matching naval combat defines are unavailable")?;
        let (own, opposing) = (&detail.sides[side], &detail.sides[1 - side]);
        positioning_description(
            terms,
            |text| {
                trees.side(detail.combat, own, opposing, 0x1b2, "", text, icons, loc)?;
                if terms.early_defender {
                    trees.side(detail.combat, own, opposing, 0x5a, "", text, icons, loc)?;
                }
                Ok(())
            },
            defines,
            icons,
            loc,
        )
    })();
    for key in [
        format!("header_container/{name}_wnd/stat_positioning"),
        format!("header_container/{name}_wnd/stat_positioning_bg"),
    ] {
        let hover = positioning.as_ref().ok().and_then(|text| hover_text(text));
        set_hover(&mut out.values, key, hover, None);
    }
    if let Err(reason) = positioning {
        out.refuse(format!(
            "Naval combat {id} {name} positioning hover is unavailable: {reason}"
        ));
    }
}

pub(super) fn bind(
    out: &mut Bindings,
    static_content: &StaticContent,
    frame: &DecodedFrame,
    id: u64,
) -> io::Result<()> {
    let details = rows::<tables::NavalCombatDetails>(frame)?;
    let Some(detail) = details.rows().iter().find(|row| row.combat == id) else {
        return Ok(());
    };
    let icons = static_content.assets.as_ref();
    let loc = &static_content.localisation;
    let ships = rows::<tables::NavalCombatShips>(frame)?;
    let models = rows::<tables::ShipModels>(frame)?;
    let bases = rows::<tables::AirBases>(frame)?;
    let live_ships = rows::<tables::Ships>(frame)?;
    let dense_rows = rows::<tables::NavalCombatAirBaseWingHandles>(frame)?;
    let wings = rows::<tables::AirWings>(frame)?;
    let trees = super::naval_trees::Sources::read(frame, &static_content.recorded_labels)?;
    let definitions = Definitions::read(frame, &static_content.modifier_names)?;
    let keyed_models = models
        .is_complete()
        .then(|| super::keyed::Keyed::new(models.rows(), |model| model.ship))
        .flatten();
    let dense = (dense_rows.is_complete() && wings.is_complete())
        .then(|| super::naval_air_groups::DenseBases::new(dense_rows.rows(), wings.rows()))
        .flatten();
    let position_terms = if replay_derive::fixed_layout::validate_recording(
        frame,
        replay_derive::fixed_layout::NAVAL_POSITIONING,
    )
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
        if !detail.sides[side].summary.countries.is_empty() {
            side_hovers(
                out,
                icons,
                loc,
                id,
                name,
                &detail.sides[side],
                &detail.sides[1 - side],
                static_content.assets.naval_defines.as_ref(),
            );
            positioning_hover(
                out,
                id,
                name,
                side,
                detail,
                position_terms[side],
                &trees,
                icons.naval_defines.as_ref(),
                icons,
                loc,
            );
            let screening_inputs = screening_inputs(
                id,
                side as u8,
                &detail.sides[side],
                ships.is_complete().then_some(ships.rows()),
                &models,
                icons.naval_defines.as_ref(),
            );
            for (carrier, widget) in [
                (false, "capital_screening_efficiency"),
                (true, "carrier_screening_efficiency"),
            ] {
                let delayed = (|| {
                    let (census, screening, defines) =
                        screening_inputs.as_ref().map_err(Clone::clone)?;
                    let correction = country_correction(
                        carrier,
                        census,
                        screening,
                        detail.combat,
                        &detail.sides[side],
                        &detail.sides[1 - side],
                        &trees,
                        icons,
                        loc,
                    )?;
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
                            out.refuse(format!("Naval combat {id} {name} {widget}: static screening modifier {dropped} has no registered definition and is dropped as the game drops it."));
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
