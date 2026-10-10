//! Own singleton attrition tooltips from fresh ordered terms.
//! The earlier counter cache cannot supply these details.

use super::*;
use engine::carmy_attrition::{
    COUNTRY_SOURCE, HEAT_SOURCE, STATE_SOURCE, UPKEEP_SOURCE, WEATHER_SOURCE, WINTER_SOURCE,
};
use replay::rows::{ArmyAttritionInputs, AttritionModifierSource, AttritionThermal};

fn fragment(context: &Context<'_>, out: &mut Tooltip, label: &str, text: Result<String, String>) {
    match text.and_then(|text| displayable(context.icons, text)) {
        Ok(text) if !text.is_empty() => {
            crate::combat::text::ensure_newlines(&mut out.title, 1);
            out.title.push_str(&text);
        }
        Ok(_) => (),
        Err(reason) => refuse(out, label, reason),
    }
}

fn missing(field: &str) -> String {
    format!("DivisionStatus.attrition_inputs.{field} was unreadable when recorded")
}

fn scalar(
    context: &Context<'_>,
    out: &mut Tooltip,
    key: &str,
    field: &str,
    value: Option<i64>,
    argument: &str,
) {
    match value {
        Some(0) => (),
        Some(value) => fragment(
            context,
            out,
            field,
            localise(context.loc, key, &[(argument, Arg::Fixed5(value))]),
        ),
        None => refuse(out, field, missing(field)),
    }
}

fn tree(
    context: &Context<'_>,
    source: Option<Option<AttritionModifierSource>>,
    field: &str,
) -> Result<String, String> {
    let source = source
        .ok_or_else(|| missing(field))?
        .ok_or_else(|| format!("attrition {field} formatter source is absent"))?;
    let rows = context
        .frame
        .complete::<tables::AttritionModifierBreakdowns>()
        .map_err(|e| e.to_string())?;
    let row = rows
        .binary_search_by_key(&(source.accumulator, source.key), |row| {
            (row.accumulator, row.key)
        })
        .map(|at| &rows[at])
        .map_err(|_| {
            format!(
                "attrition {field} accumulator {} key {} tree is absent",
                source.accumulator, source.key
            )
        })?;
    let nodes = row
        .nodes
        .as_ref()
        .ok_or_else(|| format!("attrition {field} tree was unreadable when recorded"))?;
    let definitions = context
        .frame
        .complete::<tables::ModifierDefinitions>()
        .map_err(|e| e.to_string())?;
    let definition = definitions
        .binary_search_by_key(&source.key, |row| row.index)
        .map(|at| &definitions[at])
        .map_err(|_| format!("attrition modifier definition {} is absent", source.key))?;
    if definition.feature_selector != 0 {
        if definition.eligibility == 0 {
            return Ok(String::new());
        }
        let features = context
            .frame
            .complete::<tables::SimulationFeatures>()
            .map_err(|e| e.to_string())?;
        let [features] = features else {
            return Err("modifier feature singleton is absent".into());
        };
        if definition.feature_selector & features.dlc_mask == 0 {
            return Ok(String::new());
        }
    }
    dashboard::modifier_text::breakdown(
        nodes,
        dashboard::modifier_text::Breakdown {
            depth: source.depth,
            prefix: "",
            suffix: "",
        },
        |node| {
            let mut name = dashboard::scripted_text::stored_text(
                context.loc,
                context.recorded_language,
                &node.name,
                "attrition modifier source",
            )?;
            if !name.is_empty() {
                name.push_str(": ");
            }
            let value = node.value.wrapping_mul(source.scale) / 100_000;
            name.push_str(
                &crate::combat::modifier_text::value_text(
                    definition,
                    value,
                    2,
                    false,
                    false,
                    context.icons,
                    context.loc,
                )
                .map_err(|e| e.to_string())?,
            );
            displayable(context.icons, name)
        },
    )
}

fn source_block(
    context: &Context<'_>,
    out: &mut Tooltip,
    input: &ArmyAttritionInputs,
    index: usize,
    key: &str,
    field: &str,
    separator: &str,
) {
    // A source-copy failure preserves the separately known native heading.
    fragment(
        context,
        out,
        field,
        localise(context.loc, key, &[]).map(|heading| format!("{heading}{separator}")),
    );
    // The native caller concatenates the source immediately after the separator.
    match tree(context, input.sources[index], field)
        .and_then(|text| displayable(context.icons, text))
    {
        Ok(text) => out.title.push_str(&text),
        Err(reason) => refuse(out, field, reason),
    }
}

/// Include a plus sign at zero; use a fixed-point
/// product before division, red positive / green negative / yellow zero.
fn coloured_percent(value: i64, precision: i32) -> String {
    let scaled = value.wrapping_mul(10_000_000) / 100_000;
    let colour = match scaled.signum() {
        1 => 'R',
        -1 => 'G',
        _ => 'Y',
    };
    let sign = if scaled >= 0 { "+" } else { "" };
    let number = installed_content::fixed5::fixed5_to_string(scaled, precision);
    format!("\u{11}{colour}{sign}{number}%\u{11}!")
}

