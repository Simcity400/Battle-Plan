use content_reader_map::map::ProvinceKind;
use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::{
    rows::LandCombatDetailsRow, rows::LandCombatDivisionModifierRow, rows::LandCombatDivisionRow,
    rows::LandCombatMember, rows::LandCombatMemberValues, rows::land_combat_members,
    rows::LandCombatSideRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
use viewer_engagements::engagement_progress::sides as engagement_progress_sides;
use viewer_foundation::modifier_names::{
    BM_AMPH_PENALTY, BM_DUGIN_MODIFIER, BM_ENCIRCLEMENT_PENALTY, BM_MULTIPLE, BM_PARADROP,
    BM_RIVER_PENALTY, BM_SUPPLY, BM_TERRAIN, modifier_name_key,
};
use viewer_foundation::tooltip_text::country_localised;
use viewer_foundation::tooltip_text::localise;
use viewer_foundation::tooltip_text::tooltip_keys;
use viewer_foundation::tooltip_text::viewing_tag as tooltip_text_viewing_tag;
use super::{
    Selection,
    Values,
    binding::Bindings,
    defines::{ArmourHover, LandDefines},
    fixed::{fraction, mul},
    land_details::{direction_count, division_width, total_width},
    tactics::Content as Tactics,
    tooltips::{hover_text, organisation_effects, refuse_hovers, set_hover},
};
use viewer_engagements::land_combat_math::{
    Condition,
    Divisions,
    combat_rows,
    member_values,
    condition_gap,
    conditions,
    modifier,
};
use viewer_foundation::tooltip_text::{TextIcons, builtin, token};
use super::content::Content as StaticContent;
use viewer_foundation::perspective::Vision;
use dashboard_text::table_access::{Available, rows};

mod history;
mod panel;

fn combat_strength(strength: i64, maximum: i64, factor: i64) -> Option<i64> {
    Some(
        mul(
            fraction(strength, maximum.max(100))?.clamp(0, 100_000),
            factor,
        )
        .clamp(0, 100_000),
    )
}

fn one_decimal(value: i64) -> String {
    content_reader_parser::fixed5::fixed5_to_string(value, 1)
}

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

fn combat_stat(
    icons: &dyn TextIcons,
    loc: &Localisation,
    key: &str,
    [base, scale]: [i64; 2],
    modifiers: &LandCombatDivisionModifierRow,
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
        let modifier_key = modifier_name_key(modifier.id)?;
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

fn fixed_decimal(value: i64) -> String {
    content_reader_parser::fixed5::fixed5_to_string(value, -1)
}

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

fn recorded<'a>(row: &LandCombatMember<'a>) -> Result<&'a LandCombatMemberValues, String> {
    row.values.ok_or_else(|| {
        "the division's values are in neither its LandCombatDivisions row nor its division's rows"
            .to_owned()
    })
}

fn division_summary(
    icons: &dyn TextIcons,
    loc: &Localisation,
    name: &str,
    row: &LandCombatMember<'_>,
    invasion: Option<i64>,
    defines: Option<&LandDefines>,
) -> Result<String, String> {
    let values = recorded(row)?;
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
    text.push_str(&base_stat(icons, loc, STAT_COMMON_ARMOR, values.armour)?);
    text.push_str("\n\n");
    let hardness = values.stat_values[2];
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
    text.push_str(&base_stat(icons, loc, STAT_COMMON_PIERCING, values.piercing)?);
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

fn division_experience_tooltip(
    icons: &dyn TextIcons,
    loc: &Localisation,
    experience: content_reader_key_names::unit_leader::DivisionExperienceProgress,
) -> Result<String, String> {
    let level = builtin(icons, loc, &format!("UNIT_LEVEL_{}", experience.level), &[])?;
    let combat = content_reader_localisation::localisation::format_scripted_variable(
        experience.combat_bonus,
        Some(b"+%0"),
    );
    builtin(
        icons,
        loc,
        EXP_TOOLTIP_ARMY,
        &[
            ("NUMBER", Argument::Int32(experience.level.wrapping_add(1))),
            ("LEVEL", Argument::Text(&level)),
            ("COMBAT", Argument::Text(&combat)),
            ("EXP", Argument::Fixed5(experience.next_level_fraction)),
        ],
    )
}

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

struct Wars {
    slots: BTreeMap<u32, u32>,
    pairs: BTreeSet<(u32, u32)>,
    unlinked: bool,
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
            .filter(|row| row.type_id == engine_native::cwar_relation::TYPE && row.retired == 0)
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
    unit: LandCombatMember<'frame>,
    modifiers: Option<&'frame LandCombatDivisionModifierRow>,
    opponents: Result<Vec<LandCombatMember<'frame>>, String>,
    defines: Result<&'context LandDefines, &'context str>,
    viewing_tag: Result<&'frame str, &'static str>,
    detail: &'frame LandCombatDetailsRow,
    template_inputs: &'context super::template_icons::Inputs<'frame>,
    leaders: Result<&'context Available<'frame, tables::UnitLeaders>, &'context str>,
    leader_traits: Result<&'context Available<'frame, tables::UnitLeaderTraits>, &'context str>,
}

#[derive(Debug, PartialEq, Eq)]
struct DivisionCommander<'frame> {
    name_token: &'frame [u8],
    skill: i32,
    traits: Vec<&'frame [u8]>,
}

