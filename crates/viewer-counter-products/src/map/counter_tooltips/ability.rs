
use dashboard_effects::ScriptedEffects;
use super::*;
use replay_data::{rows::ActiveAbilityRow, rows::UnitLeaderRow};
use std::collections::BTreeSet;
use viewer_foundation::tooltip_text::ensure_newlines;

const ONE: i64 = 100_000;

pub(super) fn append(
    context: &Context<'_>,
    input: &CounterHover,
    country: u32,
    ordinal: u32,
    out: &mut Tooltip,
) {
    let (reference, leader) = match first_commander(context, input) {
        Ok(Some(leader)) => leader,
        Ok(None) => return,
        Err(why) => {
            refuse(out, "ability commander", why);
            return;
        }
    };
    let definition: Result<
        (
            &ActiveAbilityRow,
            &content_reader_abilities::abilities::Definition,
        ),
        String,
    > = (|| {
        let row = active_row(context, country, ordinal)?;
        let content = context.content.ok_or("ability content is unavailable")?;
        Ok((row, content.ability_definition(&row.token)?))
    })();
    let (row, definition) = match definition {
        Ok(definition) => definition,
        Err(why) => {
            refuse(out, "ability definition", why);
            return;
        }
    };
    super::append(
        out,
        context,
        false,
        "ability name",
        context
            .loc
            .localise_with(&definition.name, &[])
            .map_err(|error| error.to_string())
            .and_then(|name| {
                localise(
                    context.loc,
                    "ABILITY_TOOLTIP_NAME",
                    &[("NAME", Arg::Text(&name))],
                )
            }),
    );
    ensure_newlines(&mut out.title, 1);
    let description = scoped_description(context, reference, leader, definition);
    let description_nonempty = match description {
        Ok(description) => {
            let known_nonempty = !description.text.is_empty();
            let known_empty = !known_nonempty && description.refusals.is_empty();
            for (why, count) in description.refusals {
                *out.refused
                    .entry(format!("counter ability description: {why}"))
                    .or_default() += count;
            }
            super::append(
                out,
                context,
                false,
                "ability description",
                Ok(description.text),
            );
            if known_nonempty {
                Some(true)
            } else if known_empty {
                Some(false)
            } else {
                None
            }
        }
        Err(why) => {
            refuse(out, "ability description", why);
            None
        }
    };
    let communications = if description_nonempty == Some(false) {
        Ok(String::new())
    } else {
        communications(context, leader).and_then(|text| {
            if description_nonempty == Some(true) || text.is_empty() {
                Ok(text)
            } else {
                Err("native description's nonempty gate is unavailable".into())
            }
        })
    };
    super::append(
        out,
        context,
        true,
        "ability prose",
        context
            .loc
            .localise_with(&definition.description, &[])
            .map_err(|error| error.to_string()),
    );
    finish(
        context,
        out,
        communications,
        late_join(context, reference, leader, &row.token, || {
            formation_count(context, leader)
        }),
    );
}

fn scoped_description(
    context: &Context<'_>,
    reference: u64,
    leader: &UnitLeaderRow,
    definition: &content_reader_abilities::abilities::Definition,
) -> Result<dashboard_effects::effect_description::Description, String> {
    use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, ScriptedText};

    let owner = context.slot(leader.owner)?;
    let content = context.content.ok_or("ability content is unavailable")?;
    let scripted = content.scripted.as_ref().as_ref().map_err(Clone::clone)?;
    let text = ScriptedText::new(context.frame, context.loc, scripted, context.vision.ours())
        .viewing(context.vision.ours())
        .with_nato_counters(context.nato);
    let effects = scripted.effects()?;
    let effect_context = dashboard_effects::effect_description::Context::new(
        &text,
        context.loc,
        context.vision.ours(),
        "common/abilities",
        effects.view(None),
    );
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::CountryUnitLeader {
        country: owner,
        leader: reference,
    })]);
    Ok(dashboard_effects::effect_description::ability_description(
        definition,
        &effect_context,
        &mut scopes,
    ))
}

fn formation_reference(leader: &UnitLeaderRow) -> Result<u64, String> {
    leader
        .formation_reference
        .require("UnitLeaders.formation_reference")?
        .ok_or_else(|| "UnitLeaders.formation_reference was unreadable when recorded".into())
}

