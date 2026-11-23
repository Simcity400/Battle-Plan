
use super::*;
use replay_data::{rows::Recorded, rows::UnitSupplyDemand, rows::UnitSupplyInputs};
use viewer_foundation::tooltip_text::ensure_newlines;
use viewer_foundation::tooltip_text::state_name;

#[derive(Default)]
struct Origins {
    requested: i64,
    received: i64,
    visible: Vec<(i32, i64)>,
}

struct Aggregate {
    scalars: UnitSupplyInputs,
    origins: Option<Origins>,
}

fn aggregate<'a>(members: impl IntoIterator<Item = &'a UnitSupplyInputs>) -> Aggregate {
    let mut out = Aggregate {
        scalars: UnitSupplyInputs::default(),
        origins: Some(Origins::default()),
    };
    for member in members {
        out.scalars.asked = out.scalars.asked.wrapping_add(member.asked);
        out.scalars.received = out.scalars.received.wrapping_add(member.received);
        out.scalars.from_state = out.scalars.from_state.wrapping_add(member.from_state);
        out.scalars.air_drops = out.scalars.air_drops.wrapping_add(member.air_drops);
        out.scalars.disruption = member.disruption;
        for (sum, value) in [
            (&mut out.scalars.division_demand, &member.division_demand),
            (&mut out.scalars.airplane_demand, &member.airplane_demand),
            (&mut out.scalars.ship_demand, &member.ship_demand),
        ] {
            sum.count = sum.count.wrapping_add(value.count);
            sum.value = sum.value.wrapping_add(value.value);
        }
        match (&mut out.origins, &member.origins) {
            (Some(sum), Some(origins)) => {
                for origin in origins {
                    sum.requested = sum.requested.wrapping_add(origin.requested);
                    sum.received = sum.received.wrapping_add(origin.received);
                    if origin.received > 0 {
                        if let Some((_, amount)) = sum
                            .visible
                            .iter_mut()
                            .find(|(province, _)| *province == origin.province)
                        {
                            *amount = amount.wrapping_add(origin.received);
                        } else {
                            sum.visible.push((origin.province, origin.received));
                        }
                    }
                }
            }
            _ => out.origins = None,
        }
    }
    if let Some(origins) = &mut out.origins {
        origins
            .visible
            .sort_by_key(|(_, amount)| std::cmp::Reverse(*amount));
    }
    out
}

fn round_two(value: i64) -> i64 {
    let shifted = value.wrapping_add(if value > 0 { 500 } else { -500 });
    shifted.wrapping_sub(shifted % 1_000)
}

fn ratio(numerator: i64, denominator: i64) -> Result<i64, String> {
    if denominator == 0 {
        return Ok(100_000);
    }
    if (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        return Ok(0xffff_ffff);
    }
    numerator
        .wrapping_mul(100_000)
        .checked_div(denominator)
        .ok_or_else(|| "native supply fraction traps".into())
}

fn line(context: &Context<'_>, out: &mut Tooltip, label: &str, text: Result<String, String>) {
    super::append(out, context, true, label, text);
}

fn consumer(
    context: &Context<'_>,
    label: &str,
    value: i64,
    spaces: usize,
) -> Result<String, String> {
    localise(
        context.loc,
        "CONSUMER_SUPPLY_TOOLTIP",
        &[
            ("LABEL", Arg::Text(label)),
            ("VAL", Arg::Fixed5(round_two(value))),
        ],
    )
    .map(|text| format!("{}{text}\n", " ".repeat(spaces)))
}