fn division_commander<'frame>(
    handle: u64,
    leaders: Result<&Available<'frame, tables::UnitLeaders>, &str>,
    traits: Result<&Available<'frame, tables::UnitLeaderTraits>, &str>,
) -> Result<DivisionCommander<'frame>, String> {
    let leaders = leaders.map_err(|reason| format!("UnitLeaders is unreadable ({reason})"))?;
    let leader = leaders
        .rows()
        .iter()
        .find(|row| (u64::from(row.id) << 32 | u64::from(row.reference_type)) == handle)
        .ok_or_else(|| match leaders {
            Available::Unavailable { reason } => {
                format!("UnitLeaders is unavailable this hour ({reason})")
            }
            _ => format!("the division commander {handle} has no UnitLeaders row this hour"),
        })?;
    if leader.skill_status == 0 {
        return Err(format!(
            "the division commander {handle}'s UnitLeaders row records no skill object, needed to read its skill level"
        ));
    }
    let traits = traits.map_err(|reason| format!("UnitLeaderTraits is unreadable ({reason})"))?;
    let Available::Complete(traits) = traits else {
        return Err(format!(
            "UnitLeaderTraits is incomplete or unavailable this hour, so the division commander {handle}'s traits are unknown"
        ));
    };
    let mut assigned: Vec<_> = traits
        .iter()
        .filter(|row| row.leader_reference == handle)
        .collect();
    assigned.sort_by_key(|row| row.ordinal);
    Ok(DivisionCommander {
        name_token: &leader.inherited_text_a,
        skill: leader.skill_level,
        traits: assigned
            .into_iter()
            .map(|row| row.token.as_slice())
            .collect(),
    })
}