fn thermal(
    context: &Context<'_>,
    out: &mut Tooltip,
    input: &ArmyAttritionInputs,
    value: Option<Option<AttritionThermal>>,
    index: usize,
    field: &str,
) {
    match value {
        Some(None) => (),
        None => refuse(out, field, missing(field)),
        Some(Some(_)) => {
            fragment(
                context,
                out,
                field,
                tree(context, input.sources[index], field).map(|text| format!(" - {text}")),
            );
            // Country and commander factor text remains independently available
            // even when either numeric factor could not be acquired.
            for (index, label) in [
                (index + 1, "country thermal modifiers"),
                (index + 2, "commander thermal modifiers"),
            ] {
                if input.sources[index] == Some(None) {
                    continue;
                }
                match tree(context, input.sources[index], label)
                    .and_then(|text| displayable(context.icons, text))
                {
                    Ok(text) => out.title.push_str(&text),
                    Err(reason) => refuse(out, label, reason),
                }
            }
        }
    }
}

fn terrain(context: &Context<'_>, out: &mut Tooltip, input: &ArmyAttritionInputs) {
    let terrain = match &input.terrain {
        Some(None) => return,
        Some(Some(terrain)) => terrain,
        None => {
            refuse(out, "attrition terrain", missing("terrain"));
            return;
        }
    };
    let before_equipment = input
        .upkeep
        .zip(terrain.attrition)
        .map(|(upkeep, terrain)| upkeep.wrapping_add(terrain));
    match before_equipment {
        Some(0) => (), // Native also suppresses the equipment line in this case.
        None => refuse(
            out,
            "attrition terrain",
            missing("terrain.attrition or upkeep"),
        ),
        Some(value) => {
            let text = (|| {
                let key = terrain
                    .name
                    .as_deref()
                    .ok_or_else(|| missing("terrain.name"))?;
                let name = context.loc.localise(key).map_err(|e| e.to_string())?;
                localise(
                    context.loc,
                    "ATTRITION_TERRAIN",
                    &[("TERR", Arg::Text(&name)), ("ATT", Arg::Fixed5(value))],
                )
            })();
            fragment(context, out, "attrition terrain", text);
            // Display the unweighted sum before the native floor. Repeated
            // current-equipment rows are counted once each, regardless of amount.
            scalar(
                context,
                out,
                "ATTRITION_EQUIPMENT",
                "attrition equipment",
                terrain.equipment,
                "ATT",
            );
        }
    }
}

