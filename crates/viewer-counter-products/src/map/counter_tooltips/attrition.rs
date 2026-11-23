
use super::*;
use replay_derive::modifier_registry::Registry;
use std::collections::BTreeMap;
use viewer_unit_reads::counter_attrition::{Content, Terrain};
use viewer_foundation::movement_timing::HourTerms;
use viewer_foundation::tooltip_text::ensure_newlines;

use viewer_unit_reads::counter_inputs::SCALE as ONE;

use viewer_foundation::division_panel as panel;

struct Lookups<'b> {
    content: &'b Content,
    terrain: &'b dyn Fn(u32) -> Result<i32, String>,
    border: &'b dyn Fn(u32, u32) -> Result<i64, String>,
}

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
            ensure_newlines(&mut out.title, 1);
            out.title.push_str(&text);
        }
        Ok(_) => (),
        Err(reason) => refuse(out, label, reason),
    }
}

fn coloured_percent(value: i64, precision: i32) -> String {
    let scaled = value.wrapping_mul(10_000_000) / ONE;
    let colour = match scaled.signum() {
        1 => 'R',
        -1 => 'G',
        _ => 'Y',
    };
    let sign = if scaled >= 0 { "+" } else { "" };
    let number = content_reader_parser::fixed5::fixed5_to_string(scaled, precision);
    format!("\u{11}{colour}{sign}{number}%\u{11}!")
}

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
            lookups.content,
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