fn commander_icon_tooltip(
    icons: &dyn TextIcons,
    loc: &Localisation,
    viewing_tag: Result<&str, &str>,
    commander: &DivisionCommander<'_>,
) -> Result<(String, Option<Result<String, String>>), String> {
    let name = token(icons, loc, commander.name_token)
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
        for (index, trait_token) in commander.traits.iter().enumerate() {
            if index != 0 {
                text.push_str(&builtin(icons, loc, COMBAT_DETAIL_TRAIT_SEPARATOR, &[])?);
            }
            let trait_token = std::str::from_utf8(trait_token).map_err(|_| {
                format!(
                    "commander trait token {} is not UTF-8",
                    String::from_utf8_lossy(trait_token)
                )
            })?;
            let name =
                super::commander_tooltips::country_localized(icons, loc, viewing_tag, trait_token)?;
            text.push_str(&builtin(
                icons,
                loc,
                COMBAT_DETAIL_TRAIT_NAME,
                &[("NAME", Argument::Text(&name))],
            )?);
        }
    }
    Ok((
        text,
        Some(builtin(icons, loc, DIFFERENT_COMMANDER_DESC, &[])),
    ))
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
        return Ok((text, None));
    }
    if frame == 12 {
        let commander = division_commander(unit.commander, context.leaders, context.leader_traits)?;
        return commander_icon_tooltip(icons, loc, viewing_tag, &commander);
    }
    if frame == 13 {
        let defines = defines?;
        let amount = engine_native_parts::csub_unit_definition::movement_air_defense_value(
            recorded(&unit)?.air_attack,
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
            .occupiers(&unit, detail, &bonuses)
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
        let [fuel, capacity] = recorded(&unit)?.fuel_inputs;
        let ratio =
            super::division_icons::fuel_ratio([fuel, capacity, defines?.fuel_penalty_start_ratio]);
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
    let unit = recorded(&unit)?;
    let mut counts = vec![0_i32; defines.piercing_thresholds.len()];
    for opponent in opponents {
        let opponent = recorded(opponent)?;
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
        }
        .ok_or("the game's armour division traps on the recorded armour and piercing")?;
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
        leader: i64,
    },
    Unrecorded(&'static str),
}

const RESERVE_SPEED_REFUSED: &str = "the reserve reinforcement speed share was refused at capture: an operand was unreadable or took a form the port does not follow (collector status land_reserve_speed)";
const SOURCES_UNREAD: &str = "the reserve chance's reinforcement-rate sources were not read";

struct ReserveSources {
    country: Result<String, String>,
    commander: Option<Result<(String, String), String>>,
}

fn reserve_tree(
    nodes: &[replay_data::rows::ModifierBreakdownNode],
    definition: &replay_data::rows::ModifierDefinitionRow,
    labels: &super::naval_labels::RecordedLabels,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<String, String> {
    super::naval_trees::breakdown(nodes, definition, 100_000, "", labels, icons, loc, true)
}

fn reserve_sources(
    frame: &DecodedFrame,
    labels: &super::naval_labels::RecordedLabels,
    row: &LandCombatDivisionRow,
    divisions: &Available<'_, tables::Divisions>,
    icons: &dyn TextIcons,
    loc: &Localisation,
    leader: i64,
) -> ReserveSources {
    let sources = (|| {
        let (trees, truncated) =
            match rows::<tables::ReserveModifierTrees>(frame).map_err(|e| e.to_string())? {
                Available::Complete(trees) => (trees, false),
                Available::Truncated { rows, .. } => (rows, true),
                Available::Unavailable { reason } => {
                    return Err(format!(
                        "ReserveModifierTrees is unavailable this hour ({reason})"
                    ));
                }
                Available::NotRecorded { mode } => {
                    return Err(format!(
                        "ReserveModifierTrees is not recorded in a {mode} recording"
                    ));
                }
            };
        let definitions = rows::<tables::ModifierDefinitions>(frame).map_err(|e| e.to_string())?;
        if !definitions.is_complete() {
            return Err("ModifierDefinitions is incomplete".to_owned());
        }
        let definition = definitions
            .rows()
            .iter()
            .find(|r| r.index == engine_executable::modifier_keys::LAND_REINFORCE_RATE)
            .ok_or("the land_reinforce_rate modifier definition is missing")?;
        Ok((trees, truncated, definition))
    })();
    let describe = |kind: u8, owner: u64, role: &str| -> Result<(&[u8], String), String> {
        let (trees, truncated, definition) = sources.as_ref().map_err(Clone::clone)?;
        let tree = trees
            .iter()
            .find(|tree| tree.kind == kind && tree.owner == owner)
            .ok_or_else(|| {
                if *truncated {
                    format!(
                        "ReserveModifierTrees was truncated this hour before the reserve's {role}"
                    )
                } else {
                    format!("ReserveModifierTrees has no row for the reserve's {role} this hour")
                }
            })?;
        let nodes = tree.nodes.as_ref().ok_or_else(|| {
            format!(
                "the reserve's {role} reinforcement-rate tree was refused at capture (collector status land_reserve_speed)"
            )
        })?;
        Ok((
            &tree.name,
            reserve_tree(nodes, definition, labels, icons, loc)?,
        ))
    };
    let country = divisions
        .rows()
        .iter()
        .find(|unit| unit.id == row.division)
        .ok_or_else(|| "a reserve has no Divisions row this hour".to_owned())
        .and_then(|unit| {
            let owner = if unit.expeditionary_owner as i32 > 0 {
                unit.expeditionary_owner
            } else {
                unit.roster_owner
            };
            describe(0, u64::from(owner), "country").map(|(_, text)| text)
        });
    let commander = (leader != 0).then(|| {
        let (name, text) = describe(row.context.leader_modifier_kind, row.commander, "commander")?;
        let name = labels.text(loc, name, "reserve commander name")?;
        Ok((name, text))
    });
    ReserveSources { country, commander }
}

const ADVANCE_STRENGTH: i64 = 10_000;
const ADVANCE_ORGANISATION: i64 = 100_000;

fn reinforcement(
    row: &LandCombatMember<'_>,
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
    let callback = |what: &str| {
        format!(
            "its {what} modifier has an eligibility callback {} the port does not evaluate",
            row.context.eligibility_callback
        )
    };
    let excess = division_width(row.row)
        .ok_or_else(|| callback("width"))?
        .wrapping_sub(available);
    if excess > 0 {
        let share = fraction(excess, width)
            .ok_or_else(|| format!("the excess width share: {}", super::fixed::TRAP))?;
        if mul(share, defines.over_width_penalty) < defines.over_width_penalty_max {
            return Ok(Reinforcement::Rejected(COMREIC_TOO_MUCH_WIDTH));
        }
    }
    let Some(share) = row.reinforcement_speed else {
        return Ok(Reinforcement::Unrecorded(RESERVE_SPEED_REFUSED));
    };
    let country = modifier(&row.context, 0, false).ok_or_else(|| callback("reinforce rate"))?;
    let leader = modifier(&row.context, 0, true).ok_or_else(|| callback("reinforce rate"))?;
    let initiative = mul(
        100_000_i64
            .wrapping_add(modifier(&row.context, 2, false).ok_or_else(|| callback("initiative"))?),
        recorded(row)?.initiative,
    );
    let initiative = if row.context.leader_modifier_kind == 0 {
        initiative
    } else {
        mul(
            100_000_i64.wrapping_add(
                modifier(&row.context, 2, true).ok_or_else(|| callback("initiative"))?,
            ),
            initiative,
        )
    };
    let initiative = mul(initiative, defines.initiative_factor);
    let speed = mul(share, defines.speed_bonus);
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
        leader,
    })
}

