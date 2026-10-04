//! Naval combat box hovers. CFEXMember's cached labels are stored text and
//! can be drawn only in the recording language.
use super::{
    INLINE_FLAG,
    binding::{Bindings, TextIcons, builtin, token},
    tooltips::{hover_text, set_hover},
};
use crate::{
    document::Document,
    table_access::{TokenNames, rows},
};
use installed_content::localisation::{Argument, Localisation};
use replay::{
    frame::DecodedFrame,
    rows::{
        ModifierDefinitionRow, NavalCombatDetailsRow, NavalCombatShipRow, VariantDefinitionRow,
        tables,
    },
};
use std::{collections::BTreeMap, io};

struct VariantContext<'a> {
    document: &'a Document,
    frame: &'a DecodedFrame,
    tokens: Result<TokenNames<'a>, String>,
    countries: BTreeMap<u32, &'a replay::rows::CountryRow>,
    appearances: Result<BTreeMap<u32, &'a replay::rows::CountryAppearanceRow>, String>,
    escape: EscapeContext<'a>,
}

struct EscapeContext<'a> {
    details: Result<&'a [NavalCombatDetailsRow], String>,
    definitions: Result<&'a [ModifierDefinitionRow], String>,
}

// Executable localisation keys used by 1417D3220, 1417DBBE0, 1415B3FB0,
// 14195F760 (descriptor table 142A0A460) and 1415B43F0.
super::text::tooltip_keys! {
    NAVAL_COMBAT_INCOMING_UNKNOWN = "NAVAL_COMBAT_INCOMING_UNKNOWN",
    SHIP_AND_TYPE = "SHIP_AND_TYPE",
    SHIP_SUNK_BY = "SHIP_SUNK_BY",
    FEX_STATE_UNINITIALIZED = "FEX_STATE_UNINITIALIZED",
    FEX_STATE_WAITING = "FEX_STATE_WAITING",
    FEX_STATE_FIGHTING = "FEX_STATE_FIGHTING",
    FEX_STATE_ON_THE_WAY = "FEX_STATE_ON_THE_WAY",
    FEX_STATE_ESCAPING = "FEX_STATE_ESCAPING",
    FEX_STATE_RUN_AWAY = "FEX_STATE_RUN_AWAY",
    FEX_STATE_SUNK = "FEX_STATE_SUNK",
    NAVAL_COMBAT_SHIP_TOOLTIP = "NAVAL_COMBAT_SHIP_TOOLTIP",
    NAVAL_COMBAT_SHIP_STRENGTH = "NAVAL_COMBAT_SHIP_STRENGTH",
    NAVAL_COMBAT_SHIP_ORGANIZATION_FACTOR = "NAVAL_COMBAT_SHIP_ORGANIZATION_FACTOR",
    ESCAPE_PROGRESS = "ESCAPE_PROGRESS",
    CHANCE_TO_ESCAPE = "CHANCE_TO_ESCAPE",
    BASE_ESCAPE_SPEED = "BASE_ESCAPE_SPEED",
    ESCAPE_SPEED_COMBAT_DURATION = "ESCAPE_SPEED_COMBAT_DURATION",
    ESCAPE_SPEED_FROM_SHIP_SPEED = "ESCAPE_SPEED_FROM_SHIP_SPEED",
    HIDDEN_SUBS_ESCAPE_BONUS = "HIDDEN_SUBS_ESCAPE_BONUS",
    SUBS_ESCAPE_BONUS = "SUBS_ESCAPE_BONUS",
    SCREENING_VALUE = "SCREENING_VALUE",
    NAVAL_COMBAT_HOURS_TO_ARRIVE = "NAVAL_COMBAT_HOURS_TO_ARRIVE",
    NAVAL_COMBAT_RECEIVED_CRITICALS = "NAVAL_COMBAT_RECEIVED_CRITICALS",
    CARRIER_PLANES_EVAC_STATUS_DESTROYED = "CARRIER_PLANES_EVAC_STATUS_DESTROYED",
    CARRUER_PLANES_EVAC_STATUS_STATE = "CARRUER_PLANES_EVAC_STATUS_STATE",
    TOTAL_DAMAGE_RECEIVED = "total_damage_received",
    DAMAGE_DEALT_LIGHT_GUNS = "damage_dealt_light_guns",
    DAMAGE_DEALT_HEAVY_GUNS = "damage_dealt_heavy_guns",
    DAMAGE_DEALT_TORPEDOES = "damage_dealt_torpedoes",
    DAMAGE_DEALT_DEPTH_CHARGES = "damage_dealt_depth_charges",
    DAMAGE_DEALT_AIR = "damage_dealt_air",
    NAVAL_COMBAT_AIRWING_TOOLTIP_SHOT_DOWN = "NAVAL_COMBAT_AIRWING_TOOLTIP_SHOT_DOWN",
    NAVAL_COMBAT_AIRWING_TOOLTIP_SHOT_DOWN_ENTRY = "NAVAL_COMBAT_AIRWING_TOOLTIP_SHOT_DOWN_ENTRY",
}