fn weather_lines(context: &Context<'_>, id: u64, province: u32) -> Result<bool, String> {
    let rows = context
        .frame
        .complete::<tables::WeatherProvinces>()
        .map_err(|e| e.to_string())?;
    let Ok(at) = rows.binary_search_by_key(&province, |row| row.id) else {
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
    use engine_executable::{
        modifier_keys::ATTRITION, modifier_keys::HEAT_ATTRITION, modifier_keys::WINTER_ATTRITION,
    };
    if effective("attrition", modifiers.get(ATTRITION))? != 0 {
        return Ok(true);
    }
    let thermal = [
        effective("heat_attrition", modifiers.get(HEAT_ATTRITION))?,
        effective("winter_attrition", modifiers.get(WINTER_ATTRITION))?,
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
    id: u64,
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
            let equipment = (|| {
                let lookups = lookups.as_ref().map_err(Clone::clone)?;
                panel::held_bonus(context.frame, id, (lookups.terrain)(province)? as u32)
            })();
            match equipment {
                Ok(0) => (),
                Ok(value) => fragment(
                    context,
                    out,
                    "attrition equipment",
                    localise(
                        context.loc,
                        "ATTRITION_EQUIPMENT",
                        &[("ATT", Arg::Fixed5(value))],
                    ),
                ),
                Err(why) => refuse(out, "attrition equipment", why),
            }
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
        (Ok(true), Ok(province)) => terrain(context, &lookups, upkeep, id, *province, out),
        (Err(reason), _) | (Ok(true), Err(reason)) => refuse(out, "attrition terrain", reason),
    }
    let state = province
        .clone()
        .and_then(|province| province_state(context, province));
    match &state {
        Ok(0) => (),
        Ok(state) => {
            let value = (|| {
                let states = context
                    .frame
                    .complete::<tables::States>()
                    .map_err(|why| why.to_string())?;
                let controller = states
                    .iter()
                    .find(|row| row.id == *state)
                    .ok_or("attrition state controller unavailable")?
                    .controller;
                panel::state_amount(
                    context.frame,
                    *state,
                    context.division(id)?.roster_owner,
                    controller,
                )
            })();
            amount(
                context,
                out,
                "attrition state",
                "ATTRITION_STATE",
                "ATT",
                value,
            );
        }
        Err(reason) => refuse(out, "attrition state", reason.clone()),
    }
    supply(context, &lookups, id, out);
    fragment(
        context,
        out,
        "attrition upkeep modifiers",
        source_lines(
            context,
            id,
            engine_executable::modifier_keys::UNIT_UPKEEP_ATTRITION_FACTOR,
            -1,
        ),
    );
    match (&moving, &province) {
        (Ok(false), _) => (),
        (Ok(true), Ok(province)) => {
            let gate = (|| {
                let links = context
                    .frame
                    .complete::<tables::CountryLinks>()
                    .map_err(|why| why.to_string())?
                    .iter()
                    .map(|row| (row.id, row.canonical_slot))
                    .collect();
                panel::at_war(context.frame, &links, context.division(id)?.logical_owner)
            })();
            match gate {
                Ok(false) => (),
                Err(why) => refuse(out, "attrition weather", why),
                Ok(true) => {
                    let value = state.clone().and_then(|state| {
                        if state == 0 {
                            return Ok(0);
                        }
                        panel::state(
                            context.frame,
                            state,
                            context.division(id)?.logical_owner,
                            engine_executable::modifier_keys::ATTRITION,
                        )
                    });
                    amount(
                        context,
                        out,
                        "attrition moving state",
                        "ATTRITION_STATE",
                        "ATT",
                        value,
                    );
                    match weather_lines(context, id, *province) {
                        Ok(false) => (),
                        Ok(true) => refuse(
                            out,
                            "attrition weather",
                            format!(
                                "province {province} ordered weather source labels are absent from WeatherProvinces.modifiers"
                            ),
                        ),
                        Err(reason) => refuse(out, "attrition weather", reason),
                    }
                }
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
    fragment(
        context,
        out,
        "attrition country modifiers",
        source_lines(context, id, engine_executable::modifier_keys::ATTRITION, 2),
    );
}

fn amount(
    context: &Context<'_>,
    out: &mut Tooltip,
    label: &str,
    key: &str,
    arg: &str,
    value: Result<i64, String>,
) {
    match value {
        Ok(0) => (),
        Ok(value) => fragment(
            context,
            out,
            label,
            localise(context.loc, key, &[(arg, Arg::Fixed5(value))]),
        ),
        Err(why) => refuse(out, label, why),
    }
}
fn source_lines(context: &Context<'_>, id: u64, key: u32, depth: i32) -> Result<String, String> {
    let unit = context.division(id)?;
    let owner = if (unit.expeditionary_owner as i32) > 0 {
        unit.expeditionary_owner
    } else {
        unit.roster_owner
    };
    let identity = panel::country_identity(context.frame, owner)?;
    let nodes = panel::tree(context.frame, 0, identity, key)?;
    let definitions = context
        .frame
        .complete::<tables::ModifierDefinitions>()
        .map_err(|why| why.to_string())?;
    let definition = definitions
        .iter()
        .find(|row| row.index == key)
        .ok_or_else(|| format!("attrition modifier {key} definition unavailable"))?;
    dashboard_text::modifier_text::breakdown(
        &nodes,
        dashboard_text::modifier_text::Breakdown {
            depth,
            prefix: "",
            suffix: "",
        },
        |node| {
            let name = std::str::from_utf8(&node.name)
                .map_err(|_| "attrition source label is not UTF-8")?;
            let label = if context.loc.language == context.recorded_language {
                name.to_owned()
            } else {
                context.loc.localise(&node.name).map_err(|_| {
                    format!(
                        "attrition source label {name} has no localisation in {}",
                        context.loc.language
                    )
                })?
            };
            let value = dashboard_text::modifier_text::value_text(
                definition,
                node.value,
                2,
                false,
                false,
                |key, _| {
                    let key = std::str::from_utf8(key)
                        .map_err(|_| "attrition value localisation key is not UTF-8")?;
                    localise(context.loc, key, &[])
                },
            )?;
            Ok(if label.is_empty() {
                value
            } else {
                format!("{label}: {value}")
            })
        },
    )
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
            if !out.title.is_empty() && !out.title.starts_with('\n') {
                ensure_newlines(&mut header, 1);
            }
            out.title.insert_str(0, &header);
        }
        Err(reason) => refuse(out, "attrition total", reason),
    }
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
