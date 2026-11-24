use super::super::tooltips::{Described, hover_text, set_hover, tactic_description};
use super::*;
use content_reader_parser::fixed5::fixed5_percent;

fn counter_id(content: &Content, definition: &Definition) -> Evaluation<i32> {
    let name = field(&definition.items, "countered_by")
        .and_then(|item| item.value.scalar())
        .map(c_string)
        .unwrap_or_default();
    if name.is_empty() {
        return Ok(0);
    }
    content.named_id(name)
}

pub(super) fn main(
    icons: &dyn TextIcons,
    loc: &Localisation,
    content: &Content,
    candidate: &Candidate,
    definition: &Definition,
    unavailable: &mut BTreeSet<String>,
) -> Evaluation<String> {
    let mut text = tactic_description(
        icons,
        loc,
        content,
        Described::Content {
            id: candidate.tactic_id,
            token: &candidate.token,
        },
        definition.attacker,
        false,
    )?
    .0;
    text.push('\n');
    let countered_by = (|| {
        let counter = counter_id(content, definition)?;
        if counter == 0 {
            return Ok(String::new());
        }
        let entry = content
            .registry
            .iter()
            .find(|entry| entry.id == counter)
            .ok_or_else(|| format!("counter tactic ID {counter} is outside the registry"))?;
        let mut text = localize_named(icons, loc, "TACTIC_COUNTERED_BY", "NAME", &entry.token)?;
        text.push('\n');
        Ok(text)
    })();
    let counters = (|| {
        let mut counters = Vec::new();
        if let Some(why) = &content.registry_unsettled {
            return Err(format!(
                "the tactics this entry counters are unknown: {why}"
            ));
        }
        for entry in &content.registry {
            let definition = entry.definition.as_ref().map_err(Clone::clone)?;
            if counter_id(content, definition)? == candidate.tactic_id {
                counters.push(token(icons, loc, &entry.token)?);
            }
        }
        if counters.is_empty() {
            return Ok(String::new());
        }
        let mut text = builtin(icons, loc, "TACTIC_COUNTERS", &[])?;
        if counters.len() > 1 {
            text.push('\n');
        }
        for name in counters {
            text.push_str(&format!("\u{11}Y  {name}\u{11}!\n"));
        }
        Ok(text)
    })();
    for (part, fragment) in [("countered-by", countered_by), ("counters", counters)] {
        append(&mut text, &candidate.token, part, fragment, unavailable);
    }
    Ok(text)
}

fn append(
    text: &mut String,
    tactic: &[u8],
    part: &str,
    fragment: Evaluation<String>,
    unavailable: &mut BTreeSet<String>,
) {
    match fragment {
        Ok(fragment) => text.push_str(&fragment),
        Err(why) => {
            unavailable.insert(format!(
                "tactic {}'s {part} hover: {why}",
                String::from_utf8_lossy(tactic)
            ));
        }
    }
}

pub(super) fn locked(
    icons: &dyn TextIcons,
    loc: &Localisation,
    sources: &[String],
) -> Evaluation<String> {
    let mut text = if sources.is_empty() {
        builtin(icons, loc, "TACTICS_LOCKED_BY_NOT_HANDLED", &[])?
    } else {
        let names = sources
            .iter()
            .map(|source| format!("\n{source}"))
            .collect::<String>();
        builtin(
            icons,
            loc,
            "TACTICS_LOCKED_BY_TECH",
            &[("TECH", Argument::Text(&names))],
        )?
    };
    text.push('\n');
    Ok(text)
}