/// 1417DBBE0 builds the group icon hover from its members in native order,
/// appending a newline after each 1417D3220 member string. An incoming box
/// whose owner is hidden gets the single executable key instead.
pub(super) fn bind(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    id: u64,
) -> io::Result<()> {
    let ships = rows::<tables::NavalCombatShips>(frame)?;
    let variants = rows::<tables::VariantDefinitions>(frame)?;
    let countries = rows::<tables::Countries>(frame)?;
    let by_slot: BTreeMap<_, _> = ships
        .rows()
        .iter()
        .filter(|row| row.combat == id)
        .map(|row| ((row.side, row.battle_slot), row))
        .collect();
    let by_variant: BTreeMap<_, _> = variants.rows().iter().map(|row| (row.id, row)).collect();
    let by_country: BTreeMap<_, _> = countries.rows().iter().map(|row| (row.id, row)).collect();
    let context = VariantContext {
        document,
        frame,
        tokens: TokenNames::new(frame).map_err(|error| error.to_string()),
        countries: by_country.clone(),
        appearances: rows::<tables::CountryAppearance>(frame)
            .map(|rows| rows.rows().iter().map(|row| (row.country, row)).collect())
            .map_err(|error| error.to_string()),
        escape: EscapeContext {
            details: rows::<tables::NavalCombatDetails>(frame)
                .map_err(|error| error.to_string())
                .and_then(|read| {
                    if read.is_complete() {
                        Ok(read.rows())
                    } else {
                        Err("NavalCombatDetails is incomplete".into())
                    }
                }),
            definitions: rows::<tables::ModifierDefinitions>(frame)
                .map_err(|error| error.to_string())
                .and_then(|read| {
                    if read.is_complete() {
                        Ok(read.rows())
                    } else {
                        Err("ModifierDefinitions is incomplete".into())
                    }
                }),
        },
    };
    let icons = &document.icons.combat;
    let loc = &document.names.localisation;
    let mut refusals = Vec::new();
    for entry in &mut out.naval_entries {
        if entry.battle_slots.is_empty() {
            continue; // Air-group hover ownership is reported by naval_details.
        }
        let side = entry.side;
        let built = if entry.unknown {
            // Executable key at 1417DBBE0, resolved through the content stack.
            builtin(icons, loc, NAVAL_COMBAT_INCOMING_UNKNOWN, &[]).map_err(|reason| vec![reason])
        } else {
            group_tooltip(
                &entry.battle_slots,
                side,
                &by_slot,
                &by_variant,
                &by_country,
                icons,
                loc,
                Some(&context),
            )
        };
        match built {
            Ok(text) => {
                set_hover(&mut entry.values, "icon", hover_text(&text), None);
                if !entry.unknown {
                    for slot in &entry.battle_slots {
                        if let Some(row) = by_slot.get(&(side, *slot)).filter(|row| row.state == 8)
                        {
                            refusals.push(escape_breakdown_refusal(row));
                        }
                    }
                }
            }
            Err(reasons) => {
                set_hover(&mut entry.values, "icon", None, None);
                for reason in reasons {
                    refusals.push(format!(
                        "Naval combat {id} ship box hover is withheld: {reason}."
                    ));
                }
            }
        }
    }
    for refusal in refusals {
        out.refuse(refusal);
    }
    Ok(())
}

fn escape_breakdown_refusal(row: &NavalCombatShipRow) -> String {
    let missing = if row.live_ship != 0 {
        "country, ship, doctrine, critical and HQ modifier breakdown"
    } else {
        "convoy escape chance and country/weather/terrain/commander modifier breakdown"
    };
    format!(
        "Naval combat {} side {} slot {} {missing} is pending capture-on-change (14195C7D0, 14195EEC0, 14054D0F0).",
        row.combat, row.side, row.battle_slot
    )
}

#[allow(clippy::too_many_arguments)] // The group's recorded rows and text resources.
fn group_tooltip(
    slots: &[u32],
    side: u8,
    ships: &BTreeMap<(u8, u32), &NavalCombatShipRow>,
    variants: &BTreeMap<u64, &VariantDefinitionRow>,
    countries: &BTreeMap<u32, &replay::rows::CountryRow>,
    icons: &dyn TextIcons,
    loc: &Localisation,
    context: Option<&VariantContext<'_>>,
) -> Result<String, Vec<String>> {
    let mut text = String::new();
    let mut refusals = Vec::new();
    for slot in slots {
        let part = (|| {
            let row = ships
                .get(&(side, *slot))
                .ok_or_else(|| format!("NavalCombatShips side {side} slot {slot} is absent"))?;
            let country = countries
                .get(&row.owner)
                .ok_or_else(|| format!("country {} has no Countries row", row.owner))?;
            let tag = std::str::from_utf8(&country.tag)
                .map_err(|_| format!("country {} tag is not UTF-8", row.owner))?;
            member_tooltip(
                row,
                tag,
                variants,
                icons,
                loc,
                context,
                context.map(|context| context.document.recorded_language.as_str()),
            )
        })();
        match part {
            Ok(part) => {
                text.push_str(&part);
                text.push('\n');
            }
            Err(reason) => refusals.push(reason),
        }
    }
    if refusals.is_empty() {
        Ok(text)
    } else {
        Err(refusals)
    }
}

