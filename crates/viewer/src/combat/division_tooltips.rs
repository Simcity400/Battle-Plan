//! CCombatantEntry/ReserveEntry hover builders, rebuilt from simulation rows.
//! Native dispatchers 1415BF4B0/1415C0840; stat composition 1415BE410/1415BD840;
//! reserve chance 140C6D610 and movement contribution 140C76E90.
use super::{
    Selection, Values,
    binding::Bindings,
    binding::{TextIcons, builtin, token},
    defines::{ArmourHover, LandDefines},
    fixed::{fraction, mul},
    land_details::{
        Condition, Divisions, condition_gap, conditions, direction_count, division_width, modifier,
        total_width,
    },
    tactics::Content as Tactics,
    tooltips::{hover_text, organisation_effects, refuse_hovers, set_hover},
};
use crate::{
    document::Document,
    map::perspective::Vision,
    table_access::{Available, rows},
};
use installed_content::{
    localisation::{Argument, Localisation},
    map::ProvinceKind,
};
use replay::{
    frame::DecodedFrame,
    rows::{
        DivisionCombatModifiers, LandCombatDetailsRow, LandCombatDivisionRow, LandCombatSideRow,
        tables,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

// 140C781E0 -> 140C6F8B0. The maximum getter floors max HP to .001.
fn combat_strength(strength: i64, maximum: i64, factor: i64) -> Option<i64> {
    Some(
        mul(
            fraction(strength, maximum.max(100))?.clamp(0, 100_000),
            factor,
        )
        .clamp(0, 100_000),
    )
}

// Fixed5ToString (1424B6200 -> format_fixed5 1424B89F0) at precision 1:
// truncated, a negative value near zero keeping its sign, and the whole
// part through the signed i32 formatter (1424B7A80).
fn one_decimal(value: i64) -> String {
    installed_content::fixed5::fixed5_to_string(value, 1)
}

/// The complete modifier-name dispatch141440750, in the recorded native order.
const MODIFIER_NAMES: [&str; 30] = [
    BM_LEADER_BONUS,
    BM_COUNTRY_BONUS,
    BM_ENCIRCLEMENT_PENALTY,
    BM_EXPERIENCE,
    BM_PLANNING,
    BM_STACKING,
    BM_WIDTH,
    BM_DUGIN_MODIFIER,
    BM_AMPH_PENALTY,
    BM_FORT_MODIFIER,
    BM_RIVER_PENALTY,
    BM_SHORE_BOMBARD,
    BM_RAILWAY_GUN_BOMBARD,
    BM_MULTIPLE,
    BM_ENEMY_AIR_SUPERIORITY,
    BM_AIR_SUPPORT,
    BM_PARADROP,
    BM_SUPPLY,
    BM_DECRYPTION,
    BM_HATES_COUNTRY,
    BM_HATES_COUNTRY_ON_CORE,
    BM_BORDER_WAR_MODIFIER,
    BM_EXILE_LEADER_DIVISION,
    BM_ACTIVE_CODE_CRACKING_BONUSES,
    BM_LOCAL_BONUS,
    BM_BASE_PROXIMITY,
    BM_POOR_SCREEN_PENALTY,
    BM_TERRAIN,
    BM_WEATHER,
    BM_NIGHT_MODIFIER,
];

fn tactic_effects(
    icons: &dyn TextIcons,
    loc: &Localisation,
    detail: &LandCombatDetailsRow,
    sides: Option<&LandCombatSideRow>,
    tactics: &Tactics,
    side: u8,
) -> Result<String, String> {
    let organisation = organisation_effects(tactics, detail, sides)?;
    let (damage_key, damage, org_key, org_gate) = if side == 0 {
        (
            TACTICS_DAMAGE_ATTACKER,
            detail.attacker_bonus,
            TACTICS_ATTACKER_ORG_DAMAGE_MODIFIER,
            organisation[0],
        )
    } else {
        (
            TACTICS_DAMAGE_DEFENDER,
            detail.defender_bonus,
            TACTICS_DEFENDER_ORG_DAMAGE_MODIFIER,
            organisation[1],
        )
    };
    let mut text = String::new();
    //1412A03A0 tests defender C8 but formats attacker C0 on the defender org
    // line as well (ASM1412A0FEC/1412A1182/1412A1357). Preserve that native quirk.
    for (key, gate, value) in [
        (damage_key, damage, damage),
        (org_key, org_gate, organisation[0]),
    ] {
        if gate == 0 {
            continue;
        }
        text.push_str(&builtin(icons, loc, key, &[])?);
        text.push(' ');
        text.push_str(&builtin(
            icons,
            loc,
            if gate > 0 {
                TACTICS_VALUE_POSITIVE
            } else {
                TACTICS_VALUE_NEGATIVE
            },
            &[("VALUE", Argument::Fixed5(value))],
        )?);
        text.push('\n');
    }
    Ok(text)
}

/// 1415BE410's combat-stat hover, or why it cannot be made: the refused
/// localisation key or missing operand.
fn combat_stat(
    icons: &dyn TextIcons,
    loc: &Localisation,
    key: &str,
    [base, scale]: [i64; 2],
    modifiers: &DivisionCombatModifiers,
    attack: bool,
    tactics: Result<&str, &str>,
) -> Result<String, String> {
    let name = builtin(icons, loc, key, &[])?;
    let value = builtin(
        icons,
        loc,
        &format!("{key}_VALUE"),
        &[("VALUE", Argument::Fixed5(mul(base, scale)))],
    )?;
    let mut text = builtin(
        icons,
        loc,
        COMBAT_STAT,
        &[
            ("STAT_NAME", Argument::Text(&name)),
            ("VALUE", Argument::Text(&value)),
        ],
    )?;
    text.push('\n');
    text.push_str(&builtin(
        icons,
        loc,
        BASE_VALUE,
        &[("VALUE", Argument::Fixed5(base))],
    )?);
    text.push('\n');
    text.push_str(&builtin(
        icons,
        loc,
        TOTAL_MODIFIER_IS,
        &[("VALUE", Argument::Fixed5(scale))],
    )?);
    text.push('\n');
    for modifier in &modifiers.entries {
        let value = if attack {
            modifier.value_a
        } else {
            modifier.value_b
        };
        if value == 0 {
            continue;
        }
        let modifier_key = MODIFIER_NAMES
            .get(modifier.id as usize)
            .ok_or_else(|| format!("combat modifier {} has no native name key", modifier.id))?;
        let name = builtin(icons, loc, modifier_key, &[])?;
        text.push_str(&builtin(
            icons,
            loc,
            SINGLE_COMBAT_VALUE,
            &[
                ("NAME", Argument::Text(&name)),
                ("VALUE1", Argument::Fixed5(value)),
            ],
        )?);
        text.push('\n');
    }
    if attack {
        text.push_str(
            tactics.map_err(|reason| format!("the current tactic's attack lines: {reason}"))?,
        );
    }
    Ok(text)
}

// 1415BF4B0's default child dispatch. These two stat lines use the same
// 1415BE410 formatter as the special attack/defence hovers, without a combat
// modifier breakdown. FormatFixed5Trimmed (1424B60D0) formats the headline
// value before COMBAT_STAT: the magnitude by wrapping negation, its whole
// part through the signed i32 formatter (1424B7A80) and every nonzero
// decimal, trailing zeros trimmed, which is format_fixed5's mode 0
// (1424B89F0 with a negative precision). The numeric base line still uses
// fixed5 localisation. The piercing table is authored in NMilitary, and its
// final "less than" label uses the preceding threshold (ASM 1415C00A2).
fn fixed_decimal(value: i64) -> String {
    installed_content::fixed5::fixed5_to_string(value, -1)
}

/// 1415C3AA0 frame 8/9: the two authored dice sizes are signed integers;
/// native Fixed5 division first scales each by 100000 and returns 0xffffffff
/// for a zero denominator. The bonus text is gated by the resulting ratio,
/// while the defence reduction always appears in the delayed paragraph.
fn armour_description(
    icons: &dyn TextIcons,
    loc: &Localisation,
    defines: &ArmourHover,
) -> Result<String, String> {
    let mut text = builtin(
        icons,
        loc,
        GOOD_ARMOR_DEFENSE_DESC,
        &[
            (
                "ORG",
                Argument::Fixed5(100_000_i64.wrapping_sub(defines.organisation_deflection)),
            ),
            (
                "STR",
                Argument::Fixed5(100_000_i64.wrapping_sub(defines.strength_deflection)),
            ),
        ],
    )?;
    for (key, numerator, denominator) in [
        (
            GOOD_ARMOR_ATTACK_DESC_STR,
            defines.strength_armour_dice,
            defines.strength_die,
        ),
        (
            GOOD_ARMOR_ATTACK_DESC_ORG,
            defines.organisation_armour_dice,
            defines.organisation_die,
        ),
    ] {
        let ratio = fraction(
            i64::from(numerator).wrapping_mul(100_000),
            i64::from(denominator).wrapping_mul(100_000),
        )
        .ok_or_else(|| "an armour die size is zero".to_owned())?
        .wrapping_sub(100_000);
        if ratio > 0 {
            text.push_str(&builtin(
                icons,
                loc,
                key,
                &[("VAL", Argument::Fixed5(ratio))],
            )?);
        }
    }
    Ok(text)
}

fn base_stat(
    icons: &dyn TextIcons,
    loc: &Localisation,
    key: &str,
    value: i64,
) -> Result<String, String> {
    let name = builtin(icons, loc, key, &[])?;
    let number = fixed_decimal(value);
    let mut text = builtin(
        icons,
        loc,
        COMBAT_STAT,
        &[
            ("STAT_NAME", Argument::Text(&name)),
            ("VALUE", Argument::Text(&number)),
        ],
    )?;
    text.push('\n');
    text.push_str(&builtin(
        icons,
        loc,
        BASE_VALUE,
        &[("VALUE", Argument::Fixed5(value))],
    )?);
    Ok(text)
}

/// `name` is the division's standard template's display name: 1415BF4B0
/// passes `CArmy+0x3B8`'s `CDivisionTemplateData` (140C6F080, through the
/// identity thunk in vtable slot 8) to GetDisplayName (140B91170).
/// `invasion` is the first-edge progress (140BEC9A0) of a division making an
/// amphibious invasion move (140BF0E20), whose section follows the name.
fn division_summary(
    icons: &dyn TextIcons,
    loc: &Localisation,
    name: &str,
    row: &LandCombatDivisionRow,
    invasion: Option<i64>,
    defines: Option<&LandDefines>,
) -> Result<String, String> {
    let mut text = builtin(
        icons,
        loc,
        if row.side == 0 {
            COMBAT_DIVISION_ATTACKER
        } else {
            COMBAT_DIVISION_DEFENDER
        },
        &[("NAME", Argument::Text(name))],
    )?;
    if let Some(progress) = invasion {
        text.push_str(&invasion_section(icons, loc, progress, row.scales)?);
    }
    text.push_str(&base_stat(icons, loc, STAT_COMMON_ARMOR, row.armour)?);
    text.push_str("\n\n");
    let hardness = row.stat_values[2];
    text.push_str(&builtin(
        icons,
        loc,
        HARDNESS_COMBAT_DESC,
        &[
            ("HARDNESS", Argument::Fixed5(hardness)),
            (
                "SOFTNESS",
                Argument::Fixed5(100_000_i64.wrapping_sub(hardness)),
            ),
        ],
    )?);
    text.push('\n');
    text.push_str(&builtin(icons, loc, COMBAT_TOOLTIP_SEPARATOR, &[])?);
    text.push('\n');
    text.push_str(&base_stat(icons, loc, STAT_COMMON_PIERCING, row.piercing)?);
    text.push('\n');
    text.push_str(&builtin(icons, loc, PIERCING_COMBAT_DESC, &[])?);
    if let Some(defines) = defines {
        for (index, damage) in defines.piercing_damage.iter().enumerate() {
            if !text.ends_with('\n') {
                text.push('\n');
            }
            let last = index + 1 == defines.piercing_damage.len();
            text.push_str(&builtin(
                icons,
                loc,
                if last {
                    PIERCING_THRESHOLD_TO_DAMAGE_LOWEST
                } else {
                    PIERCING_THRESHOLD_TO_DAMAGE
                },
                &[
                    (
                        "THRESHOLD",
                        Argument::Fixed5(
                            defines.piercing_thresholds[if last { index - 1 } else { index }],
                        ),
                    ),
                    ("DAMAGE", Argument::Fixed5(*damage)),
                ],
            )?);
        }
        text.push('\n');
        text.push_str(&builtin(icons, loc, COMBAT_TOOLTIP_SEPARATOR, &[])?);
    }
    Ok(text)
}

/// 1415BF4B0's naval invasion section. `scales` are CUnit+0x188..+0x1A8; the
/// section shows +0x198 and +0x1A0 less one (fixed5) when either differs.
/// The separators are the executable's "\n\n" (14271A054) and "\n"
/// (142702E68).
fn invasion_section(
    icons: &dyn TextIcons,
    loc: &Localisation,
    progress: i64,
    scales: [i64; 4],
) -> Result<String, String> {
    let mut text = builtin(icons, loc, NAVAL_INVASION_COMBAT_PROGRESS_DESC, &[])?;
    text.push_str("\n\n");
    text.push_str(&builtin(
        icons,
        loc,
        if progress < 100_000 {
            NAVAL_INVASION_PROGRESS
        } else {
            NAVAL_INVASION_PROGRESS_COMPLETE
        },
        &[("VALUE", Argument::Fixed5(progress))],
    )?);
    let [attack, defend] = [scales[2], scales[3]].map(|scale| scale.wrapping_sub(100_000));
    if attack != 0 || defend != 0 {
        for (key, value) in [
            (NAVAL_INVASION_ATTACK_SCALE, attack),
            (NAVAL_INVASION_DEFEND_SCALE, defend),
        ] {
            text.push('\n');
            text.push_str(&builtin(
                icons,
                loc,
                key,
                &[("VALUE", Argument::Fixed5(value))],
            )?);
        }
    }
    text.push_str("\n\n");
    Ok(text)
}

/// The canonical-slot joins every invasion test of one bind shares: built
/// once per bind, not once per division.
struct Wars {
    slots: BTreeMap<u32, u32>,
    /// Live (nonretired) war relations as canonical-slot pairs, both ways.
    pairs: BTreeSet<(u32, u32)>,
    /// A live war relation names a country without a link, so whether a pair
    /// is at war can be unknown.
    unlinked: bool,
    /// The tables the test reads that are not a complete observation.
    incomplete: Vec<&'static str>,
}

impl Wars {
    fn new(
        links: &Available<'_, tables::CountryLinks>,
        relations: &Available<'_, tables::RelationTuples>,
        mut incomplete: Vec<&'static str>,
    ) -> Self {
        let slots: BTreeMap<_, _> = links
            .rows()
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        let mut pairs = BTreeSet::new();
        let mut unlinked = false;
        for relation in relations
            .rows()
            .iter()
            .filter(|row| row.type_id == engine::cwar_relation::TYPE && row.retired == 0)
        {
            match (
                slots.get(&relation.first_country),
                slots.get(&relation.second_country),
            ) {
                (Some(&first), Some(&second)) => {
                    pairs.insert((first, second));
                    pairs.insert((second, first));
                }
                _ => unlinked = true,
            }
        }
        if !links.is_complete() {
            incomplete.push("CountryLinks");
        }
        if !relations.is_complete() {
            incomplete.push("RelationTuples");
        }
        Self {
            slots,
            pairs,
            unlinked,
            incomplete,
        }
    }
}

/// 140BF0E20(unit, false): the unit's next route step leaves a sea province
/// for a land province controlled by a country it is at war with (1406F3A00,
/// the live, nonretired relation). The error names the row or relation
/// whose absence leaves the test undecided.
fn naval_invasion(
    unit: u64,
    movements: &Available<'_, tables::DivisionMovement>,
    divisions: &Available<'_, tables::Divisions>,
    provinces: &Available<'_, tables::Provinces>,
    kinds: &impl Fn(u32) -> Option<ProvinceKind>,
    wars: &Wars,
) -> Result<bool, String> {
    let movement = movements
        .rows()
        .iter()
        .find(|row| row.division == unit)
        .ok_or("it has no DivisionMovement row")?;
    if movement.path_total == 0 {
        return Ok(false);
    }
    if !wars.incomplete.is_empty() {
        return Err(format!(
            "the {} table{} incomplete this hour",
            wars.incomplete.join(", "),
            if wars.incomplete.len() == 1 {
                " is"
            } else {
                "s are"
            }
        ));
    }
    let next = movement
        .path
        .first()
        .copied()
        .filter(|&next| next != 0)
        .ok_or("its route's next step is not recorded")?;
    let division = divisions
        .rows()
        .iter()
        .find(|row| row.id == unit)
        .ok_or("it has no Divisions row")?;
    let target = provinces
        .rows()
        .iter()
        .find(|row| row.id == next)
        .ok_or_else(|| format!("its next province {next} has no Provinces row"))?;
    let kind = |province: u32| {
        kinds(province).ok_or_else(|| format!("province {province} is not in the map definition"))
    };
    if kind(division.province)? != ProvinceKind::Sea || kind(target.id)? != ProvinceKind::Land {
        return Ok(false);
    }
    if target.controller == 0 || division.logical_owner == 0 {
        return Ok(false);
    }
    let slot = |country: u32| {
        wars.slots
            .get(&country)
            .copied()
            .ok_or_else(|| format!("country {country} has no CountryLinks row"))
    };
    let owner = slot(division.logical_owner)?;
    let controller = slot(target.controller)?;
    if owner == controller {
        return Ok(false);
    }
    if wars.pairs.contains(&(owner, controller)) {
        Ok(true)
    } else if wars.unlinked {
        Err("a live war relation names a country without a CountryLinks row, so whether its owner is at war with the controller is unknown".into())
    } else {
        Ok(false)
    }
}

/// CModifierIconEntry::GetTooltip (1415C3AA0), frames generated by
/// division_icons::frames. The first eight frames retain their source's
/// first matching modifier entry; armor/piercing frames count the complete
/// opposing committed roster into the same native buckets as their icon law.
const ICON_MODIFIERS: [(u32, &str); 8] = [
    (2, BM_ENCIRCLEMENT_PENALTY),
    (7, BM_DUGIN_MODIFIER),
    (10, BM_RIVER_PENALTY),
    (13, BM_MULTIPLE),
    (27, BM_TERRAIN),
    (8, BM_AMPH_PENALTY),
    (17, BM_SUPPLY),
    (16, BM_PARADROP),
];

struct ModifierIconContext<'context, 'frame> {
    icons: &'context dyn TextIcons,
    loc: &'context Localisation,
    side: u8,
    unit: &'frame LandCombatDivisionRow,
    modifiers: Option<&'frame DivisionCombatModifiers>,
    /// The opposing committed rows, or which one is missing.
    opponents: Result<Vec<&'frame LandCombatDivisionRow>, String>,
    /// The land combat defines, or why they are unavailable.
    defines: Result<&'context LandDefines, &'context str>,
    viewing_tag: Result<&'frame str, &'static str>,
    detail: &'frame LandCombatDetailsRow,
    template_inputs: &'context super::template_icons::Inputs<'frame>,
}

fn modifier_icon_tooltip(
    frame: u32,
    context: &ModifierIconContext<'_, '_>,
) -> Result<(String, Option<Result<String, String>>), String> {
    let (icons, loc) = (context.icons, context.loc);
    let side = context.side;
    let unit = context.unit;
    let modifiers = context.modifiers;
    let opponents = context.opponents.as_deref().map_err(String::as_str);
    let defines = context.defines.map_err(|reason| {
        format!("the land combat defines (NMilitary, NCountry) are unavailable ({reason})")
    });
    let viewing_tag = context.viewing_tag;
    let detail = context.detail;
    let template_inputs = context.template_inputs;
    if let Some((id, key)) = ICON_MODIFIERS.get(frame as usize) {
        let name = builtin(icons, loc, key, &[])?;
        let mut text = builtin(
            icons,
            loc,
            super::tooltips::HEADER,
            &[("KEY", Argument::Text(&name))],
        )?;
        if let Some(entry) =
            modifiers.and_then(|modifiers| modifiers.entries.iter().find(|entry| entry.id == *id))
        {
            if entry.value_a != 0 {
                text.push_str(&builtin(
                    icons,
                    loc,
                    BM_ATTACK_VALUE,
                    &[("VALUE", Argument::Fixed5(entry.value_a))],
                )?);
                text.push('\n');
            }
            if entry.value_b != 0 {
                text.push_str(&builtin(
                    icons,
                    loc,
                    if side == 0 {
                        BM_BREAKTHROUGH_VALUE
                    } else {
                        BM_DEFEND_VALUE
                    },
                    &[("VALUE", Argument::Fixed5(entry.value_b))],
                )?);
                text.push('\n');
            }
            return Ok((text, Some(builtin(icons, loc, &format!("{key}_DESC"), &[]))));
        }
        // 1415BEBA0 returns an empty breakdown/description when the icon's
        // modifier is absent; 1415C3AA0 still emits the authored header.
        return Ok((text, None));
    }
    if frame == 12 {
        let commander = unit
            .commander_inputs
            .as_ref()
            .ok_or_else(|| "the division's commander operands are not recorded".to_owned())?;
        let name = token(icons, loc, &commander.name_token)
            .map_err(|refusal| format!("the commander's recorded name: {refusal}"))?;
        let mut text = builtin(
            icons,
            loc,
            COMBAT_OTHER_COMMANDER_DESC,
            &[
                ("NAME", Argument::Text(&name)),
                ("SKILL", Argument::Int32(commander.skill)),
            ],
        )?;
        if !commander.traits.is_empty() {
            text.push_str(&builtin(icons, loc, COMBAT_TRAIT_LIST_HEADER, &[])?);
            for (index, trait_row) in commander.traits.iter().enumerate() {
                if index != 0 {
                    text.push_str(&builtin(icons, loc, COMBAT_DETAIL_TRAIT_SEPARATOR, &[])?);
                }
                let token = std::str::from_utf8(&trait_row.token).map_err(|_| {
                    format!(
                        "commander trait token {} is not UTF-8",
                        String::from_utf8_lossy(&trait_row.token)
                    )
                })?;
                let name =
                    super::commander_tooltips::country_localized(icons, loc, viewing_tag, token)?;
                text.push_str(&builtin(
                    icons,
                    loc,
                    COMBAT_DETAIL_TRAIT_NAME,
                    &[("NAME", Argument::Text(&name))],
                )?);
            }
        }
        return Ok((
            text,
            Some(builtin(icons, loc, DIFFERENT_COMMANDER_DESC, &[])),
        ));
    }
    if frame == 13 {
        let defines = defines?;
        let amount = engine::csub_unit_definition::movement_air_defense_value(
            unit.air_attack,
            defines.air_superiority[0],
            defines.air_superiority[1],
        )
        .map_err(|_| "the air-defence arithmetic overflows".to_owned())?;
        let name = builtin(icons, loc, BM_ENEMY_AIR_SUPERIORITY_COUNTER, &[])?;
        return Ok((
            builtin(
                icons,
                loc,
                SINGLE_COMBAT_VALUE,
                &[
                    ("NAME", Argument::Text(&name)),
                    ("VALUE1", Argument::Fixed5(amount)),
                ],
            )?,
            Some(builtin(
                icons,
                loc,
                BM_ENEMY_AIR_SUPERIORITY_COUNTER_DESC,
                &[],
            )),
        ));
    }
    if frame == 14 {
        let bonuses = defines?.exile_against_occupier;
        let amount = template_inputs
            .occupiers(unit, detail, &bonuses)
            .map_err(|refused| format!("the occupier count is unknown: {refused}"))?;
        let total = detail.sides[1 - side as usize].committed.len();
        return Ok((
            builtin(
                icons,
                loc,
                COMBAT_EXILES_DESC,
                &[
                    (
                        "AMOUNT",
                        Argument::Int32(
                            i32::try_from(amount)
                                .map_err(|_| "the occupier count overflows".to_owned())?,
                        ),
                    ),
                    (
                        "TOTAL",
                        Argument::Int32(
                            i32::try_from(total)
                                .map_err(|_| "the committed count overflows".to_owned())?,
                        ),
                    ),
                ],
            )?,
            Some(builtin(
                icons,
                loc,
                COMBAT_EXILES_DESC_DELAYED,
                &[
                    ("ATK_VS_OCCUPIER", Argument::Fixed5(bonuses[0])),
                    ("DEF_VS_OCCUPIER", Argument::Fixed5(bonuses[1])),
                ],
            )),
        ));
    }
    if frame == 15 {
        let bonuses = defines?.exile_on_core;
        return Ok((
            builtin(
                icons,
                loc,
                COMBAT_EXILES_ON_CORES_DESC,
                &[
                    ("ATK_ON_CORE", Argument::Fixed5(bonuses[0])),
                    ("DEF_ON_CORE", Argument::Fixed5(bonuses[1])),
                ],
            )?,
            None,
        ));
    }
    if frame == 16 {
        let ratio = super::division_icons::fuel_ratio([
            unit.fuel_inputs[0],
            unit.fuel_inputs[1],
            defines?.fuel_penalty_start_ratio,
        ]);
        return Ok((
            builtin(
                icons,
                loc,
                LACK_OF_FUEL,
                &[("VAL", Argument::Fixed5(ratio))],
            )?,
            Some(builtin(icons, loc, LACK_OF_FUEL_DESC, &[])),
        ));
    }
    if !(8..=11).contains(&frame) {
        return Err(format!("frame {frame} is not a native modifier-icon frame"));
    }
    let opponents = opponents.map_err(str::to_owned)?;
    let defines = defines?;
    let mut counts = vec![0_i32; defines.piercing_thresholds.len()];
    for opponent in opponents {
        let bucket = if frame <= 9 {
            super::division_icons::pierce_bucket(
                &defines.piercing_thresholds,
                unit.armour,
                opponent.piercing,
            )
        } else {
            super::division_icons::pierce_bucket(
                &defines.piercing_thresholds,
                opponent.armour,
                unit.piercing,
            )
        };
        *counts
            .get_mut(bucket)
            .ok_or_else(|| "a piercing bucket is outside the defines' thresholds".to_owned())? += 1;
    }
    let mut text = String::new();
    for (index, &count) in counts.iter().enumerate() {
        if count <= 0 || (frame >= 10 && index + 1 == counts.len()) {
            continue;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        let key = if frame >= 10 {
            PIERCING_ENEMIES_THRESHOLD_ENTRY
        } else if index + 1 == counts.len() {
            NON_PIERCING_THRESHOLD_ENTRY
        } else {
            PIERCING_THRESHOLD_ENTRY
        };
        text.push_str(&builtin(
            icons,
            loc,
            key,
            &[
                ("NUM", Argument::Int32(count)),
                (
                    "PIERCE_VALUE",
                    Argument::Fixed5(defines.piercing_damage[index]),
                ),
            ],
        )?);
    }
    let delayed = if frame >= 10 {
        builtin(icons, loc, PIERCING_GOOD_DESC, &[])
    } else {
        armour_description(icons, loc, &defines.armour_hover)
    };
    Ok((text, Some(delayed)))
}

#[derive(Debug, PartialEq, Eq)]
enum Reinforcement {
    Rejected(&'static str),
    Chance {
        value: i64,
        base: i64,
        initiative: i64,
        speed: i64,
    },
}

/// CanAdvance (140C684A0) compares raw HP and current organisation against
/// these executable constants (14293C980, 14293C978), not authored defines.
const ADVANCE_STRENGTH: i64 = 10_000;
const ADVANCE_ORGANISATION: i64 = 100_000;

fn reinforcement(
    row: &LandCombatDivisionRow,
    condition: &Condition,
    widths: Result<(i64, i64), &str>,
    content: Result<&LandDefines, &str>,
) -> Result<Reinforcement, String> {
    if condition.strength < ADVANCE_STRENGTH || condition.organisation < ADVANCE_ORGANISATION {
        return Ok(Reinforcement::Rejected(COMREIC_NOT_FIGHT));
    }
    let defines = content
        .map_err(|reason| format!("the land combat defines are unavailable ({reason})"))?
        .reinforce;
    let (width, used) = widths.map_err(str::to_owned)?;
    let available = width.wrapping_sub(used);
    // 140C684A0's modifier getters (140551820) with an unsupported
    // eligibility callback cannot be evaluated.
    let callback = |what: &str| {
        format!(
            "its {what} modifier has an eligibility callback {} the port does not evaluate",
            row.context.eligibility_callback
        )
    };
    let excess = division_width(row)
        .ok_or_else(|| callback("width"))?
        .wrapping_sub(available);
    if excess > 0 {
        let share = fraction(excess, width)
            .ok_or_else(|| format!("the excess width share: {}", super::fixed::TRAP))?;
        if mul(share, defines.over_width_penalty) < defines.over_width_penalty_max {
            return Ok(Reinforcement::Rejected(COMREIC_TOO_MUCH_WIDTH));
        }
    }
    let country = modifier(&row.context, 0, false).ok_or_else(|| callback("reinforce rate"))?;
    let leader = modifier(&row.context, 0, true).ok_or_else(|| callback("reinforce rate"))?;
    let initiative = mul(
        row.initiative,
        100_000_i64
            .wrapping_add(modifier(&row.context, 2, false).ok_or_else(|| callback("initiative"))?),
    );
    let initiative = if row.context.leader_modifier_kind == 0 {
        initiative
    } else {
        mul(
            initiative,
            100_000_i64.wrapping_add(
                modifier(&row.context, 2, true).ok_or_else(|| callback("initiative"))?,
            ),
        )
    };
    let initiative = mul(initiative, defines.initiative_factor);
    let speed = mul(
        row.context.base_speed,
        super::reinforcement_speed::modifier(row)
            .map_err(|reason| format!("its reinforcement speed factor is unknown: {reason}"))?,
    );
    let speed = fraction(speed, defines.slowest_speed)
        .ok_or_else(|| format!("the slowest-speed ratio: {}", super::fixed::TRAP))?
        .wrapping_sub(100_000)
        .clamp(0, 200_000);
    let speed = mul(speed, defines.speed_bonus);
    Ok(Reinforcement::Chance {
        value: defines
            .chance
            .wrapping_add(country)
            .wrapping_add(initiative)
            .wrapping_add(speed)
            .wrapping_add(leader),
        base: defines.chance,
        initiative,
        speed,
    })
}

fn reserve_tree(
    nodes: &[replay::rows::ModifierBreakdownNode],
    definition: &replay::rows::ModifierDefinitionRow,
    recorded: &str,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<String, String> {
    dashboard::modifier_text::breakdown(
        nodes,
        dashboard::modifier_text::Breakdown {
            depth: -1,
            prefix: "",
            suffix: "",
        },
        |node| {
            let mut text = dashboard::scripted_text::stored_text(
                loc,
                recorded,
                &node.name,
                "reserve reinforcement modifier source",
            )?;
            super::binding::validate(icons, &text).map_err(|reason| {
                format!("reserve reinforcement modifier source holds {reason}")
            })?;
            if !text.is_empty() {
                text.push_str(": ");
            }
            text.push_str(
                &super::modifier_text::value_text(
                    definition, node.value, 2, false, false, icons, loc,
                )
                .map_err(|e| e.to_string())?,
            );
            Ok(text)
        },
    )
}

type ReserveSources = (String, Option<(String, String)>);
fn reserve_sources(
    frame: &DecodedFrame,
    document: &Document,
    row: &LandCombatDivisionRow,
    divisions: &Available<'_, tables::Divisions>,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<ReserveSources, String> {
    let sources = rows::<tables::ReserveModifierBreakdowns>(frame).map_err(|e| e.to_string())?;
    let Available::Complete(sources) = sources else {
        return Err("reserve reinforcement: ReserveModifierBreakdowns is incomplete".into());
    };
    let definitions = rows::<tables::ModifierDefinitions>(frame).map_err(|e| e.to_string())?;
    if !definitions.is_complete() {
        return Err("reserve reinforcement: ModifierDefinitions is incomplete".into());
    }
    let definition = definitions
        .rows()
        .iter()
        .find(|r| r.index == engine::modifier_keys::LAND_REINFORCE_RATE)
        .ok_or("reserve reinforcement: land_reinforce_rate definition is missing")?;
    let unit = divisions
        .rows()
        .iter()
        .find(|r| r.id == row.division)
        .ok_or_else(|| {
            format!(
                "reserve reinforcement: division {} has no Divisions row",
                row.division
            )
        })?;
    // 140C6D610: positive expeditionary owner, otherwise the roster owner.
    let country = if unit.expeditionary_owner as i32 > 0 {
        unit.expeditionary_owner
    } else {
        unit.roster_owner
    };
    let describe = |kind, owner| {
        let source = sources
            .iter()
            .find(|r| r.kind == kind && r.owner == owner)
            .ok_or_else(|| {
                format!(
                    "reserve reinforcement: modifier source kind {kind} owner {owner} is missing"
                )
            })?;
        reserve_tree(
            &source.nodes,
            definition,
            &document.recorded_language,
            icons,
            loc,
        )
    };
    let country = describe(0, country as u64)?;
    // 140C6D610 emits the commander only when its getter is nonzero.
    let commander = if modifier(&row.context, 0, true)
        .ok_or("reserve reinforcement: commander eligibility callback is unsupported")?
        != 0
    {
        let inputs = row
            .commander_inputs
            .as_ref()
            .ok_or("reserve reinforcement: commander name is missing")?;
        Some((
            token(icons, loc, &inputs.name_token)?,
            describe(row.context.leader_modifier_kind, row.commander)?,
        ))
    } else {
        None
    };
    Ok((country, commander))
}

fn reinforcement_text(
    icons: &dyn TextIcons,
    loc: &Localisation,
    result: &Reinforcement,
    sources: Result<&ReserveSources, String>,
) -> Result<String, String> {
    let chance = match result {
        Reinforcement::Rejected(_) => 0,
        Reinforcement::Chance { value, .. } => *value,
    };
    let mut text = builtin(
        icons,
        loc,
        BATTLE_REINFORCE_CHANCE2,
        &[("VALUE", Argument::Fixed5(chance))],
    )?;
    match result {
        Reinforcement::Rejected(key) => text.push_str(&builtin(icons, loc, key, &[])?),
        Reinforcement::Chance {
            base,
            initiative,
            speed,
            ..
        } => {
            let (country, commander) = sources?;
            text.push_str(&builtin(
                icons,
                loc,
                MODIFIER_BASE_VALUE_PERCENTAGE,
                &[("VALUE", Argument::Fixed5(*base))],
            )?);
            text.push('\n');
            text.push_str(country);
            if *initiative != 0 {
                text.push_str(&builtin(icons, loc, STAT_ARMY_INITIATIVE, &[])?);
                text.push_str(&builtin(
                    icons,
                    loc,
                    STAT_ARMY_INITIATIVE_DIFF,
                    &[("VALUE", Argument::Fixed5(*initiative))],
                )?);
                text.push('\n');
            }
            if *speed != 0 {
                text.push_str(&builtin(
                    icons,
                    loc,
                    SPEED_REINFORCEMENT_BONUS,
                    &[("VAL", Argument::Fixed5(*speed))],
                )?);
                text.push('\n');
            }
            if let Some((name, sources)) = commander {
                text.push_str(name);
                text.push_str(": ");
                text.push_str(sources);
            }
            return Ok(text);
        }
    }
    text.push('\n');
    Ok(text)
}

/// The committed entry's children 1415BF4B0 gives the division summary.
const SUMMARY_HOVERS: [&str; 7] = [
    "Background",
    "unit_button",
    "unit_icon",
    "name",
    "soft_attack_icon",
    "hard_attack_icon",
    "breakthrough_icon",
];
/// The committed entry's stat children 1415BF4B0 dispatches specially.
const STAT_HOVERS: [&str; 3] = ["soft_attack", "hard_attack", "defense"];
/// The reserve entry's children, which all take the reinforcement hover
/// (the reserve dispatcher 1415C0840).
const RESERVE_HOVERS: [&str; 6] = [
    "Background",
    "unit_button",
    "unit_icon",
    "name",
    "org_bar",
    "str_bar",
];

/// Clears every hover the division entry's builder owns, when it cannot
/// build the entry at all.
fn refuse_entry(values: &mut Values, reserve: bool) {
    if reserve {
        refuse_hovers(values, RESERVE_HOVERS);
        return;
    }
    refuse_hovers(values, SUMMARY_HOVERS);
    refuse_hovers(values, STAT_HOVERS);
    refuse_hovers(values, ["str_bar", "org_bar"]);
    if let Some(grid) = values.get_mut("modifier_icons_container/list") {
        for entry in &mut grid.entries {
            refuse_hovers(entry, ["modifier_icon"]);
        }
    }
}

pub(super) fn bind(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    selection: Selection,
    vision: &Vision,
) -> io::Result<()> {
    let Selection::Land(id) = selection else {
        return Ok(());
    };
    // Hover parts refused by localisation key or missing operand, counted.
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    let detail_rows = rows::<tables::LandCombatDetails>(frame)?;
    let Some(detail) = detail_rows.rows().iter().find(|row| row.combat == id) else {
        out.warnings.push(format!(
            "Land combat {id}: no LandCombatDetails row, so the division entries' stat, reserve and modifier-icon hovers are omitted."
        ));
        for (index, entries) in out.entries.iter_mut().enumerate() {
            for (_, values) in entries {
                refuse_entry(values, index >= 2);
            }
        }
        return Ok(());
    };
    let sides = super::progress::sides(frame)?;
    let sides = sides.get(&id).copied();
    let division_rows = rows::<tables::LandCombatDivisions>(frame)?;
    let divisions: Divisions<'_> = division_rows
        .rows()
        .iter()
        .filter(|row| row.combat == id)
        .map(|row| ((row.side, row.division), row))
        .collect();
    let modifier_rows = rows::<tables::LandCombatModifiers>(frame)?;
    let movements = rows::<tables::DivisionMovement>(frame)?;
    let all_divisions = rows::<tables::Divisions>(frame)?;
    let provinces = rows::<tables::Provinces>(frame)?;
    let incomplete = [
        ("DivisionMovement", movements.is_complete()),
        ("Divisions", all_divisions.is_complete()),
        ("Provinces", provinces.is_complete()),
    ]
    .into_iter()
    .filter(|(_, complete)| !complete)
    .map(|(table, _)| table)
    .collect();
    let wars = Wars::new(
        &rows::<tables::CountryLinks>(frame)?,
        &rows::<tables::RelationTuples>(frame)?,
        incomplete,
    );
    let template_inputs = super::template_icons::Inputs::read(frame)?;
    // Recorded daily with the template dictionary, so a rename shows from the
    // table's next daily capture.
    let template_names: BTreeMap<u64, &[u8]> = rows::<tables::DivisionTemplates>(frame)?
        .rows()
        .iter()
        .map(|row| (row.template, row.name.as_slice()))
        .collect();
    // The landing's first-edge progress, rebuilt from the recorded route and
    // content as the hour's other movement consumers rebuild it.
    let etas =
        crate::document::Hour::new(&document.content, frame, document.hour_fraction).etas()?;
    let conditions = conditions(frame)?;
    let condition_gap = condition_gap(frame)?;
    let land_defines = document
        .icons
        .combat
        .land_defines
        .as_ref()
        .map_err(String::as_str);
    let defines = land_defines.ok();
    let map = &document.map;
    let kinds = |province| map.definitions.get(&province).map(|row| row.kind);
    let countries = rows::<tables::Countries>(frame)?;
    let viewing_tag = super::text::viewing_tag(countries.rows(), vision);
    let (icons, loc) = (&document.icons.combat, &document.names.localisation);
    let movement_sources: BTreeMap<_, _> = movements
        .rows()
        .iter()
        .map(|row| (row.division, row.source))
        .collect();
    // 141298D50's total width, or why it is unknown.
    let width = direction_count(
        &detail.sides[0].committed,
        &divisions,
        detail.province,
        &movement_sources,
    )
    .map_err(|reason| format!("the 141298E00 direction count is unknown: {reason}"))
    .and_then(|count| {
        let terrain = document
            .counters
            .combat_terrain
            .get(&detail.province)
            .ok_or_else(|| {
                format!(
                    "province {} has no terrain category in the content",
                    detail.province
                )
            })?;
        let widths = document
            .counters
            .combat_widths(terrain)
            .ok_or_else(|| format!("terrain {terrain} is not a content terrain category"))?;
        Ok(total_width(detail, *widths, count))
    });
    // Shown default hovers without the piercing table, counted.
    let mut unpierced = 0_usize;
    for (index, entries) in out.entries.iter_mut().enumerate() {
        let side = (index % 2) as u8;
        let used = detail.sides[side as usize]
            .committed
            .iter()
            .try_fold(0_i64, |sum, unit| {
                let row = divisions.get(&(side, *unit)).ok_or_else(|| {
                    format!("committed division {unit} has no LandCombatDivisions row")
                })?;
                let width = division_width(row).ok_or_else(|| {
                    format!("committed division {unit}'s width modifiers have an eligibility callback the port does not evaluate")
                })?;
                Ok::<_, String>(sum.wrapping_add(width))
            });
        let widths = match (&width, &used) {
            (Ok(width), Ok(used)) => Ok((*width, *used)),
            (Err(reason), _) | (_, Err(reason)) => Err(reason.as_str()),
        };
        let modifiers = modifier_rows
            .rows()
            .iter()
            .find(|row| row.combat == id && row.side == side);
        let tactics = tactic_effects(
            icons,
            loc,
            detail,
            sides,
            &document.icons.combat.tactics,
            side,
        );
        for (unit, values) in entries {
            let Some(row) = divisions.get(&(side, *unit)) else {
                out.warnings.push(format!(
                    "Land combat {id} division {unit}: its LandCombatDivisions row is missing; its hovers are omitted."
                ));
                refuse_entry(values, index >= 2);
                continue;
            };
            let Some(condition) = conditions.get(unit) else {
                out.warnings.push(format!(
                    "Land combat {id}: {}; its hovers are omitted.",
                    condition_gap(*unit)
                ));
                refuse_entry(values, index >= 2);
                continue;
            };
            if index >= 2 {
                // The reserve entries are built from this recorded reserve
                // array (details.rs), so this only fails for a mismatched row.
                if !detail.sides[side as usize].reserves.contains(unit) {
                    out.warnings.push(format!(
                        "Land combat {id} division {unit}: its reserve entry is not in the recorded reserve array; its hover is omitted."
                    ));
                    refuse_entry(values, true);
                    continue;
                }
                let text = match reinforcement(row, condition, widths, land_defines) {
                    Ok(result) => {
                        let sources = if matches!(result, Reinforcement::Chance { .. }) {
                            reserve_sources(frame, document, row, &all_divisions, icons, loc)
                        } else {
                            Ok((String::new(), None))
                        };
                        match reinforcement_text(
                            icons,
                            loc,
                            &result,
                            sources.as_ref().map_err(Clone::clone),
                        ) {
                            Ok(text) => hover_text(&text),
                            Err(reason) => {
                                *refused.entry(reason).or_default() += 1;
                                None
                            }
                        }
                    }
                    Err(reason) => {
                        out.warnings.push(format!(
                            "Land combat {id} division {unit}: the reserve reinforcement hover is omitted: {reason}."
                        ));
                        None
                    }
                };
                // The reserve callback ignores the hovered child entirely.
                for key in RESERVE_HOVERS {
                    set_hover(values, key, text.clone(), None);
                }
                continue;
            }
            // Both maxima are DivisionStatus's (+0x108 HP, organisation).
            let missing = |what: &str| {
                format!("division {unit} has no DivisionStatus row, which holds its maximum {what}")
            };
            let strength = condition
                .max_strength
                .ok_or_else(|| missing("HP"))
                .and_then(|maximum| {
                    combat_strength(condition.strength, maximum, row.strength_combat_factor)
                        .ok_or_else(|| {
                            format!(
                                "division {unit}'s strength fraction is undefined for its recorded maximum"
                            )
                        })
                })
                .and_then(|strength| {
                    builtin(
                        icons,
                        loc,
                        CURR_COMB_STR,
                        &[("VALUE", Argument::Fixed5(strength))],
                    )
                });
            let strength = match strength {
                Ok(text) => hover_text(&format!("{text}\n")),
                Err(reason) => {
                    *refused.entry(reason).or_default() += 1;
                    None
                }
            };
            set_hover(values, "str_bar", strength, None);
            let organisation = condition
                .max_organisation
                .ok_or_else(|| missing("organisation"))
                .and_then(|maximum| {
                    builtin(
                        icons,
                        loc,
                        CURRENT_ORGANIZATION,
                        &[
                            ("CUR", Argument::Text(&one_decimal(condition.organisation))),
                            ("MAX", Argument::Text(&one_decimal(maximum.max(100)))),
                        ],
                    )
                });
            let organisation = match organisation {
                Ok(text) => hover_text(&format!("{text}\n")),
                Err(reason) => {
                    *refused.entry(reason).or_default() += 1;
                    None
                }
            };
            set_hover(values, "org_bar", organisation, None);
            // GetDisplayName localises the template's name key for its
            // country when the explicit name is empty; that key is not
            // recorded, so such a template's summary is refused.
            let name = match template_names.get(&row.context.template) {
                None => Err(format!(
                    "its template {} has no DivisionTemplates row",
                    row.context.template
                )),
                Some([]) => Err(format!(
                    "its template {}'s recorded name is empty, and the name key GetDisplayName would localise instead is not recorded",
                    row.context.template
                )),
                Some(name) => std::str::from_utf8(name).map_err(|_| {
                    format!(
                        "its template {}'s recorded name is not UTF-8",
                        row.context.template
                    )
                }),
            };
            let invasion =
                naval_invasion(*unit, &movements, &all_divisions, &provinces, &kinds, &wars);
            let summary = match invasion {
                Ok(true) => etas.progress(*unit).map(Some).map_err(|reason| {
                    format!(
                        "it invades from the sea, and its landing progress (140BEC9A0) cannot be rebuilt: {reason}"
                    )
                }),
                Ok(false) => Ok(None),
                Err(reason) => Err(format!(
                    "whether it invades from the sea is unknown: {reason}"
                )),
            }
            .and_then(|progress| division_summary(icons, loc, name.clone()?, row, progress, defines));
            let summary = match summary {
                Ok(text) => {
                    unpierced += usize::from(defines.is_none());
                    hover_text(&text)
                }
                Err(reason) => {
                    out.warnings.push(format!(
                        "Land combat {id} division {unit}: the default hover is omitted: {reason}."
                    ));
                    None
                }
            };
            // 1415BF4B0 dispatches only five named children specially. Every
            // other visible child of the committed row requests this summary.
            for key in SUMMARY_HOVERS {
                set_hover(values, key, summary.clone(), None);
            }
            let unit_modifiers = modifiers.and_then(|modifiers| {
                modifiers
                    .divisions
                    .iter()
                    .find(|modifier| modifier.division == *unit)
            });
            if let Some(modifiers) = unit_modifiers {
                let attack = mul(
                    mul(row.scales[2], row.scales[0]),
                    100_000_i64.wrapping_add(if side == 0 {
                        detail.attacker_bonus
                    } else {
                        detail.defender_bonus
                    }),
                );
                for (widget, key, base, scale, attacking) in [
                    (
                        "soft_attack",
                        STAT_ARMY_SOFT_ATTACK,
                        row.stat_values[3],
                        attack,
                        true,
                    ),
                    (
                        "hard_attack",
                        STAT_ARMY_HARD_ATTACK,
                        row.stat_values[4],
                        attack,
                        true,
                    ),
                    (
                        "defense",
                        if side == 0 {
                            STAT_ARMY_BREAKTHROUGH
                        } else {
                            STAT_ARMY_DEFENSE
                        },
                        row.stat_values[usize::from(side == 0)],
                        mul(row.scales[3], row.scales[1]),
                        false,
                    ),
                ] {
                    let text = combat_stat(
                        icons,
                        loc,
                        key,
                        [base, scale],
                        modifiers,
                        attacking,
                        tactics.as_deref().map_err(String::as_str),
                    );
                    // Each refusal is counted by its key or missing operand.
                    let (text, description) = match text {
                        Ok(text) => {
                            let description = builtin(icons, loc, &format!("{key}_DESC"), &[]);
                            if let Err(reason) = &description {
                                *refused.entry(reason.clone()).or_default() += 1;
                            }
                            (
                                hover_text(&text),
                                description.ok().and_then(|d| hover_text(&d)),
                            )
                        }
                        Err(reason) => {
                            *refused.entry(reason).or_default() += 1;
                            (None, None)
                        }
                    };
                    set_hover(values, widget, text, description);
                }
            } else {
                out.warnings.push(format!(
                    "Land combat {id} division {unit}: its LandCombatModifiers row is missing, so its combat-stat hovers are omitted and its first eight modifier-icon hovers show no breakdown."
                ));
                refuse_hovers(values, STAT_HOVERS);
            }
            let opponents: Result<Vec<_>, String> = detail.sides[1 - side as usize]
                .committed
                .iter()
                .map(|enemy| {
                    divisions.get(&(1 - side, *enemy)).copied().ok_or_else(|| {
                        format!(
                            "opposing committed division {enemy} has no LandCombatDivisions row"
                        )
                    })
                })
                .collect();
            let icon_context = ModifierIconContext {
                icons,
                loc,
                side,
                unit: row,
                modifiers: unit_modifiers,
                opponents,
                defines: land_defines,
                viewing_tag,
                detail,
                template_inputs: &template_inputs,
            };
            let Some(grid) = values.get_mut("modifier_icons_container/list") else {
                // details.rs builds the strip only for a division with a
                // modifier row; its absence is reported above.
                continue;
            };
            for (position, entry) in grid.entries.iter_mut().enumerate() {
                let Some(icon) = entry.get_mut("modifier_icon") else {
                    out.warnings.push(format!(
                        "Land combat {id} division {unit}: modifier icon entry {position} has no modifier_icon widget; its hover is omitted."
                    ));
                    continue;
                };
                let Some(frame) = icon.frame else {
                    out.warnings.push(format!(
                        "Land combat {id} division {unit}: modifier icon entry {position} has no frame; its hover is omitted."
                    ));
                    continue;
                };
                // Every refusal is counted per combat by its exact key or
                // missing operand, immediate and delayed alike.
                match modifier_icon_tooltip(frame, &icon_context) {
                    Ok((text, delayed)) => {
                        let delayed = match delayed {
                            Some(Ok(delayed)) => hover_text(&delayed),
                            Some(Err(reason)) => {
                                *refused
                                    .entry(format!(
                                        "modifier icon frame {frame}'s delayed text: {reason}"
                                    ))
                                    .or_default() += 1;
                                None
                            }
                            None => None,
                        };
                        set_hover(entry, "modifier_icon", hover_text(&text), delayed);
                    }
                    Err(reason) => {
                        *refused
                            .entry(format!("modifier icon frame {frame}: {reason}"))
                            .or_default() += 1;
                        set_hover(entry, "modifier_icon", None, None);
                    }
                }
            }
        }
    }
    if let Err(reason) = land_defines
        && unpierced != 0
    {
        out.warnings.push(format!("Land combat {id}: {unpierced} default division hovers omit the piercing threshold table because the land combat defines are unavailable ({reason})."));
    }
    for (key, count) in refused {
        out.warnings.push(format!(
            "Land combat {id}: {count} division hover parts are omitted: {key}."
        ));
    }
    out.warnings.sort();
    out.warnings.dedup();
    Ok(())
}

crate::combat::text::tooltip_keys! {
    BASE_VALUE = "BASE_VALUE",
    TOTAL_MODIFIER_IS = "TOTAL_MODIFIER_IS",
    SINGLE_COMBAT_VALUE = "SINGLE_COMBAT_VALUE",
    BM_LEADER_BONUS = "BM_LEADER_BONUS",
    BM_COUNTRY_BONUS = "BM_COUNTRY_BONUS",
    BM_ENCIRCLEMENT_PENALTY = "BM_ENCIRCLEMENT_PENALTY",
    BM_EXPERIENCE = "BM_EXPERIENCE",
    BM_PLANNING = "BM_PLANNING",
    BM_STACKING = "BM_STACKING",
    BM_WIDTH = "BM_WIDTH",
    BM_DUGIN_MODIFIER = "BM_DUGIN_MODIFIER",
    BM_AMPH_PENALTY = "BM_AMPH_PENALTY",
    BM_FORT_MODIFIER = "BM_FORT_MODIFIER",
    BM_RIVER_PENALTY = "BM_RIVER_PENALTY",
    BM_SHORE_BOMBARD = "BM_SHORE_BOMBARD",
    BM_RAILWAY_GUN_BOMBARD = "BM_RAILWAY_GUN_BOMBARD",
    BM_MULTIPLE = "BM_MULTIPLE",
    BM_ENEMY_AIR_SUPERIORITY = "BM_ENEMY_AIR_SUPERIORITY",
    BM_AIR_SUPPORT = "BM_AIR_SUPPORT",
    BM_PARADROP = "BM_PARADROP",
    BM_SUPPLY = "BM_SUPPLY",
    BM_DECRYPTION = "BM_DECRYPTION",
    BM_HATES_COUNTRY = "BM_HATES_COUNTRY",
    BM_HATES_COUNTRY_ON_CORE = "BM_HATES_COUNTRY_ON_CORE",
    BM_BORDER_WAR_MODIFIER = "BM_BORDER_WAR_MODIFIER",
    BM_EXILE_LEADER_DIVISION = "BM_EXILE_LEADER_DIVISION",
    BM_ACTIVE_CODE_CRACKING_BONUSES = "BM_ACTIVE_CODE_CRACKING_BONUSES",
    BM_LOCAL_BONUS = "BM_LOCAL_BONUS",
    BM_BASE_PROXIMITY = "BM_BASE_PROXIMITY",
    BM_POOR_SCREEN_PENALTY = "BM_POOR_SCREEN_PENALTY",
    BM_TERRAIN = "BM_TERRAIN",
    BM_WEATHER = "BM_WEATHER",
    BM_NIGHT_MODIFIER = "BM_NIGHT_MODIFIER",
    TACTICS_DAMAGE_ATTACKER = "TACTICS_DAMAGE_ATTACKER",
    TACTICS_ATTACKER_ORG_DAMAGE_MODIFIER = "TACTICS_ATTACKER_ORG_DAMAGE_MODIFIER",
    TACTICS_DAMAGE_DEFENDER = "TACTICS_DAMAGE_DEFENDER",
    TACTICS_DEFENDER_ORG_DAMAGE_MODIFIER = "TACTICS_DEFENDER_ORG_DAMAGE_MODIFIER",
    TACTICS_VALUE_POSITIVE = "TACTICS_VALUE_POSITIVE",
    TACTICS_VALUE_NEGATIVE = "TACTICS_VALUE_NEGATIVE",
    COMBAT_STAT = "COMBAT_STAT",
    GOOD_ARMOR_DEFENSE_DESC = "GOOD_ARMOR_DEFENSE_DESC",
    GOOD_ARMOR_ATTACK_DESC_STR = "GOOD_ARMOR_ATTACK_DESC_STR",
    GOOD_ARMOR_ATTACK_DESC_ORG = "GOOD_ARMOR_ATTACK_DESC_ORG",
    COMBAT_DIVISION_ATTACKER = "COMBAT_DIVISION_ATTACKER",
    COMBAT_DIVISION_DEFENDER = "COMBAT_DIVISION_DEFENDER",
    STAT_COMMON_ARMOR = "STAT_COMMON_ARMOR",
    HARDNESS_COMBAT_DESC = "HARDNESS_COMBAT_DESC",
    COMBAT_TOOLTIP_SEPARATOR = "COMBAT_TOOLTIP_SEPARATOR",
    STAT_COMMON_PIERCING = "STAT_COMMON_PIERCING",
    PIERCING_COMBAT_DESC = "PIERCING_COMBAT_DESC",
    PIERCING_THRESHOLD_TO_DAMAGE_LOWEST = "PIERCING_THRESHOLD_TO_DAMAGE_LOWEST",
    PIERCING_THRESHOLD_TO_DAMAGE = "PIERCING_THRESHOLD_TO_DAMAGE",
    NAVAL_INVASION_COMBAT_PROGRESS_DESC = "NAVAL_INVASION_COMBAT_PROGRESS_DESC",
    NAVAL_INVASION_PROGRESS = "NAVAL_INVASION_PROGRESS",
    NAVAL_INVASION_PROGRESS_COMPLETE = "NAVAL_INVASION_PROGRESS_COMPLETE",
    NAVAL_INVASION_ATTACK_SCALE = "NAVAL_INVASION_ATTACK_SCALE",
    NAVAL_INVASION_DEFEND_SCALE = "NAVAL_INVASION_DEFEND_SCALE",
    BM_ATTACK_VALUE = "BM_ATTACK_VALUE",
    BM_BREAKTHROUGH_VALUE = "BM_BREAKTHROUGH_VALUE",
    BM_DEFEND_VALUE = "BM_DEFEND_VALUE",
    COMBAT_OTHER_COMMANDER_DESC = "COMBAT_OTHER_COMMANDER_DESC",
    COMBAT_TRAIT_LIST_HEADER = "COMBAT_TRAIT_LIST_HEADER",
    COMBAT_DETAIL_TRAIT_SEPARATOR = "COMBAT_DETAIL_TRAIT_SEPARATOR",
    COMBAT_DETAIL_TRAIT_NAME = "COMBAT_DETAIL_TRAIT_NAME",
    DIFFERENT_COMMANDER_DESC = "DIFFERENT_COMMANDER_DESC",
    BM_ENEMY_AIR_SUPERIORITY_COUNTER = "BM_ENEMY_AIR_SUPERIORITY_COUNTER",
    BM_ENEMY_AIR_SUPERIORITY_COUNTER_DESC = "BM_ENEMY_AIR_SUPERIORITY_COUNTER_DESC",
    COMBAT_EXILES_DESC = "COMBAT_EXILES_DESC",
    COMBAT_EXILES_DESC_DELAYED = "COMBAT_EXILES_DESC_DELAYED",
    COMBAT_EXILES_ON_CORES_DESC = "COMBAT_EXILES_ON_CORES_DESC",
    LACK_OF_FUEL = "LACK_OF_FUEL",
    LACK_OF_FUEL_DESC = "LACK_OF_FUEL_DESC",
    PIERCING_ENEMIES_THRESHOLD_ENTRY = "PIERCING_ENEMIES_THRESHOLD_ENTRY",
    NON_PIERCING_THRESHOLD_ENTRY = "NON_PIERCING_THRESHOLD_ENTRY",
    PIERCING_THRESHOLD_ENTRY = "PIERCING_THRESHOLD_ENTRY",
    PIERCING_GOOD_DESC = "PIERCING_GOOD_DESC",
    COMREIC_NOT_FIGHT = "COMREIC_NOT_FIGHT",
    COMREIC_TOO_MUCH_WIDTH = "COMREIC_TOO_MUCH_WIDTH",
    BATTLE_REINFORCE_CHANCE2 = "BATTLE_REINFORCE_CHANCE2",
    MODIFIER_BASE_VALUE_PERCENTAGE = "MODIFIER_BASE_VALUE_PERCENTAGE",
    STAT_ARMY_INITIATIVE = "STAT_ARMY_INITIATIVE",
    STAT_ARMY_INITIATIVE_DIFF = "STAT_ARMY_INITIATIVE_DIFF",
    SPEED_REINFORCEMENT_BONUS = "SPEED_REINFORCEMENT_BONUS",
    CURR_COMB_STR = "CURR_COMB_STR",
    CURRENT_ORGANIZATION = "CURRENT_ORGANIZATION",
    STAT_ARMY_SOFT_ATTACK = "STAT_ARMY_SOFT_ATTACK",
    STAT_ARMY_HARD_ATTACK = "STAT_ARMY_HARD_ATTACK",
    STAT_ARMY_BREAKTHROUGH = "STAT_ARMY_BREAKTHROUGH",
    STAT_ARMY_DEFENSE = "STAT_ARMY_DEFENSE",
}

#[cfg(test)]
mod tests;
