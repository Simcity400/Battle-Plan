//! Naval combat box hovers. Combat members' cached labels are stored text and
//! can be drawn only in the recording language.
use super::{
    INLINE_FLAG,
    binding::{Bindings, TextIcons, builtin, token},
    tooltips::{hover_text, set_hover},
};
use crate::{document::Document, table_access::rows};
use installed_content::localisation::{Argument, Localisation};
use replay::{
    frame::DecodedFrame,
    rows::{NavalCombatDetailsRow, NavalCombatShipRow, VariantDefinitionRow, tables},
};
use std::{collections::BTreeMap, io};

struct VariantContext<'a> {
    document: &'a Document,
    frame: &'a DecodedFrame,
    names: super::naval_variants::Names<'a>,
    escape: EscapeContext<'a>,
}

/// What the escape section reads beyond the member's own row.
struct EscapeContext<'a> {
    details: Result<&'a [NavalCombatDetailsRow], String>,
    sources: super::naval_trees::Sources<'a>,
}

// Localisation keys for naval ship and damage hovers.
super::text::tooltip_keys! {
    NAVAL_COMBAT_INCOMING_UNKNOWN = "NAVAL_COMBAT_INCOMING_UNKNOWN",
    // Append separators between members.
    SEPARATION_LINE = "SEPARATION_LINE",
    // ship icon hover prepends this for the hidden box.
    NAVAL_COMBAT_HIDDEN_SUBS = "NAVAL_COMBAT_HIDDEN_SUBS",
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

/// Build group hover text in member order, adding a newline after each
/// member string. An incoming box whose owner is hidden shows one warning key.
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
        names: super::naval_variants::Names::new(document, frame),
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
            sources: super::naval_trees::Sources::read(frame, &document.recorded_language)?,
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
        let anchor = group_anchor(&entry.battle_slots, side, &by_slot);
        let built = if entry.unknown {
            // Localisation key, resolved through the content stack.
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
                Some(&document.recorded_language),
            )
        };
        match built {
            Ok(text) => group_hovers(&mut entry.values, hover_text(&text), anchor),
            Err(reasons) => {
                group_hovers(&mut entry.values, None, anchor);
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

fn group_anchor(
    slots: &[u32],
    side: u8,
    ships: &BTreeMap<(u8, u32), &NavalCombatShipRow>,
) -> super::TooltipAnchor {
    if slots
        .first()
        .and_then(|slot| ships.get(&(side, *slot)))
        .is_some_and(|row| row.state == 32)
    {
        // Sunk entries use default tooltip placement.
        super::TooltipAnchor::Pointer
    } else {
        super::TooltipAnchor::NavalWindow // Group anchor.
    }
}

/// The hull, labels, hidden marker, condition bars, escape progress and
/// chevrons share the group hover, with a separate pride branch. Each owns its
/// hit target so chevrons cannot fall through to the grid's formation text.
fn group_hovers(
    values: &mut super::Values,
    text: Option<super::HoverText>,
    anchor: super::TooltipAnchor,
) {
    for key in [
        "icon",
        "desc_left",
        "desc_right",
        "icon_hidden",
        "bar_str",
        "bar_org",
        "escape_progress",
        "escape_icon_left",
        "escape_icon_right",
    ] {
        set_hover(values, key, text.clone(), None);
        values.entry(key.into()).or_default().tooltip_anchor = anchor;
    }
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
    recorded_language: Option<&str>,
) -> Result<String, Vec<String>> {
    // Column 4 supplies the hidden-submarine bit; the entry
    // retains it for known boxes. the game prepends the warning but still
    // appends every member's details, for either viewing side.
    let mut text = if slots
        .first()
        .and_then(|slot| ships.get(&(side, *slot)))
        // Submarine boxes override every carrier equipment flag.
        .is_some_and(|row| super::naval_details::column(row, 0) == Some("hidden"))
    {
        builtin(icons, loc, NAVAL_COMBAT_HIDDEN_SUBS, &[]).map_err(|reason| vec![reason])?
    } else {
        String::new()
    };
    let mut refusals = Vec::new();
    for (index, slot) in slots.iter().enumerate() {
        let part = (|| {
            let row = ships
                .get(&(side, *slot))
                .ok_or_else(|| format!("NavalCombatShips side {side} slot {slot} is absent"))?;
            let country = countries
                .get(&row.owner)
                .ok_or_else(|| format!("country {} has no Countries row", row.owner))?;
            let tag = std::str::from_utf8(&country.tag)
                .map_err(|_| format!("country {} tag is not UTF-8", row.owner))?;
            member_tooltip(row, tag, variants, icons, loc, context, recorded_language)
        })();
        match part {
            Ok(part) => {
                if index != 0 {
                    // Both live and sunk entries scan backwards over spaces, adding a newline
                    // only when the preceding non-space byte is not a newline. Then append the
                    // stacked SEPARATION_LINE verbatim.
                    if !text.trim_end_matches(' ').ends_with('\n') {
                        text.push('\n');
                    }
                    match builtin(icons, loc, SEPARATION_LINE, &[]) {
                        Ok(separator) => text.push_str(&separator),
                        Err(reason) => refusals.push(reason),
                    }
                }
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

fn variant_name(
    row: &VariantDefinitionRow,
    icons: &dyn TextIcons,
    loc: &Localisation,
    context: Option<&VariantContext<'_>>,
) -> Result<String, String> {
    super::naval_variants::name(row, icons, loc, context.map(|context| &context.names))
}

/// Member name and type labels are cached text, rendered only in the
/// recording language.
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
                    "side {} slot {} combat member {field}",
                    row.side, row.battle_slot
                ),
            )
        };
        (label(&row.name, "name")?, label(&row.type_name, "type")?)
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
        // The game starts a sunk member with flag, one space, green
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
                    "side {} slot {} combat member sinking attribution",
                    row.side, row.battle_slot
                ),
            )?;
            // The game uses SHIP_SUNK_BY with its executable NAME argument.
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
    // enum dispatch. State 0 also has a native string, although
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

/// Append the stored escape progress, the on-demand escape
/// chance, then its source-order description. Numeric defines come from the
/// matching content stack; source labels are stored text and only render in
/// the recording language.
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
    let assets = &context.document.icons.combat;
    let defines = assets.naval_escape_defines.as_ref().map_err(Clone::clone)?;
    escape_text(
        row,
        combat,
        &context.escape.sources,
        defines,
        &assets.naval_escape_screening,
        icons,
        loc,
    )
}