/// 140BCB030 returns a nonempty saved CEquipmentVariant+0x28 name verbatim.
/// Otherwise it localises its effective definition's +0x18 key for origin
/// (or creator) country, through the stacked equipment and recorded lexicon.
/// Finally it appends +0x48 only when +0x24 permits it.
fn variant_name(
    row: &VariantDefinitionRow,
    icons: &dyn TextIcons,
    loc: &Localisation,
    context: Option<&VariantContext<'_>>,
) -> Result<String, String> {
    let mut name = if !row.display_name.is_empty() {
        std::str::from_utf8(&row.display_name)
            .map_err(|_| format!("variant {} explicit name is not UTF-8", row.id))?
            .to_owned()
    } else {
        let context = context.ok_or_else(|| {
            format!(
                "variant {} has no context to localise its equipment definition",
                row.id
            )
        })?;
        let key = context
            .tokens
            .as_ref()
            .map_err(Clone::clone)?
            .name(row.equipment_token)
            .map_err(|error| error.to_string())?;
        let database = context
            .document
            .icons
            .combat
            .naval_air_groups
            .equipment()
            .ok_or("the stacked equipment database is unavailable")?;
        let index = database.get(key.as_bytes()).ok_or_else(|| {
            format!(
                "variant {} equipment key {key} is absent from stacked content",
                row.id
            )
        })?;
        let raw = std::str::from_utf8(&database.equipment[index].name)
            .map_err(|_| format!("variant {} equipment key is not UTF-8", row.id))?;
        let country = if row.origin_link > 0 {
            row.origin_link
        } else {
            row.creator_link
        };
        let tag =
            if country == 0 {
                // CCountryDatabase ctor 14070E670 installs this null-country tag.
                "---"
            } else {
                let linked = context.countries.get(&country).copied().ok_or_else(|| {
                    format!("variant {} country {country} has no Countries row", row.id)
                })?;
                let appearance = context.appearances.as_ref().map_err(Clone::clone)?
                .get(&country).copied()
                .ok_or_else(|| {
                    format!(
                        "variant {} country {country} has no CountryAppearance original-tag row",
                        row.id
                    )
                })?;
                let tag = if appearance.name_tag.is_empty() {
                    &linked.tag
                } else {
                    &appearance.name_tag
                };
                std::str::from_utf8(tag)
                    .map_err(|_| format!("variant {} country {country} tag is not UTF-8", row.id))?
            };
        let localised = super::text::country_localised(loc, Ok(tag), raw)?;
        super::binding::displayable(icons, localised)?
    };
    if row.append_variant_name != 0 && !row.variant_name.is_empty() {
        name.push(' ');
        name.push_str(
            std::str::from_utf8(&row.variant_name)
                .map_err(|_| format!("variant {} suffix is not UTF-8", row.id))?,
        );
    }
    Ok(name)
}