fn footer(
    context: &Context<'_>,
    data: &Aggregate,
    out: &mut Tooltip,
    mut province_name: impl FnMut(i32) -> Result<String, String>,
) {
    let scalar = &data.scalars;
    line(
        context,
        out,
        "supply needed",
        localise(
            context.loc,
            "NEEDED_SUPPLY_WITH_VALUE",
            &[("VAL", Arg::Fixed5(round_two(scalar.asked)))],
        ),
    );
    for (key, demand) in [
        ("SUPPLY_DIVISION_DEMAND", &scalar.division_demand),
        ("SUPPLY_AIRPLANE_DEMAND", &scalar.airplane_demand),
        ("SUPPLY_SHIP_DEMAND", &scalar.ship_demand),
    ] {
        if demand.value > 0 {
            ensure_newlines(&mut out.body, 1);
            line(
                context,
                out,
                "supply demand",
                demand_text(context.loc, key, demand),
            );
        }
    }
    ensure_newlines(&mut out.body, 2);
    let received = (|| {
        let value = round_two(ratio(scalar.received, scalar.asked)?);
        let colour = if value < 100_000 { 'R' } else { 'W' };
        let percent = content_reader_parser::fixed5::fixed5_percent(value, 0, true);
        let percent = format!("\u{11}{colour}{percent}\u{11}!");
        localise(
            context.loc,
            "RECEIVED_SUPPLY_WITH_VALUE",
            &[
                ("VAL", Arg::Fixed5(round_two(scalar.received))),
                ("RATIO", Arg::Text(&percent)),
            ],
        )
        .map(|text| format!("{text}\n"))
    })();
    line(context, out, "supply received", received);
    if scalar.from_state > 0 {
        let text = localise(context.loc, "FROM_STATE", &[])
            .and_then(|label| consumer(context, &label, scalar.from_state, 2));
        line(context, out, "supply state", text);
    }
    match &data.origins {
        None => refuse(
            out,
            "supply origins",
            "recorded consumer origin fragment unavailable",
        ),
        Some(origins) if origins.received > 0 => {
            let text = localise(context.loc, "FROM_SUPPLY_NODES", &[])
                .and_then(|label| consumer(context, &label, origins.received, 2));
            line(context, out, "supply nodes", text);
            for &(province, value) in origins
                .visible
                .iter()
                .take(6)
                .take_while(|(_, value)| *value >= 5_000)
            {
                let text =
                    province_name(province).and_then(|label| consumer(context, &label, value, 4));
                line(context, out, "supply origin name", text);
            }
            if origins.received < origins.requested {
                let text = ratio(origins.received, origins.requested).and_then(|fraction| {
                    let percent = content_reader_parser::fixed5::fixed5_percent(
                        round_two(100_000_i64.wrapping_sub(fraction)),
                        0,
                        true,
                    );
                    localise(
                        context.loc,
                        "SUPPLY_FLOW_REDUCTION",
                        &[("PERCENT", Arg::Text(&percent))],
                    )
                    .map(|text| format!("    {text}\n"))
                });
                line(context, out, "supply flow reduction", text);
            }
        }
        Some(_) => (),
    }
    if scalar.air_drops > 0 {
        let text = localise(context.loc, "FROM_AIR_DROPS", &[])
            .and_then(|label| consumer(context, &label, scalar.air_drops, 2));
        line(context, out, "supply air drops", text);
    }
    disruption(context, out, scalar.disruption);
}

fn disruption(context: &Context<'_>, out: &mut Tooltip, value: i64) {
    if value > 0 {
        line(
            context,
            out,
            "supply disruption",
            localise(
                context.loc,
                "SUPPLY_FLOW_DISRUPTION",
                &[("SUPPLY", Arg::Fixed5(round_two(value)))],
            )
            .map(|text| format!("  {text}\n")),
        );
    }
}

fn demand_text(loc: &Localisation, key: &str, demand: &UnitSupplyDemand) -> Result<String, String> {
    localise(
        loc,
        key,
        &[
            ("AMOUNT", Arg::Int32(demand.count)),
            ("VAL", Arg::Fixed5(round_two(demand.value))),
        ],
    )
    .map(|text| format!("  {text}"))
}