#[expect(clippy::too_many_arguments, reason = "the text resources, the context, the candidate and its definition, and its weights")]
pub(super) fn status(
    icons: &dyn TextIcons,
    loc: &Localisation,
    context: &Context<'_>,
    candidate: &Candidate,
    definition: &Definition,
    weights: &[Option<i64>],
    index: usize,
    unavailable: &mut BTreeSet<String>,
) -> Evaluation<String> {
    if !context.check(&definition.blocks("trigger"))? {
        return builtin(icons, loc, "TACTIC_CONDITIONS_NOT_MET", &[]);
    }
    let weight = weights[index].ok_or("the tactic's selection weight is unavailable")?;
    if weight <= 0 {
        return Ok(String::new());
    }
    let total = weights.iter().try_fold(0_i64, |total, weight| {
        Ok::<_, String>(
            total.wrapping_add(weight.ok_or("the total selection weight is unavailable")?),
        )
    });
    let chance = total.clone().and_then(|total| {
        let chance = if total == 0 || weight as u64 > 0x7fff_ffff_ffff_fffe {
            0xffff_ffff
        } else {
            weight
                .wrapping_mul(100_000)
                .checked_div(total)
                .ok_or("selection chance division would trap")?
        };
        Ok(fixed5_percent(chance, 2, true))
    });
    let mut text = String::new();
    let mut append =
        |part: &str, fragment| append(&mut text, &candidate.token, part, fragment, unavailable);
    append(
        "chance",
        chance.as_ref().map_err(Clone::clone).and_then(|chance| {
            builtin(
                icons,
                loc,
                "TACTIC_CHANCE_OF_SELECTION",
                &[("CHANCE", Argument::Text(chance))],
            )
        }),
    );
    append(
        "total weight",
        total.clone().and_then(|total| {
            builtin(
                icons,
                loc,
                "TACTIC_TOTAL_WEIGHT",
                &[("WEIGHT", Argument::Fixed5(total))],
            )
        }),
    );
    append(
        "selection weight",
        builtin(
            icons,
            loc,
            "TACTIC_WEIGHT_OF_TACTIC",
            &[("WEIGHT", Argument::Fixed5(weight))],
        ),
    );
    let fields = definition.blocks("base");
    let base_fields: Vec<_> = fields
        .iter()
        .filter(|item| !item.keyword_is("modifier"))
        .cloned()
        .collect();
    let evaluator = Evaluator::new(&context.world, Some(context));
    let base = evaluator
        .mean_time_to_happen("base weight", &base_fields, Scopes::combat())
        .map_err(|why| why.to_string())?;
    let scoped = evaluator.mean_time_to_happen("base weight", &fields, Scopes::combat());
    append(
        "base weight",
        builtin(
            icons,
            loc,
            "TACTICS_BASE_WEIGHT",
            &[("WEIGHT", Argument::Fixed5(base))],
        ),
    );
    if !scoped.as_ref().is_ok_and(|scoped| *scoped == base) {
        append(
            "scope weight",
            scoped.map_err(|why| why.to_string()).and_then(|scoped| {
                builtin(
                    icons,
                    loc,
                    "TACTICS_SCOPE_WEIGHT",
                    &[("WEIGHT", Argument::Fixed5(scoped))],
                )
            }),
        );
    }
    for (preference, key) in [
        "TACTIC_WEIGHT_MODIFIER_PREFERRED_TACTIC",
        "TACTIC_WEIGHT_MODIFIER_ARMY_GENERAL_PREFERRED_TACTIC",
        "TACTIC_WEIGHT_MODIFIER_FIELD_MARSHAL_PREFERRED_TACTIC",
    ]
    .into_iter()
    .enumerate()
    {
        if context.detail.tactic_inputs.preferred_tactic_ids[context.side][preference]
            == candidate.tactic_id
        {
            append(
                key,
                context
                    .preferred_weight_factor(preference)
                    .and_then(|value| {
                        let value = fixed5_percent(value, 0, true);
                        builtin(icons, loc, key, &[("MODIFIER", Argument::Text(&value))])
                    }),
            );
        }
    }
    append(
        "chance calculation",
        total.and_then(|total| {
            chance.and_then(|chance| {
                builtin(
                    icons,
                    loc,
                    "TACTIC_CHANCE_CALCULATION",
                    &[
                        ("WEIGHT", Argument::Fixed5(weight)),
                        ("TOTAL_WEIGHT", Argument::Fixed5(total)),
                        ("CHANCE", Argument::Text(&chance)),
                    ],
                )
            })
        }),
    );
    Ok(text)
}

pub(super) fn requirements(
    icons: &dyn TextIcons,
    loc: &Localisation,
    tactic: &[u8],
    body: Evaluation<String>,
    unavailable: &mut BTreeSet<String>,
) -> Evaluation<String> {
    let mut text = builtin(icons, loc, "TACTICS_ACTIVATION_REQUIREMENTS", &[])?;
    text.push('\n');
    let body = body.map(|body| {
        let mut indented = String::new();
        for line in body.split_inclusive('\n') {
            if line != "\n" {
                indented.push_str("  ");
            }
            indented.push_str(line);
        }
        indented
    });
    append(
        &mut text,
        tactic,
        "requirement descriptions",
        body,
        unavailable,
    );
    Ok(text)
}

pub(super) fn bind(
    row: &mut Values,
    tactic: &str,
    main: Evaluation<String>,
    requirements: Evaluation<String>,
    status: Evaluation<String>,
    unavailable: &mut BTreeSet<String>,
) {
    let mut fragment = |name: &str, result: Evaluation<String>| {
        result
            .map_err(|why| {
                unavailable.insert(format!("tactic {tactic}'s {name} hover: {why}"));
            })
            .ok()
    };
    let main = fragment("immediate", main);
    let mut delayed = fragment("requirements", requirements).unwrap_or_default();
    if let Some(status) = fragment("selection", status) {
        delayed.push_str(&status);
    }
    set_hover(
        row,
        "",
        main.as_deref().and_then(hover_text),
        hover_text(&delayed),
    );
}

#[cfg(test)]
mod tests;