fn formation_count(context: &Context<'_>, leader: &UnitLeaderRow) -> Result<i32, String> {
    let reference = formation_reference(leader)?;
    if reference == 0 {
        return Ok(0);
    }
    let mut total = 0_i32;
    let mut stack = vec![(reference, false)];
    let mut active = BTreeSet::new();
    while let Some((reference, exit)) = stack.pop() {
        if exit {
            active.remove(&reference);
            continue;
        }
        if !active.insert(reference) {
            return Err(format!(
                "CountFormationDivisions has a recursive group cycle at {reference}"
            ));
        }
        stack.push((reference, true));
        let group = context.orders_group(reference)?;
        if group.is_army_group != 0 {
            let children = context
                .frame
                .complete::<tables::ArmyGroupMembers>()
                .map_err(|error| error.to_string())?;
            let first = children.partition_point(|row| row.group < reference);
            let end = children.partition_point(|row| row.group <= reference);
            stack.extend(
                children[first..end]
                    .iter()
                    .rev()
                    .map(|row| (row.child, false)),
            );
        } else {
            let members = context
                .frame
                .complete::<tables::OrdersGroupMembers>()
                .map_err(|error| error.to_string())?;
            let first = members.partition_point(|row| row.group < reference);
            let end = members.partition_point(|row| row.group <= reference);
            for row in &members[first..end] {
                let division = context.division(row.unit)?;
                let templates = context
                    .frame
                    .complete::<tables::DivisionTemplateIcons>()
                    .map_err(|error| error.to_string())?;
                let template = templates
                    .binary_search_by_key(&division.template, |row| row.template)
                    .map(|index| &templates[index])
                    .map_err(|_| {
                        format!(
                            "DivisionTemplateIcons template {} is absent",
                            division.template
                        )
                    })?;
                let count = template
                    .formation_division_count
                    .require("DivisionTemplateIcons.formation_division_count")?
                    .ok_or("DivisionTemplateIcons.formation_division_count was unreadable when recorded")?;
                total = total.wrapping_add(count);
            }
        }
    }
    Ok(total)
}

fn communications(context: &Context<'_>, leader: &UnitLeaderRow) -> Result<String, String> {
    let Some(leader) = communication_leader(context, leader)? else {
        return Ok(String::new());
    };
    if (leader.category as u32) >= 2 {
        return Ok(String::new());
    }
    let features = context
        .frame
        .complete::<tables::SimulationFeatures>()
        .map_err(|error| error.to_string())?;
    let features = features
        .first()
        .ok_or("SimulationFeatures singleton is absent")?;
    if features.dlc_mask & engine_dlc::dlc_feature_state::THUNDER_AT_OUR_GATES == 0 {
        return Ok(String::new());
    }
    let reference = formation_reference(leader)?;
    if reference == 0 {
        return Ok(String::new());
    }
    let group = context.orders_group(reference)?;
    if group.is_army_group != 0 {
        return Ok(String::new());
    }
    if group.leader == 0 || group.hq_provider == 0 {
        let content = context.content.ok_or("ability content is unavailable")?;
        let multiplier = content
            .ability_no_hq_scaling
            .as_ref()
            .copied()
            .map_err(Clone::clone)?;
        return communication_text(context.loc, multiplier, true);
    }
    context
        .frame
        .complete::<tables::ArmyOrders>()
        .map_err(|error| error.to_string())?;
    context
        .frame
        .complete::<tables::ArmyOrderPlans>()
        .map_err(|error| error.to_string())?;
    context
        .frame
        .complete::<tables::OrdersGroupPlans>()
        .map_err(|error| error.to_string())?;
    let orders =
        counter_inputs::OrdersIndex::read(context.frame).map_err(|error| error.to_string())?;
    let kind = orders
        .root_order_type(group.hq_provider, group.handle)
        .ok_or("HQ member's root order is unavailable")?;
    let Some(kind @ (2 | 3)) = kind else {
        return Ok(String::new());
    };
    let input = context.group_input(reference)?;
    let distance = communication_distance(input, kind)?;
    let content = context.content.ok_or("ability content is unavailable")?;
    let scaling = content
        .ability_comms_scaling
        .as_ref()
        .map_err(Clone::clone)?;
    communication_text(context.loc, communication_value(distance, scaling)?, false)
}

fn communication_leader<'a>(
    context: &Context<'a>,
    original: &UnitLeaderRow,
) -> Result<Option<&'a UnitLeaderRow>, String> {
    use dashboard_text::character_scope::{Tables, UnitLeader};

    if original.character_type == 0 && original.character_id == 0 {
        return Ok(None);
    }
    match Tables::read(context.frame)
        .unit_leader(original.character_id)
        .map_err(|why| why.to_string())?
    {
        Some(UnitLeader::Direct(row)) => {
            let reference = u64::from(row.id) << 32 | u64::from(row.reference_type);
            let rows = context
                .frame
                .complete::<tables::UnitLeaders>()
                .map_err(|why| why.to_string())?;
            rows.iter()
                .find(|candidate| candidate.character_id == row.character_id)
                .map(Some)
                .ok_or_else(|| format!("UnitLeaders leader {reference} is absent"))
        }
        Some(UnitLeader::Referenced(row)) if (row.category as u32) >= 2 => Ok(None),
        Some(UnitLeader::Referenced(_)) => {
            Err("the character's referenced leader has no recorded army formation operands".into())
        }
        None => Ok(None),
    }
}