fn province_name(context: &Context<'_>, id: i32) -> Result<String, String> {
    let id = id as u32;
    let provinces = context
        .frame
        .complete::<tables::Provinces>()
        .map_err(|e| e.to_string())?;
    let Some(province) = provinces.iter().find(|row| row.id == id) else {
        return Ok(String::new());
    };
    let overrides = context
        .frame
        .complete::<tables::ProvinceNames>()
        .map_err(|e| e.to_string())?;
    if let Some(row) = overrides
        .iter()
        .find(|row| row.province == id && !row.name.is_empty())
    {
        let text = context.loc.localise(&row.name).map_err(|e| e.to_string())?;
        if !text.is_empty() {
            return Ok(text);
        }
        return std::str::from_utf8(&row.name)
            .map(str::to_owned)
            .map_err(|_| format!("supply province {id} override is not UTF-8"));
    }
    let state = if province.state == 0 {
        None
    } else {
        let states = context
            .frame
            .complete::<tables::States>()
            .map_err(|e| e.to_string())?;
        Some(
            states
                .iter()
                .find(|row| row.id == province.state)
                .ok_or_else(|| {
                    format!("supply province {id} state {} is absent", province.state)
                })?,
        )
    };
    let owner = state.map_or(0, |state| state.owner);
    let slot = if owner == 0 { 0 } else { context.slot(owner)? };
    let tag = content_reader_country_names::country_names::original_or_own_tag(context.frame, slot)?;
    if let Some(name) =
        viewer_runtime::label_reads::label_name(context.loc, None, Some(tag), id)
            .map_err(|e| e.to_string())?
            .filter(|name| !name.is_empty())
    {
        return Ok(name);
    }
    if let Some(state) = state {
        return state_name(
            context.loc,
            context.recorded_language,
            state.id,
            &state.custom_name,
            &state.owner_name,
            || {
                let content = context
                    .content
                    .ok_or("supply province content unavailable")?;
                let key = content
                    .state_name_keys
                    .get(&state.id)
                    .ok_or("supply state content name absent")?
                    .as_ref()
                    .map_err(Clone::clone)?;
                localise(context.loc, key, &[])
            },
        );
    }
    let content = context
        .content
        .ok_or("supply province content unavailable")?;
    let Some(region) = content.region_of.get(&(id)).copied() else {
        return Ok(id.to_string());
    };
    let region = content
        .strategic_regions
        .get(&region)
        .ok_or("supply strategic region absent")?;
    localise(context.loc, &region.name_key, &[])
}

const NOT_RECORDED: &str = "consumer supply inputs were not recorded";
const UNREADABLE: &str = "consumer supply inputs were unreadable when recorded";

pub(super) fn append(context: &Context<'_>, input: &CounterHover, out: &mut Tooltip) {
    let members = (|| {
        let mut members = Vec::new();
        if input.railway_gun {
            let guns = context
                .frame
                .complete::<tables::RailwayGunCounterInputs>()
                .map_err(|e| e.to_string())?;
            for &id in input.members.iter() {
                let Ok(index) = guns.binary_search_by_key(&id, |row| row.gun) else {
                    continue;
                };
                members.push(match &guns[index].supply {
                    Recorded::NotRecorded => Err(NOT_RECORDED.to_owned()),
                    Recorded::Value(value) => value.clone().ok_or_else(|| UNREADABLE.to_owned()),
                });
            }
        } else {
            let divisions = context
                .frame
                .complete::<tables::Divisions>()
                .map_err(|error| error.to_string())?;
            for &id in input.members.iter() {
                if divisions.binary_search_by_key(&id, |row| row.id).is_err() {
                    continue;
                }
                members.push(context.status(id).and_then(|status| {
                    match &status.supply {
                        Recorded::NotRecorded => Err(NOT_RECORDED.to_owned()),
                        Recorded::Value(value) => value
                            .as_ref()
                            .map(|footer| UnitSupplyInputs::from(footer.clone()))
                            .ok_or_else(|| UNREADABLE.to_owned()),
                    }
                }));
            }
        }
        Ok::<_, String>(members)
    })();
    let members = match members {
        Ok(members) => members,
        Err(reason) => {
            refuse(out, "supply detail", reason);
            return;
        }
    };
    let members = members
        .iter()
        .map(|member| member.as_ref().map_err(Clone::clone))
        .collect::<Vec<Result<&UnitSupplyInputs, String>>>();
    let mut result = Tooltip::default();
    member_footer(context, &members, &mut result);
    if !result.body.is_empty() {
        ensure_newlines(&mut out.title, 2);
        out.title.push_str(&result.body);
    }
    for (reason, count) in result.refused {
        *out.refused.entry(reason).or_default() += count;
    }
}

pub(super) fn member_footer(
    context: &Context<'_>,
    members: &[Result<&UnitSupplyInputs, String>],
    out: &mut Tooltip,
) {
    if let Some(reason) = members.iter().find_map(|member| member.as_ref().err()) {
        refuse(out, "supply detail", reason.clone());
        if let Some(Ok(last)) = members.last() {
            disruption(context, out, last.disruption);
        }
        return;
    }
    let data = aggregate(
        members
            .iter()
            .filter_map(|member| member.as_ref().ok().copied()),
    );
    footer(context, &data, out, |province| {
        province_name(context, province)
    });
}

#[cfg(test)]
mod tests;
