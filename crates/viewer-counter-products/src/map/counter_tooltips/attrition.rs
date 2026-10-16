//! Own singleton attrition child: the counter's division attrition calculation
//! uses caller mode zero and prepends `ATTRITION_TOTAL` to its per-source lines.
//!
//! The total is the one the counter itself shows,
//! `DivisionStatus.attrition_value` (+0x438), stored by the hourly division
//! update from the same calculation (zero while the game state's +0xA39 byte
//! is clear). The attrition-presence predicate compares it with the threshold
//! for the counter's attrition glyph. The game's hover evaluates the total afresh, so
//! a term that changes after the hourly update (acclimatization, a modifier)
//! shows there first.
//!
//! The lines follow the game in its order, each derived exactly from the
//! recording and its content where its operands are recorded:
//! - upkeep: `ATTRITION_UPKEEP` when nonzero. Its value
//!   `P(max(0, 1 + owner 0x54), UNIT_UPKEEP_ATTRITION)` with the
//!   positive expeditionary owner, else the roster owner, is the product
//!   the attrition-presence predicate compares with, which DivisionStatus
//!   records as `attrition_threshold`;
//! - terrain: for a unit with a positive route count (unit+0x20C) and a clear
//!   strategic-redeployment byte (+0x2AF) on a valid terrain (+0x10, zero only
//!   for the installed null terrain), `ATTRITION_TERRAIN` with the terrain's
//!   name and
//!   upkeep plus the getter's `attrition` (0x4F) of its common modifier
//!   (`+0xB8`) when that sum is nonzero;
//! - training: `TRAINING_ATTRITION` for an exercising unit
//!   (DivisionStatus.exercising) with a
//!   route count below one;
//! - border war: for a state whose border war is active
//!   (StateFlags.border_conflict_active),
//!   `BORDER_WAR_HEADER` and `BORDER_WAR_ATTRITION`, the enemy share
//!   for the state and logical owner in mode zero, scaled by
//!   `BORDER_WAR_ATTRITION_FACTOR`;
//! - supply: `ATTRITION_SUPPLY` with the out-of-supply severity
//!   normalized by the army attrition supply threshold, scaled by the
//!   country's and the selected commander block's out-of-supply factors,
//!   times the out-of-supply attrition define, when nonzero
//!   ([`viewer_foundation::movement_timing::HourTerms::attrition_supply`]);
//!   a positive special-forces factor needs the template's special-forces
//!   share, which is refused by name;
//! - a moving unit's weather, heat and winter lines are absent when its
//!   province's recorded weather attrition entry is zero and its heat and
//!   winter entries scaled by acclimatization are not positive.
//!
//! The other lines read operands the recording does not hold, and each is
//! refused by name: the terrain line's held-variant equipment term
//! (equipment-variant field +0x3A0), the state modifier for the country
//! (keys 0x4F/0x1EE/0x1EF), the upkeep (0x54) and country (0x4F) modifier
//! source trees of `country+0x5B8`, a moving unit's weather, heat and winter
//! source trees where those lines appear, and its moving-state block.

use super::*;
use replay::modifier_registry::Registry;
use std::collections::BTreeMap;
use viewer_foundation::map::counter_attrition::{Content, Terrain};
use viewer_foundation::movement_timing::HourTerms;
use viewer_foundation::tooltip_text::ensure_newlines;

use viewer_foundation::map::counter_inputs::SCALE as ONE;

const EQUIPMENT_UNRECORDED: &str = "the per-terrain attrition of the division's held variants (equipment variant+0x3A0) is not recorded";
const STATE_UNRECORDED: &str = "the state's attrition modifiers for the roster owner (keys 0x4F, 0x1EE and 0x1EF) are not recorded";
const UPKEEP_SOURCES_UNRECORDED: &str =
    "the country's unit_upkeep_attrition_factor sources (country+0x5B8, key 0x54) are not recorded";
const WEATHER_UNRECORDED: &str =
    "a moving unit's province weather, heat and winter modifier source trees are not recorded";
const MOVING_STATE_UNRECORDED: &str =
    "a moving unit's state attrition modifier for its logical owner (key 0x4F) is not recorded";
const COUNTRY_SOURCES_UNRECORDED: &str =
    "the country's attrition sources (country+0x5B8, key 0x4F, depth 2) are not recorded";

/// What the lines read beyond the frame.
struct Lookups<'b> {
    content: &'b Content,
    /// A province's one-based terrain ordinal (`terrain+0x88`); zero
    /// is the null terrain.
    terrain: &'b dyn Fn(u32) -> Result<i32, String>,
    /// The border war's enemy share in mode zero, for a raw country link
    /// and a state id.
    border: &'b dyn Fn(u32, u32) -> Result<i64, String>,
}