/// 1417D3220 calls 14195FAC0 on CFEXMember's cached +0x48 name and +0x68
/// type. The strings can be rendered under PLAN #164's stored-text rule.
fn member_tooltip(
    row: &NavalCombatShipRow,
    tag: &str,
    variants: &BTreeMap<u64, &VariantDefinitionRow>,
    icons: &dyn TextIcons,
    loc: &Localisation,
    context: Option<&VariantContext<'_>>,
    recorded_language: Option<&str>,
) -> Result<String, String> {
    let (name, kind) = if row.ship != 0 || row.live_ship != 0 {
        let language = recorded_language.ok_or_else(|| {
            format!(
                "side {} slot {} has no recording language for cached ship text",
                row.side, row.battle_slot
            )
        })?;
        let label = |bytes: &[u8], field: &str| {
            dashboard::scripted_text::stored_text(
                loc,
                language,
                bytes,
                &format!(
                    "side {} slot {} CFEXMember {field}",
                    row.side, row.battle_slot
                ),
            )
        };
        (
            label(&row.name, "name +0x48")?,
            label(&row.type_name, "type +0x68")?,
        )
    } else {
        let variant = variants.get(&row.variant).ok_or_else(|| {
            format!(
                "side {} slot {} variant {} has no VariantDefinitions row",
                row.side, row.battle_slot, row.variant
            )
        })?;
        (String::new(), variant_name(variant, icons, loc, context)?)
    };
    let title = builtin(
        icons,
        loc,
        SHIP_AND_TYPE,
        &[
            ("SHIP", Argument::Text(&name)),
            ("TYPE", Argument::Text(&kind)),
        ],
    )?;
    let flag = format!("{INLINE_FLAG}{tag}");
    if row.state == 32 && row.live_ship == 0 && row.live_convoy == 0 {
        // 1417D3220 starts a sunk member with flag, one space, green
        // SHIP_AND_TYPE, reset, then a newline before sinking status.
        let mut text = format!("{flag} \u{11}G{title}\u{11}!\n");
        if row.sunk_by.is_empty() {
            text.push_str(&builtin(icons, loc, FEX_STATE_SUNK, &[])?);
        } else {
            let language = recorded_language.ok_or_else(|| {
                format!(
                    "side {} slot {} has no recording language for sinking attribution",
                    row.side, row.battle_slot
                )
            })?;
            let sinking = dashboard::scripted_text::stored_text(
                loc,
                language,
                &row.sunk_by,
                &format!(
                    "side {} slot {} CFEXMember sinking attribution +0xA8",
                    row.side, row.battle_slot
                ),
            )?;
            // 1417D3220 uses SHIP_SUNK_BY with its executable NAME argument.
            text.push_str(&builtin(
                icons,
                loc,
                SHIP_SUNK_BY,
                &[("NAME", Argument::Text(&sinking))],
            )?);
        }
        criticals_and_damage(&mut text, row, icons, loc)?;
        if row.evacuation_state == -1 {
            text.push('\n');
            text.push_str(&builtin(
                icons,
                loc,
                CARRIER_PLANES_EVAC_STATUS_DESTROYED,
                &[],
            )?);
        } else if row.evacuation_state > 0 {
            let context = context.ok_or_else(|| {
                format!(
                    "side {} slot {} has no context to name evacuation state {}",
                    row.side, row.battle_slot, row.evacuation_state
                )
            })?;
            let (state, country) = evacuation_names(
                context.document,
                context.frame,
                row.evacuation_state as u32,
                icons,
                loc,
            )?;
            text.push('\n');
            text.push_str(&builtin(
                icons,
                loc,
                CARRUER_PLANES_EVAC_STATUS_STATE,
                &[
                    ("STATE", Argument::Text(&state)),
                    ("COUNTRY", Argument::Text(&country)),
                ],
            )?);
        }
        air_hits(&mut text, row, variants, icons, loc, context)?;
        return Ok(text);
    }
    if row.live_convoy == 0 && row.live_ship == 0 {
        return Err(format!(
            "side {} slot {} has no live convoy for state {}",
            row.side, row.battle_slot, row.state
        ));
    }
    // 1415B3FB0's enum dispatch. State 0 also has a native string, although
    // the combat-box binder presently draws only 1/2/4/8/16/32.
    let state_key = match row.state {
        0 => FEX_STATE_UNINITIALIZED,
        1 => FEX_STATE_WAITING,
        2 => FEX_STATE_FIGHTING,
        4 => FEX_STATE_ON_THE_WAY,
        8 => FEX_STATE_ESCAPING,
        16 => FEX_STATE_RUN_AWAY,
        32 => FEX_STATE_SUNK,
        _ => {
            return Err(format!(
                "side {} slot {} has unsupported state {}",
                row.side, row.battle_slot, row.state
            ));
        }
    };
    let status = builtin(icons, loc, state_key, &[])?;
    let mut text = builtin(
        icons,
        loc,
        NAVAL_COMBAT_SHIP_TOOLTIP,
        &[
            ("FLAG", Argument::Text(&flag)),
            ("NAME", Argument::Text(&title)),
            ("STATUS", Argument::Text(&status)),
        ],
    )?;
    let strength = ratio(row.strength, row.maximum_hit_points.max(100));
    let strength_text = builtin(
        icons,
        loc,
        NAVAL_COMBAT_SHIP_STRENGTH,
        &[("STR", Argument::Fixed5(strength))],
    )?;
    let colour = if strength < 33_001 {
        'R'
    } else if strength < 66_001 {
        'Y'
    } else {
        'W'
    };
    text.push_str(&format!("\u{11}{colour}{strength_text}\u{11}!"));
    let maximum = super::naval_details::organisation_maximum(row)?;
    let organisation = ratio(row.organisation, maximum);
    text.push_str(&builtin(
        icons,
        loc,
        NAVAL_COMBAT_SHIP_ORGANIZATION_FACTOR,
        &[("ORG", Argument::Fixed5(organisation))],
    )?);
    if row.state == 4 {
        text.push_str(&builtin(
            icons,
            loc,
            NAVAL_COMBAT_HOURS_TO_ARRIVE,
            &[("NUM", Argument::Int32(row.arrival_hours))],
        )?);
    }
    if row.state == 8 {
        let context = context.ok_or_else(|| {
            format!(
                "side {} slot {} has no naval escape tooltip context",
                row.side, row.battle_slot,
            )
        })?;
        text.push_str(&escape_description(row, icons, loc, context)?);
    }
    criticals_and_damage(&mut text, row, icons, loc)?;
    air_hits(&mut text, row, variants, icons, loc, context)?;
    Ok(text)
}