fn details(
    context: &Context<'_>,
    input: &ArmyAttritionInputs,
    out: &mut Tooltip,
    border_ratio: Result<Option<i64>, String>,
) {
    match input.province {
        Some(None) => return,
        None => {
            refuse(out, "attrition location", missing("province"));
            return;
        }
        Some(Some(_)) => (),
    }
    scalar(
        context,
        out,
        "ATTRITION_UPKEEP",
        "attrition upkeep",
        input.upkeep,
        "VALUE",
    );
    terrain(context, out, input);
    match input.state {
        Some(value) => scalar(
            context,
            out,
            "ATTRITION_STATE",
            "attrition state",
            value.or(Some(0)),
            "ATT",
        ),
        None => refuse(out, "attrition state", missing("state")),
    }
    match input.supply {
        Some(supply) if supply.penalty == Some(0) => (),
        Some(supply) => {
            let text = (|| {
                let value = supply
                    .penalty
                    .ok_or_else(|| missing("supply.penalty"))?
                    .wrapping_mul(
                        supply
                            .attrition_factor
                            .ok_or_else(|| missing("supply.attrition_factor"))?,
                    )
                    / 100_000;
                localise(
                    context.loc,
                    "ATTRITION_SUPPLY",
                    &[("ATT", Arg::Text(&coloured_percent(value, 2)))],
                )
            })();
            fragment(context, out, "attrition supply", text);
        }
        None => refuse(out, "attrition supply", missing("supply")),
    }
    match input.country_upkeep_modifier {
        Some(0) => (),
        Some(_) => source_block(
            context,
            out,
            input,
            UPKEEP_SOURCE,
            "ATTRITION_UPKEEP_MODIFIERS",
            "attrition upkeep modifiers",
            "\n",
        ),
        None => refuse(
            out,
            "attrition upkeep modifiers",
            missing("country_upkeep_modifier"),
        ),
    }
    match input.moving_state {
        Some(None | Some(0)) => (),
        Some(Some(_)) => source_block(
            context,
            out,
            input,
            STATE_SOURCE,
            "ATTRITION_STATE_MODIFIERS",
            "attrition moving state",
            "\n",
        ),
        None => refuse(out, "attrition moving state", missing("moving_state")),
    }
    match input.weather {
        Some(None) => (),
        None => refuse(out, "attrition weather", missing("weather")),
        Some(Some(weather)) => {
            match weather.attrition {
                Some(0) => (),
                Some(_) => source_block(
                    context,
                    out,
                    input,
                    WEATHER_SOURCE,
                    "ATTRITION_WEATHER_MODIFIERS",
                    "attrition weather modifiers",
                    "\n    ",
                ),
                None => refuse(
                    out,
                    "attrition weather modifiers",
                    missing("weather.attrition"),
                ),
            }
            thermal(
                context,
                out,
                input,
                weather.heat,
                HEAT_SOURCE,
                "attrition heat",
            );
            thermal(
                context,
                out,
                input,
                weather.winter,
                WINTER_SOURCE,
                "attrition winter",
            );
        }
    }
    match input.training_attrition {
        Some(None) => (),
        Some(Some(value)) => fragment(
            context,
            out,
            "attrition training",
            localise(
                context.loc,
                "TRAINING_ATTRITION",
                &[("VALUE", Arg::Text(&coloured_percent(value, 1)))],
            )
            .map(|text| format!("{text}\n")),
        ),
        None => refuse(out, "attrition training", missing("training_attrition")),
    }
    match input.border {
        Some(None) => (),
        None => refuse(out, "attrition border", missing("border")),
        Some(Some(border)) => {
            fragment(
                context,
                out,
                "attrition border heading",
                localise(context.loc, "BORDER_WAR_HEADER", &[]).map(|text| format!("{text}\n")),
            );
            let value = border_ratio
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|value| value.ok_or_else(|| "attrition border ratio is absent".into()))
                .and_then(|value| {
                    border
                        .factor
                        .ok_or_else(|| missing("border.factor"))
                        .map(|factor| value.wrapping_mul(factor) / 100_000)
                });
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
    }
    let country_tree = tree(
        context,
        input.sources[COUNTRY_SOURCE],
        "attrition country modifiers",
    );
    match country_tree {
        Ok(why) if why.is_empty() => (),
        Ok(why) => match input.country_attrition_modifier {
            Some(value) => {
                let multiplier = 100_000_i64.wrapping_add(value).max(0);
                let number = installed_content::fixed5::fixed5_to_string(multiplier, 2);
                fragment(
                    context,
                    out,
                    "attrition country modifiers",
                    localise(
                        context.loc,
                        "ATTRITION_MULMOD_BY",
                        &[("MOD", Arg::Text(&number)), ("WHY", Arg::Text(&why))],
                    )
                    .map(|text| format!("{text}\n")),
                );
            }
            None => {
                refuse(
                    out,
                    "attrition country multiplier",
                    missing("country_attrition_modifier"),
                );
                fragment(context, out, "attrition country modifiers", Ok(why));
            }
        },
        Err(reason) => refuse(out, "attrition country modifiers", reason),
    }
    // Numeric total ignores label and source-tree faults, and never substitutes
    // the cached status attrition value for a missing fresh term.
    let total = border_ratio.and_then(|ratio| input.evaluate(ratio).map_err(str::to_owned));
    match total
        .and_then(|value| {
            localise(
                context.loc,
                "ATTRITION_TOTAL",
                &[("TOT", Arg::Fixed5(value))],
            )
        })
        .and_then(|text| displayable(context.icons, text))
    {
        Ok(mut header) => {
            // The game starts the detail body on a new line before prepending total.
            if !out.title.is_empty() && !out.title.starts_with('\n') {
                crate::combat::text::ensure_newlines(&mut header, 1);
            }
            out.title.insert_str(0, &header);
        }
        Err(reason) => refuse(out, "attrition total", reason),
    }
    // Trim completed tooltip text at line-feed, tab, carriage-return and space boundaries.
    // Keep interior separators; boundary breaks are not display rows.
    out.title = out.title.trim_matches(['\n', '\t', '\r', ' ']).to_owned();
}

pub(super) fn append(context: &Context<'_>, id: u64, out: &mut Tooltip) {
    let input = context.status(id).and_then(|status| {
        status
            .attrition_inputs
            .require("DivisionStatus.attrition_inputs")
    });
    let input = match input {
        Ok(input) => input,
        Err(reason) => {
            refuse(out, "attrition detail", reason);
            return;
        }
    };
    let border_ratio = match input.border {
        Some(Some(border)) => (|| {
            border.factor.ok_or_else(|| missing("border.factor"))?;
            let country = border.country.ok_or_else(|| missing("border.country"))?;
            let state = border.state.ok_or_else(|| missing("border.state"))?;
            let content = context
                .content
                .ok_or("attrition border content unavailable")?;
            let links = context
                .links
                .as_ref()
                .map_err(Clone::clone)?
                .iter()
                .map(|row| (row.id, row.canonical_slot))
                .collect();
            content
                .province_hover
                .border_ratio(context.frame, country, state, &links)
                .map(Some)
        })(),
        _ => Ok(None),
    };
    details(context, input, out, border_ratio);
}

#[cfg(test)]
mod tests;