/// Attrition returns zero and writes nothing when the unit has neither a
/// route province (the first province of
/// a positive ordinary route) nor a location (unit+0x1F0).
fn located(context: &Context<'_>, id: u64) -> Result<bool, String> {
    if context.division(id)?.province != 0 {
        return Ok(true);
    }
    let movement = context.movement(id)?;
    if movement.path_total as i32 <= 0 {
        return Ok(false);
    }
    movement
        .path
        .first()
        .map(|&first| first != 0)
        .ok_or_else(|| "the attrition route province is among the route's unrecorded hops".into())
}

/// The attrition province: the first province of a positive route,
/// else the location.
fn selected_province(context: &Context<'_>, id: u64) -> Result<u32, String> {
    let movement = context.movement(id)?;
    if movement.path_total as i32 > 0 {
        let first = *movement
            .path
            .first()
            .ok_or("the attrition route province is among the route's unrecorded hops")?;
        if first != 0 {
            return Ok(first);
        }
    }
    Ok(context.division(id)?.province)
}

/// The province's state (`province+0xC0`); zero without one.
fn province_state(context: &Context<'_>, province: u32) -> Result<u32, String> {
    let rows = context
        .frame
        .complete::<tables::Provinces>()
        .map_err(|e| e.to_string())?;
    rows.binary_search_by_key(&province, |row| row.id)
        .map(|at| rows[at].state)
        .map_err(|_| format!("Provinces province {province} is absent"))
}

fn fragment(context: &Context<'_>, out: &mut Tooltip, label: &str, text: Result<String, String>) {
    match text.and_then(|text| displayable(context.icons, text)) {
        Ok(text) if !text.is_empty() => {
            // Ensure the existing text ends with a newline before each line.
            ensure_newlines(&mut out.title, 1);
            out.title.push_str(&text);
        }
        Ok(_) => (),
        Err(reason) => refuse(out, label, reason),
    }
}

/// The coloured percent formatter: the fixed5
/// product with a hundred, `+` at zero too, red when positive, green when
/// negative, yellow at zero.
fn coloured_percent(value: i64, precision: i32) -> String {
    let scaled = value.wrapping_mul(10_000_000) / ONE;
    let colour = match scaled.signum() {
        1 => 'R',
        -1 => 'G',
        _ => 'Y',
    };
    let sign = if scaled >= 0 { "+" } else { "" };
    let number = installed_content::fixed5::fixed5_to_string(scaled, precision);
    format!("\u{11}{colour}{sign}{number}%\u{11}!")
}

/// The coloured percent formatter's supply line: `ATTRITION_SUPPLY` with
/// the line value at two decimals, absent when the severity is zero.
fn supply(
    context: &Context<'_>,
    lookups: &Result<&Lookups<'_>, String>,
    id: u64,
    out: &mut Tooltip,
) {
    let value = (|| {
        let lookups = lookups.as_ref().map_err(Clone::clone)?;
        let division = context.division(id)?;
        let movement = context.movement(id)?;
        HourTerms::new(context.frame).attrition_supply(
            division,
            movement,
            lookups.content.supply_threshold,
        )
    })();
    match value {
        Ok(None) => (),
        Ok(Some(value)) => fragment(
            context,
            out,
            "attrition supply",
            localise(
                context.loc,
                "ATTRITION_SUPPLY",
                &[("ATT", Arg::Text(&coloured_percent(value, 2)))],
            ),
        ),
        Err(reason) => refuse(out, "attrition supply", reason),
    }
}

/// Whether a moving unit's province weather writes any line: a nonzero
/// effective attrition entry, or a heat or winter entry that is positive
/// once scaled by the unit's acclimatization factor.
fn weather_lines(context: &Context<'_>, id: u64, province: u32) -> Result<bool, String> {
    let rows = context
        .frame
        .complete::<tables::WeatherProvinces>()
        .map_err(|e| e.to_string())?;
    let Ok(at) = rows.binary_search_by_key(&province, |row| row.id) else {
        // A province without a weather record has no weather modifier.
        return Ok(false);
    };
    let modifiers = rows[at]
        .modifiers
        .require("WeatherProvinces.modifiers")?
        .as_ref()
        .ok_or("the province's weather modifier entries were refused when recorded")?;
    let registry = Registry::from_frame(context.frame)?;
    let effective = |name: &str, raw: Option<i64>| -> Result<i64, String> {
        match raw {
            None => Ok(0),
            Some(raw) => registry.effective(registry.named(name)?, raw),
        }
    };
    if effective("attrition", modifiers.attrition)? != 0 {
        return Ok(true);
    }
    let thermal = [
        effective("heat_attrition", modifiers.heat_attrition)?,
        effective("winter_attrition", modifiers.winter_attrition)?,
    ];
    if thermal == [0, 0] {
        return Ok(false);
    }
    let factor = context
        .status(id)?
        .weather_factor
        .require("DivisionStatus.weather_factor")?
        .ok_or("the division's acclimatization factor was refused when recorded")?;
    Ok(thermal
        .into_iter()
        .any(|value| value.wrapping_mul(factor) / ONE > 0))
}

