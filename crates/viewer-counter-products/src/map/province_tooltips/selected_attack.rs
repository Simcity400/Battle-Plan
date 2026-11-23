use viewer_foundation::modifier_names::modifier_name_key;
use viewer_foundation::tooltip_text::localise as tooltip_text_localise;
#[cfg(test)]
mod tests;

use super::{
    Arg, Context, Tooltip, clock, localise,
    selection::{line, refuse},
};
use content_reader_unit_path::{unit_path::Fact, unit_path::SelectedNeighbour, unit_path::StraitFleet};
use replay_data::{rows, rows::TableSchema, rows::tables};
use replay_derive::{
    attack_derive, attack_derive::FrameInputs, attack_derive::Preview, attack_derive::Row,
    attack_derive::Terrain,
};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeSet;
use crate::map::division_eta::CounterQueries;
use replay_derive::directed_wars::Owner;
use dashboard_text::table_access::TokenNames;

fn keyed<'a, T: TableSchema>(
    frame: &'a DecodedFrame,
    key: u64,
    key_of: impl Fn(&T::Row) -> u64,
    name: &str,
) -> Fact<&'a T::Row> {
    let rows = frame.complete::<T>().map_err(|e| e.to_string())?;
    rows.binary_search_by_key(&key, key_of)
        .map(|i| &rows[i])
        .map_err(|_| format!("{name} member {key} is absent"))
}

fn division(frame: &DecodedFrame, handle: u64) -> Fact<&rows::DivisionRow> {
    keyed::<tables::Divisions>(frame, handle, |row| row.id, "Divisions")
}

fn template(frame: &DecodedFrame, handle: u64) -> Fact<u64> {
    Ok(division(frame, handle)?.template)
}

fn same_current(locations: &[Fact<u32>]) -> Fact<Option<u32>> {
    let mut first = None;
    let mut missing = None;
    for location in locations {
        match location {
            Ok(current) => match first {
                Some(first) if first != *current => return Ok(None),
                None => first = Some(*current),
                _ => {}
            },
            Err(reason) => {
                missing.get_or_insert_with(|| reason.clone());
            }
        }
    }
    if let Some(reason) = missing {
        Err(reason)
    } else {
        Ok(first)
    }
}

fn first_template(
    identity: Fact<u64>,
    seen: &mut BTreeSet<u64>,
    earlier_unknown: &mut bool,
) -> Fact<Option<u64>> {
    let template = match identity {
        Ok(template) => template,
        Err(reason) => {
            *earlier_unknown = true;
            return Err(reason);
        }
    };
    if !seen.insert(template) {
        return Ok(None);
    }
    if *earlier_unknown {
        return Err("an earlier unknown template may own this first representative".into());
    }
    Ok(Some(template))
}

fn retained_edge(current: u32, route: &[u32]) -> Option<(u32, u32)> {
    let target = *route.last()?;
    let source = if route.len() > 1 {
        route[route.len() - 2]
    } else {
        current
    };
    Some((source, target))
}

fn blocked(
    context: &Context<'_>,
    queries: &CounterQueries<'_>,
    handle: u64,
    current: u32,
    target: u32,
) -> Fact<bool> {
    let Some(SelectedNeighbour { through }) = context.reads.selected_neighbour(current, target)?
    else {
        return Ok(false);
    };
    let Some(through) = through? else {
        return Ok(false);
    };
    let owner = division(context.frame, handle)?.logical_owner;
    Ok(queries.selected_strait(through, owner)? == StraitFleet::Visible)
}

struct Content<'a> {
    context: &'a Context<'a>,
}