fn reinforcement_text(
    icons: &dyn TextIcons,
    loc: &Localisation,
    result: &Reinforcement,
    sources: Option<ReserveSources>,
) -> Result<(String, Vec<String>), String> {
    let chance = match result {
        Reinforcement::Rejected(_) => 0,
        Reinforcement::Chance { value, .. } => *value,
        Reinforcement::Unrecorded(reason) => return Err((*reason).into()),
    };
    let mut text = builtin(
        icons,
        loc,
        BATTLE_REINFORCE_CHANCE2,
        &[("VALUE", Argument::Fixed5(chance))],
    )?;
    let mut omitted = Vec::new();
    match result {
        Reinforcement::Rejected(key) => {
            text.push_str(&builtin(icons, loc, key, &[])?);
            text.push('\n');
        }
        Reinforcement::Chance {
            base,
            initiative,
            speed,
            leader,
            ..
        } => {
            text.push_str(&builtin(
                icons,
                loc,
                MODIFIER_BASE_VALUE_PERCENTAGE,
                &[("VALUE", Argument::Fixed5(*base))],
            )?);
            text.push('\n');
            let sources = sources.unwrap_or(ReserveSources {
                country: Err(SOURCES_UNREAD.into()),
                commander: (*leader != 0).then(|| Err(SOURCES_UNREAD.into())),
            });
            match sources.country {
                Ok(country) => text.push_str(&country),
                Err(reason) => omitted.push(reason),
            }
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
            match sources.commander {
                Some(Ok((name, sources))) => {
                    text.push_str(&name);
                    text.push_str(": ");
                    text.push_str(&sources);
                }
                Some(Err(reason)) => omitted.push(reason),
                None => {}
            }
        }
        Reinforcement::Unrecorded(_) => unreachable!("refused above"),
    }
    Ok((text, omitted))
}