/// 1417D3220 appends the stored escape progress, the on-demand 14195C7D0
/// chance where its source terms are recorded. Numeric defines come from the
/// content stack; child descriptions are pending capture-on-change.
fn escape_description(
    row: &NavalCombatShipRow,
    icons: &dyn TextIcons,
    loc: &Localisation,
    context: &VariantContext<'_>,
) -> Result<String, String> {
    let details = context.escape.details.as_ref().map_err(Clone::clone)?;
    let combat = details
        .iter()
        .find(|detail| detail.combat == row.combat)
        .ok_or_else(|| format!("naval combat {} has no detail row for escape", row.combat))?;
    let side = combat
        .sides
        .get(row.side as usize)
        .ok_or_else(|| format!("naval combat {} has no side {}", row.combat, row.side))?;
    let definitions = if row.live_ship != 0 {
        context
            .escape
            .definitions
            .as_ref()
            .map_err(Clone::clone)?
            .iter()
            .map(|definition| (definition.index, definition))
            .collect()
    } else {
        BTreeMap::new()
    };
    let assets = &context.document.icons.combat;
    let defines = assets.naval_escape_defines.as_ref().map_err(Clone::clone)?;
    let keys: &[u32] = if row.live_ship != 0 {
        if row.formation_flags & (1 << 7) != 0 {
            &[0x5c]
        } else if combat.duration > defines.initial_duration {
            &[0x5b, 0x59]
        } else {
            &[0x5b]
        }
    } else if row.live_convoy != 0 {
        &[0x5d]
    } else {
        return Err(format!(
            "side {} slot {} state-8 has no live ship or convoy",
            row.side, row.battle_slot
        ));
    };
    let mut modifiers = 0_i64;
    let mut by_key = Vec::new();
    for &key in keys.iter().filter(|_| row.live_ship != 0) {
        let (total, screening) = super::naval_escape::modifier_value(
            row,
            side,
            key,
            &assets.naval_escape_screening,
            &definitions,
        )?;
        modifiers = modifiers.wrapping_add(total);
        by_key.push((key, screening));
    }
    let speed_modifier = if row.live_ship != 0 {
        super::naval_environment::modifier_default_leader(
            &row.weapon_context,
            engine::modifier_keys::NAVAL_SPEED_FACTOR,
        )
        .ok_or_else(|| {
            format!(
                "side {} slot {} naval speed modifier sources are unsupported",
                row.side, row.battle_slot
            )
        })?
    } else {
        0
    };
    let parts =
        super::naval_escape::chance(row, combat.duration, modifiers, speed_modifier, defines);
    escape_text(row, parts, &by_key, row.live_ship != 0, icons, loc)
}