impl attack_derive::Context for Content<'_> {
    fn terrain(&self, province: u32) -> Fact<Terrain> {
        let movement = self
            .context
            .content
            .movement
            .as_deref()
            .ok_or("selected attack terrain content is unavailable")?;
        let index = movement
            .map
            .terrain(province)
            .ok_or_else(|| format!("province {province} has no content terrain"))?
            .index;
        let terrains = self.context.content.movement_terrain.clone()?;
        let category = usize::try_from(index)
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|at| terrains.categories.get(at))
            .ok_or_else(|| {
                format!("terrain {index} is outside the content's terrain categories")
            })?;
        Ok(Terrain {
            index,
            attack: category.common_adjuster.attack,
        })
    }

    fn crossing(&self, from: u32, to: u32) -> Fact<Option<u8>> {
        self.context
            .content
            .movement
            .as_deref()
            .ok_or("selected attack river content is unavailable")?
            .map
            .neighbour_kind(from, to)
    }

    fn region(&self, province: u32) -> Fact<Option<u32>> {
        Ok(self.context.reads.selected_region(province).ok())
    }

    fn land_fort(&self, building_type: u32) -> Fact<i32> {
        let names = TokenNames::new(self.context.frame).map_err(|e| e.to_string())?;
        let name = names.name(building_type).map_err(|e| e.to_string())?;
        self.context
            .reads
            .trigger_content
            .buildings
            .by_token(name)
            .map_err(|e| e.to_string())?
            .map(|building| building.land_fort)
            .ok_or_else(|| format!("building {name} is not an authored building type"))
    }

    fn night_factor(&self, province: u32) -> Fact<i64> {
        let frame = self.context.frame;
        let weather = frame
            .complete::<tables::WeatherProvinces>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|row| row.id == province)
            .ok_or("province weather absent")?;
        let clock = clock::resolve(
            weather,
            |province| {
                self.context
                    .reads
                    .updated_clock(province, frame.hour)
                    .map_err(str::to_owned)
            },
            || {
                frame
                    .complete::<tables::SimulationFeatures>()
                    .map_err(|e| e.to_string())?
                    .first()
                    .ok_or("simulation features absent")?
                    .weather_suspend
                    .require("SimulationFeatures.weather_suspend")
                    .copied()
            },
        )?;
        Ok(clock.night)
    }

    fn at_war(&self, receiver: u32, target: u32) -> Fact<bool> {
        Owner::new(self.context.frame, receiver)?.pointer_active(target)
    }
}

fn fallback_key(bytes: &[u8]) -> Fact<&str> {
    std::str::from_utf8(content_reader_parser::parser::c_string(bytes))
        .map_err(|_| "template fallback localisation key is not UTF-8".to_owned())
}

fn localise_template_fallback(
    localisation: &content_reader_localisation::localisation::Localisation,
    tag: &[u8],
    bytes: &[u8],
) -> Fact<String> {
    let length = tag
        .len()
        .checked_add(1)
        .and_then(|len| len.checked_add(bytes.len()))
        .ok_or("template qualified localisation key length overflow")?;
    let mut qualified = Vec::new();
    engine_allocation::reserve_exact(&mut qualified, length).map_err(|e| e.to_string())?;
    qualified.extend_from_slice(tag);
    qualified.push(b'_');
    qualified.extend_from_slice(bytes);
    let candidate = fallback_key(&qualified)?;
    let key = if localisation.get(candidate).is_some() {
        candidate
    } else {
        fallback_key(bytes)?
    };
    tooltip_text_localise(localisation, key, &[])
}

fn country_tag(frame: &DecodedFrame, raw: i32) -> Fact<&[u8]> {
    if raw <= 0 {
        return Err("template country is not positive; its tag is not recorded".into());
    }
    let slot =
        keyed::<tables::CountryLinks>(frame, raw as u64, |row| u64::from(row.id), "CountryLinks")?
            .canonical_slot;
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())?;
    let mut found = countries.iter().filter(|row| row.canonical_slot == slot);
    let country = found
        .next()
        .ok_or_else(|| format!("Countries lacks canonical slot {slot}"))?;
    if found.next().is_some() {
        return Err(format!("Countries duplicates canonical slot {slot}"));
    }
    Ok(&country.tag)
}

fn heading(context: &Context<'_>, template: u64) -> Fact<String> {
    let name = template_display_name(&context.content.localisation, context.frame, template)?;
    localise(
        &context.content.localisation,
        "LAND_COMBAT_ATTACK_PROVINCE_MODIFIERS",
        &[("DIVISION", Arg::Text(&name))],
    )
}

pub fn template_display_name(
    localisation: &content_reader_localisation::localisation::Localisation,
    frame: &DecodedFrame,
    template: u64,
) -> Fact<String> {
    let icon = keyed::<tables::DivisionTemplateIcons>(
        frame,
        template,
        |r| r.template,
        "DivisionTemplateIcons",
    )?;
    let display = icon
        .display_name
        .require("DivisionTemplateIcons.display_name")?
        .as_ref()
        .ok_or("DivisionTemplateIcons.display_name was refused when recorded")?;
    let daily = if matches!(
        &display.value,
        rows::TemplateNameValue::SameAsDivisionTemplates
    ) {
        Some(keyed::<tables::DivisionTemplates>(
            frame,
            template,
            |r| r.template,
            "DivisionTemplates",
        )?)
    } else {
        None
    };
    let parts = icon
        .display_name_inputs(frame.hour, daily)
        .map_err(str::to_owned)?;
    dashboard_text::unit_names::template_name(parts, icon.country, |raw, key| {
        localise_template_fallback(localisation, country_tag(frame, raw)?, key)
    })
}