const SUMMARY_HOVERS: [&str; 7] = [
    "Background",
    "unit_button",
    "unit_icon",
    "name",
    "soft_attack_icon",
    "hard_attack_icon",
    "breakthrough_icon",
];
const STAT_HOVERS: [&str; 3] = ["soft_attack", "hard_attack", "defense"];
const RESERVE_HOVERS: [&str; 6] = [
    "Background",
    "unit_button",
    "unit_icon",
    "name",
    "org_bar",
    "str_bar",
];

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
    static_content: &StaticContent,
    frame: &DecodedFrame,
    selection: Selection,
    vision: &Vision,
) -> io::Result<()> {
    let Selection::Land(id) = selection else {
        return Ok(());
    };
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
    let sides = engagement_progress_sides(frame)?;
    let sides = sides.get(&id).copied();
    let division_rows = rows::<tables::LandCombatDivisions>(frame)?;
    let combat_members = combat_rows(division_rows.rows(), id);
    let member_values = member_values(frame, combat_members)?;
    let divisions: Divisions<'_> = land_combat_members(combat_members, &member_values)
        .map(|row| ((row.side, row.division), row))
        .collect();
    let member_rows = rows::<tables::LandCombatDivisionModifiers>(frame)?;
    let members = super::details::division_modifiers(member_rows.rows(), combat_members, id);
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
    let leaders = rows::<tables::UnitLeaders>(frame).map_err(|error| error.to_string());
    let leader_traits = rows::<tables::UnitLeaderTraits>(frame).map_err(|error| error.to_string());
    let etas = static_content
        .eta_cache
        .etas(frame, static_content.movement.as_deref())?;
    let conditions = conditions(frame)?;
    let condition_gap = condition_gap(frame)?;
    let land_defines = static_content
        .assets
        .land_defines
        .as_ref()
        .map_err(String::as_str);
    let defines = land_defines.ok();
    let map = &static_content.map;
    let kinds = |province| map.definitions.get(&province).map(|row| row.kind);
    let countries = rows::<tables::Countries>(frame)?;
    let viewing_tag = tooltip_text_viewing_tag(countries.rows(), vision);
    let (icons, loc) = (static_content.assets.as_ref(), &static_content.localisation);
    let movement_sources: BTreeMap<_, _> = movements
        .rows()
        .iter()
        .map(|row| (row.division, row.source))
        .collect();
    let width = direction_count(
        &detail.sides[0].committed,
        &divisions,
        detail.province,
        &movement_sources,
    )
    .map_err(|reason| format!("the attack direction count is unknown: {reason}"))
    .and_then(|count| {
        let terrain = static_content
            .terrain
            .combat_terrain
            .get(&detail.province)
            .ok_or_else(|| {
                format!(
                    "province {} has no terrain category in the content",
                    detail.province
                )
            })?;
        let widths = static_content
            .terrain
            .combat_widths(terrain)
            .ok_or_else(|| format!("terrain {terrain} is not a content terrain category"))?;
        Ok(total_width(detail, *widths, count))
    });
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
                let width = division_width(row.row).ok_or_else(|| {
                    format!("committed division {unit}'s width modifiers have an eligibility callback the port does not evaluate")
                })?;
                Ok::<_, String>(sum.wrapping_add(width))
            });
        let widths = match (&width, &used) {
            (Ok(width), Ok(used)) => Ok((*width, *used)),
            (Err(reason), _) | (_, Err(reason)) => Err(reason.as_str()),
        };
        let tactics = tactic_effects(
            icons,
            loc,
            detail,
            sides,
            &static_content.assets.tactics,
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
                if !detail.sides[side as usize].reserves.contains(unit) {
                    out.warnings.push(format!(
                        "Land combat {id} division {unit}: its reserve entry is not in the recorded reserve array; its hover is omitted."
                    ));
                    refuse_entry(values, true);
                    continue;
                }
                let text = match reinforcement(row, condition, widths, land_defines) {
                    Ok(result) => match reinforcement_text(icons, loc, &result, {
                        match &result {
                            Reinforcement::Chance { leader, .. } => Some(reserve_sources(
                                frame,
                                &static_content.recorded_labels,
                                row.row,
                                &all_divisions,
                                icons,
                                loc,
                                *leader,
                            )),
                            _ => None,
                        }
                    }) {
                        Ok((text, omitted)) => {
                            for reason in omitted {
                                *refused.entry(reason).or_default() += 1;
                            }
                            hover_text(&text)
                        }
                        Err(reason) => {
                            *refused.entry(reason).or_default() += 1;
                            None
                        }
                    },
                    Err(reason) => {
                        out.warnings.push(format!(
                            "Land combat {id} division {unit}: the reserve reinforcement hover is omitted: {reason}."
                        ));
                        None
                    }
                };
                for key in RESERVE_HOVERS {
                    set_hover(values, key, text.clone(), None);
                }
                continue;
            }
            let missing = |what: &str| {
                format!("division {unit} has no maximum {what} in DivisionPanelStats")
            };
            let strength = condition
                .max_strength
                .ok_or_else(|| missing("HP"))
                .and_then(|maximum| {
                    let factor = recorded(row)?.strength_combat_factor;
                    combat_strength(condition.strength, maximum, factor)
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
            let name = template_display_name(loc, frame, row.context.template);
            let invasion =
                naval_invasion(*unit, &movements, &all_divisions, &provinces, &kinds, &wars);
            let summary = match invasion {
                Ok(true) => etas.progress(*unit).map(Some).map_err(|reason| {
                    format!(
                        "it invades from the sea, and its landing progress cannot be rebuilt: {reason}"
                    )
                }),
                Ok(false) => Ok(None),
                Err(reason) => Err(format!(
                    "whether it invades from the sea is unknown: {reason}"
                )),
            }
            .and_then(|progress| division_summary(icons, loc, &name?, row, progress, defines));
            let summary = match summary {
                Ok(text) => {
                    unpierced += usize::from(defines.is_none());
                    hover_text(&text)
                }
                Err(reason) => {
                    *refused
                        .entry(format!("default division hover: {reason}"))
                        .or_default() += 1;
                    None
                }
            };
            for key in SUMMARY_HOVERS {
                set_hover(values, key, summary.clone(), None);
            }
            let unit_modifiers = members.get(&(side, *unit)).copied();
            let numbers = row.values.map(|values| (row.scales, values.stat_values));
            if let (Some(modifiers), Some((scales, stats))) = (unit_modifiers, numbers) {
                let attack = mul(
                    mul(scales[2], scales[0]),
                    100_000_i64.wrapping_add(if side == 0 {
                        detail.attacker_bonus
                    } else {
                        detail.defender_bonus
                    }),
                );
                for (widget, key, base, scale, attacking) in [
                    ("soft_attack", STAT_ARMY_SOFT_ATTACK, stats[3], attack, true),
                    ("hard_attack", STAT_ARMY_HARD_ATTACK, stats[4], attack, true),
                    (
                        "defense",
                        if side == 0 {
                            STAT_ARMY_BREAKTHROUGH
                        } else {
                            STAT_ARMY_DEFENSE
                        },
                        stats[usize::from(side == 0)],
                        mul(scales[3], scales[1]),
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
            } else if unit_modifiers.is_some() {
                out.warnings.push(format!(
                    "Land combat {id} division {unit}: its stats are in neither its LandCombatDivisions row nor its division's rows, so its combat-stat hovers are omitted."
                ));
                refuse_hovers(values, STAT_HOVERS);
            } else {
                out.warnings.push(format!(
                    "Land combat {id} division {unit}: its LandCombatDivisionModifiers row is missing, so its combat-stat hovers are omitted and its first eight modifier-icon hovers show no breakdown."
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
                unit: *row,
                modifiers: unit_modifiers,
                opponents,
                defines: land_defines,
                viewing_tag,
                detail,
                template_inputs: &template_inputs,
                leaders: leaders.as_ref().map_err(String::as_str),
                leader_traits: leader_traits.as_ref().map_err(String::as_str),
            };
            let Some(grid) = values.get_mut("modifier_icons_container/list") else {
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

tooltip_keys! {
    BASE_VALUE = "BASE_VALUE",
    TOTAL_MODIFIER_IS = "TOTAL_MODIFIER_IS",
    SINGLE_COMBAT_VALUE = "SINGLE_COMBAT_VALUE",
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
    EXP_TOOLTIP_ARMY = "EXP_TOOLTIP_ARMY",
}

#[cfg(test)]
mod tests;

pub fn division_panel(
    content: &StaticContent,
    frame: &DecodedFrame,
    id: u64,
) -> viewer_foundation::tooltip_text::Tooltip {
    let mut out = viewer_foundation::tooltip_text::Tooltip::default();
    for (name, result) in [
        ("division stats", panel::text(content, frame, id)),
        ("division career", history::text(content, frame, id)),
    ] {
        match result {
            Ok((text, reasons)) => {
                out.line(true, 2, name, Ok(text));
                for reason in reasons {
                    out.refuse(name, reason);
                }
            }
            Err(reason) => out.refuse(name, reason),
        }
    }
    let experience = (|| {
        let status = frame
            .complete::<tables::DivisionStatus>()
            .map_err(|why| why.to_string())?;
        let status = status
            .binary_search_by_key(&id, |row| row.division)
            .ok()
            .map(|at| &status[at])
            .ok_or("division status unavailable")?;
        let inputs = frame
            .complete::<tables::DivisionLandInputs>()
            .map_err(|why| why.to_string())?;
        let inputs = inputs
            .binary_search_by_key(&id, |row| row.division)
            .ok()
            .map(|at| &inputs[at])
            .ok_or("division manpower unavailable")?;
        let xp = status
            .normalized_experience(
                inputs
                    .manpower
                    .iter()
                    .fold(0_i32, |sum, entry| sum.wrapping_add(entry.amount)),
            )
            .ok_or("division experience unavailable")?;
        let defines = content.assets.land_defines.as_ref().map_err(Clone::clone)?;
        let experience = defines.experience.as_ref().ok_or_else(|| {
            defines
                .experience_refusal
                .clone()
                .unwrap_or("division experience defines unavailable".into())
        })?;
        division_experience_tooltip(
            content.assets.as_ref(),
            &content.localisation,
            experience.progress(xp),
        )
    })();
    out.line(true, 2, "division experience", experience);
    let transition = (|| {
        let statuses = frame
            .complete::<tables::DivisionStatus>()
            .map_err(|why| why.to_string())?;
        let status = statuses
            .binary_search_by_key(&id, |row| row.division)
            .ok()
            .map(|at| &statuses[at])
            .ok_or("division status unavailable")?;
        let old = status
            .pending_template
            .require("DivisionStatus.pending_template")?
            .ok_or("division pending template unavailable")?;
        if old == 0 {
            return Ok(String::new());
        }
        let units = frame
            .complete::<tables::Divisions>()
            .map_err(|why| why.to_string())?;
        let unit = units
            .binary_search_by_key(&id, |row| row.id)
            .ok()
            .map(|at| &units[at])
            .ok_or("division unavailable")?;
        let name = |handle| template_display_name(&content.localisation, frame, handle);
        builtin(
            content.assets.as_ref(),
            &content.localisation,
            "DIVISION_IN_TEMPLATE_TRANSFER",
            &[
                ("OLD", Argument::Text(&name(old)?)),
                ("NEW", Argument::Text(&name(unit.template)?)),
            ],
        )
    })();
    out.line(true, 2, "division conversion", transition);
    out
}

fn template_display_name(
    loc: &Localisation,
    frame: &DecodedFrame,
    handle: u64,
) -> Result<String, String> {
    let templates = rows::<tables::DivisionTemplates>(frame).map_err(|why| why.to_string())?;
    let templates = templates.rows();
    let countries = rows::<tables::Countries>(frame).map_err(|why| why.to_string())?;
    let countries = countries.rows();
    let template = templates
        .iter()
        .find(|template| template.template == handle)
        .ok_or_else(|| format!("template {} has no DivisionTemplates row", handle))?;
    if !template.name.is_empty() {
        return std::str::from_utf8(&template.name)
            .map(str::to_owned)
            .map_err(|_| "template name is not UTF-8".to_owned());
    }
    let key = template.name_key.require("DivisionTemplates.name_key")?;
    if key.is_empty() {
        return Ok(String::new());
    }
    let key = std::str::from_utf8(key).map_err(|_| "template name_key is not UTF-8")?;
    let template_icons = rows::<tables::DivisionTemplateIcons>(frame).map_err(|e| e.to_string())?;
    let icon = template_icons
        .rows()
        .iter()
        .find(|icon| icon.template == template.template)
        .ok_or("template localisation country unavailable")?;
    if icon.country <= 0 {
        return localise(loc, key, &[]);
    }
    let links = rows::<tables::CountryLinks>(frame).map_err(|e| e.to_string())?;
    let slot = links
        .rows()
        .iter()
        .find(|link| link.id == icon.country as u32)
        .ok_or("template country link unavailable")?
        .canonical_slot;
    let tag = countries
        .iter()
        .find(|country| country.canonical_slot == slot)
        .ok_or("template country unavailable")?;
    let tag = std::str::from_utf8(&tag.tag).map_err(|_| "template country tag is not UTF-8")?;
    country_localised(loc, Ok(tag), key)
}