/// 14195C7D0's recorded state-8 lines in native order. A convoy's modifier
/// roots were supplied only by the retired tree capture, so its chance waits.
fn escape_text(
    row: &NavalCombatShipRow,
    parts: super::naval_escape::Parts,
    by_key: &[(u32, i64)],
    chance_supported: bool,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<String, String> {
    let mut out = String::new();
    out.push('\n');
    out.push_str(&builtin(
        icons,
        loc,
        ESCAPE_PROGRESS,
        &[("NUM", Argument::Fixed5(row.escape_progress))],
    )?);
    out.push('\n');
    if chance_supported {
        out.push_str(&builtin(
            icons,
            loc,
            CHANCE_TO_ESCAPE,
            &[("NUM", Argument::Fixed5(parts.total))],
        )?);
        out.push('\n');
    }
    let line = |key, value| builtin(icons, loc, key, &[("NUM", Argument::Fixed5(value))]);
    if parts.base != 0 {
        out.push_str("  ");
        out.push_str(&line(BASE_ESCAPE_SPEED, parts.base)?);
        out.push('\n');
    }
    if parts.duration != 0 {
        out.push_str("  ");
        out.push_str(&line(ESCAPE_SPEED_COMBAT_DURATION, parts.duration)?);
        out.push('\n');
    }
    for &(_, screening) in by_key {
        if screening != 0 {
            out.push_str("  ");
            out.push_str(&line(SCREENING_VALUE, screening)?);
            out.push('\n');
        }
    }
    if parts.ship_speed != 0 {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("  ");
        out.push_str(&line(ESCAPE_SPEED_FROM_SHIP_SPEED, parts.ship_speed)?);
        out.push('\n');
    }
    if row.detection_state == 0 {
        out.push_str("  ");
        out.push_str(&line(HIDDEN_SUBS_ESCAPE_BONUS, parts.detection)?);
        out.push('\n');
    } else if row.live_ship != 0 && row.formation_flags & (1 << 7) != 0 {
        out.push_str("  ");
        out.push_str(&line(SUBS_ESCAPE_BONUS, parts.detection)?);
        out.push('\n');
    }
    Ok(out)
}

/// 1417D3220 resolves evacuation state +0x128, then reads its
/// CState::display_name@1409CC380 and the current owner-country name through
/// 140BA5960. Both have reader-language paths elsewhere in the viewer.
fn evacuation_names(
    document: &Document,
    frame: &DecodedFrame,
    id: u32,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<(String, String), String> {
    let states = rows::<tables::States>(frame).map_err(|error| error.to_string())?;
    let row = states
        .rows()
        .iter()
        .find(|row| row.id == id)
        .ok_or_else(|| format!("evacuation state {id} has no States row"))?;
    let state = super::text::state_name(
        loc,
        &document.recorded_language,
        id,
        &row.custom_name,
        &row.owner_name,
        || {
            let key = document
                .state_name_keys
                .get(&id)
                .ok_or_else(|| format!("evacuation state {id} has no content definition"))?
                .as_ref()
                .map_err(Clone::clone)?;
            token(icons, loc, key.as_bytes())
        },
    )?;
    let links = rows::<tables::CountryLinks>(frame).map_err(|error| error.to_string())?;
    let owner = crate::table_access::country_slot(&links, row.owner)?
        .ok_or_else(|| format!("evacuation state {id} has null country owner"))?;
    let names = installed_content::country_names::CountryNames::read(frame, loc);
    let country = names.name(owner)?;
    Ok((
        super::binding::displayable(icons, state)?,
        super::binding::displayable(icons, country)?,
    ))
}

/// 141AC37A0/141AC3510 and 140C2B520 divide the current amount by the
/// native maximum (minimum 100). The recorded row already has those operands.
fn ratio(value: i64, maximum: i64) -> i64 {
    if maximum == 0 || value == i64::MAX {
        return 0xffff_ffff;
    }
    value.wrapping_mul(100_000) / maximum
}

/// 14195F760's six fixed5 received-damage lanes. Only five have detailed
/// descriptor keys (static table 142A0A460); the sixth contributes to total.
fn criticals_and_damage(
    text: &mut String,
    row: &NavalCombatShipRow,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<(), String> {
    if row.received_criticals > 0 {
        text.push('\n');
        text.push_str(&builtin(
            icons,
            loc,
            NAVAL_COMBAT_RECEIVED_CRITICALS,
            &[("NUM", Argument::Int32(row.received_criticals))],
        )?);
    }
    let total = row
        .received_damage
        .iter()
        .fold(0_i64, |sum, value| sum.wrapping_add(*value));
    if total <= 0 {
        return Ok(());
    }
    text.push_str(&builtin(
        icons,
        loc,
        TOTAL_DAMAGE_RECEIVED,
        &[("NUM", Argument::Fixed5(total))],
    )?);
    const KEYS: [&str; 5] = [
        DAMAGE_DEALT_LIGHT_GUNS,
        DAMAGE_DEALT_HEAVY_GUNS,
        DAMAGE_DEALT_TORPEDOES,
        DAMAGE_DEALT_DEPTH_CHARGES,
        DAMAGE_DEALT_AIR,
    ];
    for (value, key) in row.received_damage.iter().zip(KEYS) {
        if *value <= 0 {
            continue;
        }
        let share = if *value == i64::MAX {
            0xffff_ffff
        } else {
            value.wrapping_mul(100_000) / total
        };
        text.push_str(&builtin(
            icons,
            loc,
            key,
            &[
                ("NUM", Argument::Fixed5(*value)),
                ("RATIO", Argument::Fixed5(share)),
            ],
        )?);
    }
    Ok(())
}

/// 1415B43F0 appends each of the two ordered air-hit vectors separately.
fn air_hits(
    text: &mut String,
    row: &NavalCombatShipRow,
    variants: &BTreeMap<u64, &VariantDefinitionRow>,
    icons: &dyn TextIcons,
    loc: &Localisation,
    context: Option<&VariantContext<'_>>,
) -> Result<(), String> {
    for vector in &row.air_hits {
        if vector.is_empty() {
            continue;
        }
        text.push_str(&builtin(
            icons,
            loc,
            NAVAL_COMBAT_AIRWING_TOOLTIP_SHOT_DOWN,
            &[],
        )?);
        for hit in vector {
            let variant = variants.get(&hit.variant).ok_or_else(|| {
                format!(
                    "air-hit variant {} has no VariantDefinitions row",
                    hit.variant
                )
            })?;
            let name = variant_name(variant, icons, loc, context)?;
            text.push_str(&builtin(
                icons,
                loc,
                NAVAL_COMBAT_AIRWING_TOOLTIP_SHOT_DOWN_ENTRY,
                &[
                    ("NUM", Argument::Int32(hit.count)),
                    ("MOD", Argument::Text(&name)),
                ],
            )?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::binding::NoIcons;

    #[test]
    fn escaping_member_section_has_native_newlines_and_source_order() {
        let localisation: Localisation = [
            ("ESCAPE_PROGRESS", "Progress|"),
            ("CHANCE_TO_ESCAPE", "Chance|"),
            ("BASE_ESCAPE_SPEED", "Base|"),
            ("ESCAPE_SPEED_COMBAT_DURATION", "Duration|"),
            ("ESCAPE_SPEED_FROM_SHIP_SPEED", "Speed|"),
            ("HIDDEN_SUBS_ESCAPE_BONUS", "Hidden|"),
        ]
        .into_iter()
        .map(|(key, text)| (key.to_owned(), text.to_owned()))
        .collect();
        let row = NavalCombatShipRow {
            state: 8,
            live_ship: 1,
            detection_state: 0,
            escape_progress: 50_000,
            ..Default::default()
        };
        let parts = super::super::naval_escape::Parts {
            base: 10_000,
            duration: 20_000,
            modifiers: 0,
            ship_speed: 630,
            detection: 5_000,
            total: 35_630,
        };
        assert_eq!(
            escape_text(&row, parts, &[], true, &NoIcons, &localisation).unwrap(),
            "\nProgress|\nChance|\n  Base|\n  Duration|\n  Speed|\n  Hidden|\n"
        );
    }

    #[test]
    fn convoy_escape_keeps_stored_lines_without_inventing_a_chance() {
        // Deliberately no CHANCE_TO_ESCAPE localisation: that unrecorded
        // value must not be formatted, even as a zero/default chance.
        let localisation: Localisation = [
            ("ESCAPE_PROGRESS", "Progress $NUM$"),
            ("BASE_ESCAPE_SPEED", "Base $NUM$"),
            ("ESCAPE_SPEED_COMBAT_DURATION", "Duration $NUM$"),
            ("ESCAPE_SPEED_FROM_SHIP_SPEED", "Speed $NUM$"),
            ("HIDDEN_SUBS_ESCAPE_BONUS", "Hidden $NUM$"),
        ]
        .into_iter()
        .map(|(key, text)| (key.to_owned(), text.to_owned()))
        .collect();
        let row = NavalCombatShipRow {
            combat: 42,
            side: 1,
            battle_slot: 7,
            state: 8,
            live_convoy: 1,
            detection_state: 0,
            escape_progress: 50_000,
            ..Default::default()
        };
        let parts = super::super::naval_escape::Parts {
            base: 10_000,
            duration: 20_000,
            modifiers: 0,
            ship_speed: 630,
            detection: 5_000,
            total: 35_630,
        };
        let text = escape_text(&row, parts, &[], false, &NoIcons, &localisation).unwrap();
        assert_eq!(
            text,
            "\nProgress 0.50\n  Base 0.10\n  Duration 0.20\n  Speed 0.00\n  Hidden 0.05\n"
        );

        let mut out = Bindings::default();
        set_hover(&mut out.values, "icon", hover_text(&text), None);
        let refusal = escape_breakdown_refusal(&row);
        out.refuse(refusal.clone());
        out.refuse(refusal.clone());
        assert_eq!(out.refused()[&refusal], 2);
        assert!(refusal.contains("convoy escape chance"));
        assert!(refusal.contains("pending capture-on-change"));
        assert_eq!(
            out.values["icon"].tooltip.as_deref(),
            Some("Progress 0.50\n  Base 0.10\n  Duration 0.20\n  Speed 0.00\n  Hidden 0.05")
        );
    }

    fn loc() -> Localisation {
        [
            ("SHIP_AND_TYPE", "$SHIP$:$TYPE$"),
            ("NAVAL_COMBAT_SHIP_TOOLTIP", "$FLAG$ $NAME$ $STATUS$|"),
            ("FEX_STATE_FIGHTING", "fighting"),
            ("FEX_STATE_ON_THE_WAY", "incoming"),
            ("FEX_STATE_ESCAPING", "escaping"),
            ("FEX_STATE_SUNK", "sunk"),
            ("SHIP_SUNK_BY", "sunk by $NAME$"),
            ("NAVAL_COMBAT_SHIP_STRENGTH", "S$STR$|"),
            ("NAVAL_COMBAT_SHIP_ORGANIZATION_FACTOR", "O$ORG$|"),
            ("NAVAL_COMBAT_HOURS_TO_ARRIVE", "A$NUM$|"),
            ("NAVAL_COMBAT_RECEIVED_CRITICALS", "C$NUM$|"),
            ("total_damage_received", "T$NUM$|"),
            ("damage_dealt_light_guns", "L$NUM$/$RATIO$|"),
            ("damage_dealt_heavy_guns", "H$NUM$/$RATIO$|"),
            ("damage_dealt_torpedoes", "P$NUM$/$RATIO$|"),
            ("damage_dealt_depth_charges", "D$NUM$/$RATIO$|"),
            ("damage_dealt_air", "F$NUM$/$RATIO$|"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
    }

    #[test]
    fn explicit_convoy_hover_orders_condition_arrival_criticals_and_damage() {
        let row = NavalCombatShipRow {
            state: 4,
            live_convoy: 1,
            variant: 7,
            strength: 50_000,
            maximum_hit_points: 100_000,
            organisation: 25_000,
            base_organisation: 100_000,
            arrival_hours: 3,
            received_criticals: 2,
            received_damage: [20_000, 0, 0, 0, 0, 5_000],
            ..Default::default()
        };
        let variant = VariantDefinitionRow {
            id: 7,
            display_name: b"Escort".to_vec(),
            variant_name: b"Mk II".to_vec(),
            append_variant_name: 1,
            ..Default::default()
        };
        let variants = BTreeMap::from([(7, &variant)]);
        let text = member_tooltip(&row, "AAA", &variants, &NoIcons, &loc(), None, None).unwrap();
        let ordered = ["Escort Mk II", "incoming", "S", "O", "A3", "C2", "T", "L"];
        let mut cursor = 0;
        for term in ordered {
            let position = text[cursor..].find(term).unwrap();
            cursor += position + term.len();
        }
        assert!(text.contains(&format!("{INLINE_FLAG}AAA")));
    }

    #[test]
    fn cached_ship_text_draws_in_recorded_language_and_escape_requires_context() {
        let ship = NavalCombatShipRow {
            side: 1,
            battle_slot: 9,
            ship: 11,
            live_ship: 0,
            state: 32,
            name: b"client-language ship".to_vec(),
            type_name: b"client-language type".to_vec(),
            sunk_by: b"the carrier".to_vec(),
            ..Default::default()
        };
        let mut localisation = loc();
        localisation.language = "english".into();
        let text = member_tooltip(
            &ship,
            "AAA",
            &BTreeMap::new(),
            &NoIcons,
            &localisation,
            None,
            Some("english"),
        )
        .unwrap();
        assert!(text.contains("client-language ship:client-language type"));
        assert!(text.contains("sunk by the carrier"));
        let reason = member_tooltip(
            &ship,
            "AAA",
            &BTreeMap::new(),
            &NoIcons,
            &localisation,
            None,
            Some("german"),
        )
        .unwrap_err();
        assert!(reason.contains("stored in german"), "{reason}");
        // 1417D3220 reserves the sunk branch for two null live references.
        let mut still_live = ship.clone();
        still_live.live_ship = 1;
        let live_text = member_tooltip(
            &still_live,
            "AAA",
            &BTreeMap::new(),
            &NoIcons,
            &localisation,
            None,
            Some("english"),
        )
        .unwrap();
        assert!(!live_text.contains("sunk by the carrier"));
        let mut escaping = ship;
        escaping.state = 8;
        escaping.live_convoy = 1;
        escaping.base_organisation = 100_000;
        let reason = member_tooltip(
            &escaping,
            "AAA",
            &BTreeMap::new(),
            &NoIcons,
            &localisation,
            None,
            Some("english"),
        )
        .unwrap_err();
        assert!(reason.contains("naval escape tooltip context"), "{reason}");
    }

    #[test]
    fn one_unavailable_member_withholds_the_whole_group_and_counts_each_gap() {
        let ships = [
            NavalCombatShipRow {
                side: 0,
                battle_slot: 2,
                owner: 4,
                ship: 10,
                live_ship: 1,
                ..Default::default()
            },
            NavalCombatShipRow {
                side: 0,
                battle_slot: 3,
                owner: 4,
                ship: 11,
                live_ship: 1,
                ..Default::default()
            },
        ];
        let by_slot = ships
            .iter()
            .map(|row| ((row.side, row.battle_slot), row))
            .collect();
        let country = replay::rows::CountryRow {
            id: 4,
            tag: b"AAA".to_vec(),
            ..Default::default()
        };
        let countries = BTreeMap::from([(4, &country)]);
        let result = group_tooltip(
            &[2, 3],
            0,
            &by_slot,
            &BTreeMap::new(),
            &countries,
            &NoIcons,
            &loc(),
            None,
        );
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 2);
        assert!(errors[0].contains("slot 2"));
        assert!(errors[1].contains("slot 3"));
    }
}