fn communication_distance(
    input: &replay_data::rows::OrdersGroupCounterInputRow,
    kind: i32,
) -> Result<i32, String> {
    let (distance, field) = if kind == 2 {
        (&input.hq_distance, "OrdersGroupCounterInputs.hq_distance")
    } else {
        (
            &input.invasion_source_distance,
            "OrdersGroupCounterInputs.invasion_source_distance",
        )
    };
    distance
        .require(field)?
        .ok_or_else(|| format!("{field} was unreadable when recorded"))
}

fn first_commander<'a>(
    context: &Context<'a>,
    input: &CounterHover,
) -> Result<Option<(u64, &'a UnitLeaderRow)>, String> {
    let Some(&first) = input.members.first() else {
        return Ok(None);
    };
    let group = context.division(first)?.orders_group;
    if group == 0 {
        return Ok(None);
    }
    let reference = context.orders_group(group)?.leader;
    if reference == 0 {
        return Ok(None);
    }
    let leaders = context
        .frame
        .complete::<tables::UnitLeaders>()
        .map_err(|error| error.to_string())?;
    let leader = leaders
        .binary_search_by_key(&reference, |row| {
            u64::from(row.id) << 32 | u64::from(row.reference_type)
        })
        .map(|index| &leaders[index])
        .map_err(|_| format!("UnitLeaders leader {reference} is absent"))?;
    Ok(Some((reference, leader)))
}

fn active_row<'a>(
    context: &Context<'a>,
    country: u32,
    ordinal: u32,
) -> Result<&'a ActiveAbilityRow, String> {
    let rows = context
        .frame
        .complete::<tables::ActiveAbilities>()
        .map_err(|error| error.to_string())?;
    rows.binary_search_by_key(&(country, ordinal), |row| (row.country, row.ordinal))
        .map(|index| &rows[index])
        .map_err(|_| format!("ActiveAbilities row {country}:{ordinal} is absent"))
}

fn late_join_multiplier(activation_count: i32, current_count: i32) -> i64 {
    (i64::from(activation_count).wrapping_mul(10_000_000_000)
        / (i64::from(current_count.max(1)) * ONE))
        .min(ONE)
}

fn late_join(
    context: &Context<'_>,
    reference: u64,
    leader: &UnitLeaderRow,
    token: &[u8],
    current_count: impl FnOnce() -> Result<i32, String>,
) -> Result<i64, String> {
    let owner = context.slot(leader.owner)?;
    let rows = context
        .frame
        .complete::<tables::ActiveAbilities>()
        .map_err(|error| error.to_string())?;
    for row in rows {
        if row.caster == Some(reference)
            && row.token == token
            && context.slot(row.country)? == owner
        {
            let count = if (leader.category as u32) < 2 {
                current_count()?
            } else {
                1
            };
            return Ok(late_join_multiplier(row.division_count, count));
        }
    }
    Ok(ONE)
}

fn communication_value(distance: i32, scaling: &[i64]) -> Result<i64, String> {
    let last = scaling
        .last()
        .copied()
        .ok_or("NMilitary.ABILITY_COMMS_SCALING is empty; native lookup has no final entry")?;
    Ok(usize::try_from(distance)
        .ok()
        .and_then(|index| scaling.get(index).copied())
        .unwrap_or(last))
}

fn communication_text(loc: &Localisation, multiplier: i64, no_hq: bool) -> Result<String, String> {
    if multiplier == ONE {
        return Ok(String::new());
    }
    let value = localise(
        loc,
        "COMMS_MULT_TEXT",
        &[("MULT", Arg::Fixed5(multiplier.wrapping_sub(ONE)))],
    )?;
    localise(
        loc,
        if no_hq {
            "COMMS_MULT_NO_HQ"
        } else {
            "COMMS_MULT"
        },
        &[("MULT_TEXT", Arg::Text(&value))],
    )
}

fn finish(
    context: &Context<'_>,
    out: &mut Tooltip,
    communications: Result<String, String>,
    late_join: Result<i64, String>,
) {
    match communications.and_then(|text| displayable(context.icons, text)) {
        Ok(text) if text.is_empty() => {}
        Ok(text) => {
            ensure_newlines(&mut out.title, 1);
            out.title.push_str(&text);
        }
        Err(why) => refuse(out, "ability communications", why),
    }
    match late_join {
        Ok(ONE) => {}
        Ok(multiplier) => match localise(
            context.loc,
            "ABILITY_TOOLTIP_LATE_JOINER_MULT",
            &[("MULT", Arg::Fixed5(ONE.wrapping_sub(multiplier)))],
        )
        .and_then(|text| displayable(context.icons, text))
        {
            Ok(text) => {
                ensure_newlines(&mut out.title, 2);
                out.title.push_str(&text);
            }
            Err(why) => refuse(out, "ability late join", why),
        },
        Err(why) => refuse(out, "ability late join", why),
    }
    if !out.body.is_empty() && out.title.ends_with('\n') {
        out.title.pop();
    }
}

#[cfg(test)]
#[path = "ability/tests.rs"]
mod tests;