fn terrain(
    context: &Context<'_>,
    lookups: &Result<&Lookups<'_>, String>,
    upkeep: i64,
    province: u32,
    out: &mut Tooltip,
) {
    let line: Result<Option<(&Terrain, i64)>, String> = (|| {
        let lookups = lookups.as_ref().map_err(Clone::clone)?;
        let ordinal = (lookups.terrain)(province)?;
        if ordinal == 0 {
            return Ok(None);
        }
        let terrain = usize::try_from(ordinal - 1)
            .ok()
            .and_then(|at| lookups.content.terrains.get(at))
            .ok_or_else(|| {
                format!("terrain ordinal {ordinal} is outside the content's terrain database")
            })?;
        // Sum the common modifier's accumulator (+0xB8), key 0x4F.
        let attrition = match terrain.attrition {
            None => 0,
            Some(raw) => {
                let registry = Registry::from_frame(context.frame)?;
                registry.effective(registry.named("attrition")?, raw)?
            }
        };
        Ok(Some((terrain, upkeep.wrapping_add(attrition))))
    })();
    match line {
        Ok(None) | Ok(Some((_, 0))) => (),
        Ok(Some((terrain, value))) => {
            let text = context
                .loc
                .localise(&terrain.name)
                .map_err(|e| e.to_string())
                .and_then(|name| {
                    localise(
                        context.loc,
                        "ATTRITION_TERRAIN",
                        &[("TERR", Arg::Text(&name)), ("ATT", Arg::Fixed5(value))],
                    )
                });
            fragment(context, out, "attrition terrain", text);
            // The equipment line follows inside the same branch when nonzero.
            refuse(out, "attrition equipment", EQUIPMENT_UNRECORDED);
        }
        Err(reason) => refuse(out, "attrition terrain", reason),
    }
}

fn lines(context: &Context<'_>, lookups: Result<&Lookups<'_>, String>, id: u64, out: &mut Tooltip) {
    let status = match context.status(id) {
        Ok(status) => status,
        Err(reason) => {
            refuse(out, "attrition lines", reason);
            return;
        }
    };
    let upkeep = status.attrition_threshold;
    if upkeep != 0 {
        fragment(
            context,
            out,
            "attrition upkeep",
            localise(
                context.loc,
                "ATTRITION_UPKEEP",
                &[("VALUE", Arg::Fixed5(upkeep))],
            ),
        );
    }
    let province = selected_province(context, id);
    let movement = context.movement(id);
    let moving = movement
        .as_ref()
        .map_err(Clone::clone)
        .map(|movement| movement.path_total as i32 > 0 && movement.strategic_redeployment == 0);
    match (&moving, &province) {
        (Ok(false), _) => (),
        (Ok(true), Ok(province)) => terrain(context, &lookups, upkeep, *province, out),
        (Err(reason), _) | (Ok(true), Err(reason)) => refuse(out, "attrition terrain", reason),
    }
    let state = province
        .clone()
        .and_then(|province| province_state(context, province));
    match &state {
        Ok(0) => (),
        Ok(_) => refuse(out, "attrition state", STATE_UNRECORDED),
        Err(reason) => refuse(out, "attrition state", reason.clone()),
    }
    supply(context, &lookups, id, out);
    refuse(out, "attrition upkeep modifiers", UPKEEP_SOURCES_UNRECORDED);
    // The moving-state, weather, heat and winter blocks run for a moving unit
    // whose logical owner is at war (mode 0).
    match (&moving, &province) {
        (Ok(false), _) => (),
        (Ok(true), Ok(province)) => {
            if !matches!(state, Ok(0)) {
                refuse(out, "attrition moving state", MOVING_STATE_UNRECORDED);
            }
            match weather_lines(context, id, *province) {
                Ok(false) => (),
                Ok(true) => refuse(out, "attrition weather", WEATHER_UNRECORDED),
                Err(reason) => refuse(out, "attrition weather", reason),
            }
        }
        (Err(reason), _) | (Ok(true), Err(reason)) => {
            refuse(out, "attrition weather", reason.clone())
        }
    }
    match &movement {
        Ok(movement) if status.exercising != 0 && (movement.path_total as i32) < 1 => {
            let text = lookups
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|lookups| {
                    localise(
                        context.loc,
                        "TRAINING_ATTRITION",
                        &[(
                            "VALUE",
                            Arg::Text(&coloured_percent(lookups.content.training, 1)),
                        )],
                    )
                })
                .map(|text| format!("{text}\n"));
            fragment(context, out, "attrition training", text);
        }
        Ok(_) => (),
        Err(reason) if status.exercising != 0 => refuse(out, "attrition training", reason.clone()),
        Err(_) => (),
    }
    if let Ok(state) = state
        && state != 0
    {
        border(context, &lookups, id, state, out);
    }
    refuse(
        out,
        "attrition country modifiers",
        COUNTRY_SOURCES_UNRECORDED,
    );
}