fn modifier_text(name: &str, value: i64) -> String {
    let percent = content_reader_parser::fixed5::fixed5_percent(value, 2, true);
    if value > 0 {
        format!("{name}: \u{11}G+{percent}\u{11}!")
    } else {
        format!("{name}: \u{11}R{percent}\u{11}!")
    }
}

fn row_text(context: &Context<'_>, row: Row) -> Fact<String> {
    let key = modifier_name_key(row.kind)?;
    let name = localise(&context.content.localisation, key, &[])?;
    Ok(modifier_text(&name, row.value))
}

struct Slot<'s> {
    location: &'s dyn Fn(u64) -> Fact<u32>,
    template: &'s dyn Fn(u64) -> Fact<u64>,
    blocked: &'s mut dyn FnMut(u64, u32) -> Fact<bool>,
    notice: &'s dyn Fn() -> Fact<String>,
    attack: &'s dyn Fn(u64, u32, u32) -> Fact<Option<Preview>>,
    heading: &'s dyn Fn(u64) -> Fact<String>,
    row: &'s dyn Fn(Row) -> Fact<String>,
}

fn attack_slot(out: &mut Tooltip, members: &[u64], route: &Fact<Vec<u32>>, slot: Slot<'_>) {
    let locations: Vec<_> = members
        .iter()
        .map(|&handle| (slot.location)(handle))
        .collect();
    let current = match same_current(&locations) {
        Ok(Some(current)) => current,
        Ok(None) => return,
        Err(reason) => {
            refuse(out, "attack same-source gate", reason);
            return;
        }
    };
    let route = match route {
        Ok(route) => route,
        Err(reason) => {
            refuse(out, "attack retained route", reason.clone());
            return;
        }
    };
    let mut seen = BTreeSet::new();
    let mut earlier_unknown = false;
    for &handle in members {
        let template =
            match first_template((slot.template)(handle), &mut seen, &mut earlier_unknown) {
                Ok(Some(template)) => template,
                Ok(None) => continue,
                Err(reason) => {
                    refuse(out, "attack representative", reason);
                    continue;
                }
            };
        let Some((source, target)) = retained_edge(current, route) else {
            match (slot.blocked)(handle, current) {
                Ok(true) => match (slot.notice)() {
                    Ok(text) if !text.is_empty() => line(out, "blocked strait", 1, Ok(text)),
                    Ok(_) => {}
                    Err(reason) => refuse(out, "blocked strait", reason),
                },
                Ok(false) => {}
                Err(reason) => refuse(out, "blocked strait", reason),
            }
            continue;
        };
        let preview = match (slot.attack)(handle, source, target) {
            Ok(Some(preview)) => preview,
            Ok(None) => continue,
            Err(reason) => {
                refuse(out, "attack outer gate", reason);
                continue;
            }
        };
        if !preview.rows.is_empty() {
            line(out, "attack heading", 1, (slot.heading)(template));
        } else if !preview.refusals.is_empty() {
            refuse(
                out,
                "attack heading gate",
                "the native modifier-list count is unavailable".into(),
            );
        }
        for row in preview.rows {
            line(out, "attack modifier", 2, (slot.row)(row));
        }
        for (name, reason) in preview.refusals {
            refuse(out, &format!("attack {name}"), reason);
        }
    }
}

pub(super) fn append(
    out: &mut Tooltip,
    context: &Context<'_>,
    members: &[u64],
    route: &Fact<Vec<u32>>,
    queries: &CounterQueries<'_>,
) {
    let content = Content { context };
    attack_slot(
        out,
        members,
        route,
        Slot {
            location: &|handle| division(context.frame, handle).map(|unit| unit.province),
            template: &|handle| template(context.frame, handle),
            blocked: &mut |handle, current| {
                blocked(context, queries, handle, current, context.province.id)
            },
            notice: &|| {
                localise(
                    &context.content.localisation,
                    "UNIT_MOVEMENT_BLOCKED_BY_STRAIT",
                    &[],
                )
            },
            attack: &|handle, source, target| {
                attack_derive::preview(
                    &FrameInputs::new(context.frame, &content, handle),
                    source,
                    target,
                )
            },
            heading: &|template| heading(context, template),
            row: &|row| row_text(context, row),
        },
    );
}