/// The complete appended state-8 section: progress, chance, base and
/// duration terms, each key's sources in native order with its screening
/// line, then the speed and submarine terms.
fn escape_text(
    row: &NavalCombatShipRow,
    combat: &NavalCombatDetailsRow,
    sources: &super::naval_trees::Sources<'_>,
    defines: &super::naval_escape::Defines,
    screening_content: &super::naval_escape::Screening,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<String, String> {
    let side_index = usize::from(row.side);
    let side = combat
        .sides
        .get(side_index)
        .ok_or_else(|| format!("naval combat {} has no side {}", row.combat, row.side))?;
    let opposing = &combat.sides[1 - side_index];
    let definitions = sources.definitions()?;
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
    for &key in keys {
        // Source formatting takes the side sum when the member has no live
        // ship; its text is written below, in order.
        let side_total = if row.live_ship != 0 {
            Ok(0)
        } else {
            let mut scratch = String::new();
            sources.side(
                combat.combat,
                side,
                opposing,
                key,
                "  ",
                &mut scratch,
                icons,
                loc,
            )
        };
        let (total, screening) = super::naval_escape::modifier_value(
            row,
            side,
            key,
            side_total,
            screening_content,
            definitions,
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
    let mut out = String::new();
    out.push('\n');
    out.push_str(&builtin(
        icons,
        loc,
        ESCAPE_PROGRESS,
        &[("NUM", Argument::Fixed5(row.escape_progress))],
    )?);
    out.push('\n');
    out.push_str(&builtin(
        icons,
        loc,
        CHANCE_TO_ESCAPE,
        &[("NUM", Argument::Fixed5(parts.total))],
    )?);
    out.push('\n');
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
    for &(key, screening) in &by_key {
        if row.live_ship != 0 {
            sources.ship(row, key, "  ", &mut out, icons, loc)?;
        } else {
            sources.side(
                combat.combat,
                side,
                opposing,
                key,
                "  ",
                &mut out,
                icons,
                loc,
            )?;
        }
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

/// Resolve the evacuation state, then its display name and current owner
/// name. Both use the shared language-aware readers.
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

/// Divide the current amount by its maximum, floored at 100.
/// The recorded row already contains both operands.
fn ratio(value: i64, maximum: i64) -> i64 {
    if maximum == 0 || value == i64::MAX {
        return 0xffff_ffff;
    }
    value.wrapping_mul(100_000) / maximum
}

/// six fixed5 received-damage lanes. Only five have detailed
/// descriptor keys (static table); the sixth contributes to total.
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

/// The game appends each of the two ordered air-hit vectors separately.
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
mod tests;