fn border(
    context: &Context<'_>,
    lookups: &Result<&Lookups<'_>, String>,
    id: u64,
    state: u32,
    out: &mut Tooltip,
) {
    let active = (|| {
        let flags = context
            .frame
            .complete::<tables::StateFlags>()
            .map_err(|e| e.to_string())?;
        let row = flags
            .iter()
            .find(|row| row.state == state)
            .ok_or_else(|| format!("StateFlags state {state} is absent"))?;
        Ok::<_, String>(
            *row.border_conflict_active
                .require("StateFlags.border_conflict_active")?
                != 0,
        )
    })();
    match active {
        Ok(false) => return,
        Ok(true) => (),
        Err(reason) => {
            refuse(out, "attrition border", reason);
            return;
        }
    }
    fragment(
        context,
        out,
        "attrition border heading",
        localise(context.loc, "BORDER_WAR_HEADER", &[]).map(|text| format!("{text}\n")),
    );
    let value = (|| {
        let lookups = lookups.as_ref().map_err(Clone::clone)?;
        let owner = context.division(id)?.logical_owner;
        let share = (lookups.border)(owner, state)?;
        Ok::<_, String>(share.wrapping_mul(lookups.content.border_factor) / ONE)
    })();
    fragment(
        context,
        out,
        "attrition border",
        value
            .and_then(|value| {
                localise(
                    context.loc,
                    "BORDER_WAR_ATTRITION",
                    &[("VAL", Arg::Fixed5(value))],
                )
            })
            .map(|text| format!("{text}\n")),
    );
}

/// `ATTRITION_TOTAL` with the counter's cached total, before the lines.
fn total(context: &Context<'_>, id: u64, out: &mut Tooltip) {
    let total = context
        .status(id)
        .and_then(|status| {
            localise(
                context.loc,
                "ATTRITION_TOTAL",
                &[("TOT", Arg::Fixed5(status.attrition_value))],
            )
        })
        .and_then(|text| displayable(context.icons, text));
    match total {
        Ok(mut header) => {
            // The game starts the line body on a new line before prepending the total.
            if !out.title.is_empty() && !out.title.starts_with('\n') {
                ensure_newlines(&mut header, 1);
            }
            out.title.insert_str(0, &header);
        }
        Err(reason) => refuse(out, "attrition total", reason),
    }
    // Trim completed tooltip text with its four whitespace bytes.
    out.title = out.title.trim_matches(['\n', '\t', '\r', ' ']).to_owned();
}

fn append_with(
    context: &Context<'_>,
    lookups: Result<&Lookups<'_>, String>,
    id: u64,
    out: &mut Tooltip,
) {
    match located(context, id) {
        Ok(true) => (),
        Ok(false) => return,
        Err(reason) => {
            refuse(out, "attrition total", reason);
            return;
        }
    }
    lines(context, lookups, id, out);
    total(context, id, out);
}

pub(super) fn append(context: &Context<'_>, id: u64, out: &mut Tooltip) {
    let Some(content) = context.content else {
        append_with(
            context,
            Err("the counter content is unavailable".into()),
            id,
            out,
        );
        return;
    };
    let attrition = match &content.attrition {
        Ok(attrition) => attrition,
        Err(reason) => {
            append_with(context, Err(reason.clone()), id, out);
            return;
        }
    };
    let links: Result<BTreeMap<u32, u32>, String> = context
        .links
        .as_ref()
        .map(|rows| {
            rows.iter()
                .map(|row| (row.id, row.canonical_slot))
                .collect()
        })
        .map_err(Clone::clone);
    let terrain = |province: u32| -> Result<i32, String> {
        content
            .movement
            .as_deref()
            .ok_or("the movement map is unavailable")?
            .map
            .terrain(province)
            .map(|terrain| terrain.index)
            .ok_or_else(|| format!("province {province} has no terrain record"))
    };
    let border = |country: u32, state: u32| -> Result<i64, String> {
        let links = links.as_ref().map_err(Clone::clone)?;
        content
            .province_hover
            .border_enemy_share(context.frame, country, links, state)
    };
    let lookups = Lookups {
        content: attrition,
        terrain: &terrain,
        border: &border,
    };
    append_with(context, Ok(&lookups), id, out);
}

#[cfg(test)]
mod tests;
