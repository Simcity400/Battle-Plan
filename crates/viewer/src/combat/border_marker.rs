//! Border-conflict outcome tail of 1413D69D0, after the common marker text.
use super::{
    INLINE_FLAG,
    binding::{TextIcons, builtin, token},
};
use crate::{document::Document, map::raids::tooltips::Tooltip, table_access::rows};
use dashboard::scripted_text::{EventObject, EventScope, EventScopes, Link, ScriptedText};
use installed_content::{
    events::Event,
    localisation::{Argument, Localisation},
    parser::Item,
};
use replay::{
    frame::DecodedFrame,
    rows::{LandBorderWarInputs, tables},
};
use std::collections::BTreeMap;

// Executable keys/substitution names at 1413D69D0.
super::text::tooltip_keys! {
    BORDER_CONFLICT_IF_YOU_WIN = "BORDER_CONFLICT_IF_YOU_WIN",
    BORDER_CONFLICT_IF_YOU_LOSE = "BORDER_CONFLICT_IF_YOU_LOSE",
    BORDER_CONFLICT_CHANGE_STATE = "BORDER_CONFLICT_CHANGE_STATE",
    BORDER_CONFLICT_FOR_ENEMY = "BORDER_CONFLICT_FOR_ENEMY",
    EFFECT_COUNTRY_EVENT = "EFFECT_COUNTRY_EVENT",
}

fn canonical(raw: i32, links: &BTreeMap<u32, u32>) -> Result<u32, String> {
    links
        .get(&(raw as u32))
        .copied()
        .ok_or_else(|| format!("CountryLinks lacks border country {raw}"))
}
fn member(raw: i32, viewer: u32, links: &BTreeMap<u32, u32>) -> Result<bool, String> {
    if raw as u32 == viewer {
        return Ok(true);
    }
    if raw == 0 || viewer == 0 {
        return Ok(false);
    }
    Ok(canonical(raw, links)? == viewer)
}
fn newline(body: &mut String) {
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
}

fn country_marker(name: &str, flag: &str, transfer: bool) -> String {
    // 1413D69D0 brackets the transfer target; the enemy heading uses a space.
    if transfer {
        format!("{name} ({INLINE_FLAG}{flag})")
    } else {
        format!("{name} {INLINE_FLAG}{flag}")
    }
}

/// ROOT is a fresh country scope; FROM is this side's state, whose FROM is
/// a deep copy of the opposing country/state context. PREV stays itself.
/// 1413D69D0 builds copies before linking the original state scopes.
fn scopes(own: u32, own_state: u32, enemy: u32, enemy_state: u32) -> EventScopes {
    let mut values = vec![
        EventScope::of(EventObject::Country(own)),
        EventScope::of(EventObject::Country(own)),
        EventScope::of(EventObject::State(own_state)),
        EventScope::of(EventObject::Country(enemy)),
        EventScope::of(EventObject::Country(enemy)),
        EventScope::of(EventObject::State(enemy_state)),
    ];
    values[0].root = Link::Scope(1);
    values[0].from = Link::Scope(2);
    values[2].from = Link::Scope(3);
    values[3].root = Link::Scope(4);
    values[3].from = Link::Scope(5);
    EventScopes::new(values)
}

#[derive(Default)]
struct Description {
    body: String,
    refused: BTreeMap<String, usize>,
}
impl Description {
    fn refuse(&mut self, why: String) {
        *self.refused.entry(why).or_default() += 1;
    }
}

fn effects(
    items: &[Item],
    render: &impl Fn(&[u8]) -> Result<String, String>,
    event: &Event,
    out: &mut Description,
) -> String {
    let mut body = String::new();
    for item in items {
        let key = item.key.as_deref().unwrap_or_default();
        let line = if item.keyword_is("custom_effect_tooltip") {
            match item.value.scalar() {
                Some(value) if !value.contains(&b'|') => {
                    match render(installed_content::parser::c_string(value)) {
                        Ok(line) => line,
                        Err(why) => {
                            out.refuse(format!(
                                "{} event {} custom_effect_tooltip: {why}",
                                event.source,
                                String::from_utf8_lossy(&event.id)
                            ));
                            String::new()
                        }
                    }
                }
                _ => {
                    out.refuse(format!("{} event {} custom_effect_tooltip bound formatter is not ported (1413832B0)", event.source, String::from_utf8_lossy(&event.id)));
                    String::new()
                }
            }
        } else if item.keyword_is("effect_tooltip") {
            effects(item.value.items().unwrap_or_default(), render, event, out)
        } else if item.keyword_is("hidden_effect") {
            // 1413818F0 builds, then discards its description; count failures too.
            effects(item.value.items().unwrap_or_default(), render, event, out);
            String::new()
        } else {
            out.refuse(format!("{} event {} effect {} description is not ported (CEffect::BuildDescription 140532780); its scope/conditional/value evaluator is unavailable",
                event.source, String::from_utf8_lossy(&event.id), String::from_utf8_lossy(key)));
            String::new()
        };
        // Native suppresses descriptions of signed byte length <=1.
        if line.len() as i32 > 1 {
            newline(&mut body);
            body.push_str(&line);
        }
    }
    body
}

fn event_description(
    event: &Event,
    render: &impl Fn(&[u8]) -> Result<String, String>,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Description {
    let mut out = Description::default();
    if let Some(why) = &event.refused {
        out.refuse(why.clone());
        return out;
    }
    if event.options.len() < 2 {
        out.body = effects(&event.immediate, render, event, &mut out);
        newline(&mut out.body);
        if let Some(option) = event.options.first() {
            let body = effects(option, render, event, &mut out);
            out.body.push_str(&body);
            newline(&mut out.body);
        }
    } else {
        // 14116B3B0/14116C180 choose a scoped title, not any effect body.
        let title = match event.titles.as_slice() {
            [] => Ok(String::new()),
            [item] => {
                // 14116C180 returns the sole candidate before trigger evaluation.
                let key = item
                    .value
                    .scalar()
                    .or_else(|| {
                        item.value
                            .items()?
                            .iter()
                            .rev()
                            .find(|item| item.keyword_is("text"))?
                            .value
                            .scalar()
                    })
                    .unwrap_or_default();
                if key.is_empty() {
                    Ok(String::new())
                } else {
                    render(installed_content::parser::c_string(key))
                }
            }
            _ => Err(format!(
                "{} event {} has multiple title candidates; 14116C180 selects with an unrecorded high-resolution seed",
                event.source,
                String::from_utf8_lossy(&event.id)
            )),
        };
        match title.and_then(|title| {
            builtin(
                icons,
                loc,
                EFFECT_COUNTRY_EVENT,
                &[("EVENT", Argument::Text(&title))],
            )
        }) {
            Ok(body) => out.body = body,
            Err(why) => out.refuse(why),
        }
    }
    out
}

#[allow(clippy::too_many_arguments)] // Native 1413D69D0 context and output.
fn build(
    icons: &dyn TextIcons,
    loc: &Localisation,
    border: &LandBorderWarInputs,
    mine: usize,
    body: &mut String,
    country: &impl Fn(i32, bool) -> Result<String, String>,
    state: &impl Fn(i32) -> Result<String, String>,
    event: &impl Fn(&[u8], usize) -> Description,
    refused: &mut BTreeMap<String, usize>,
) -> Result<(), String> {
    let own = &border.sides[mine];
    let enemy = &border.sides[1 - mine];
    for win in [true, false] {
        let own_id = if win { &own.win_event } else { &own.lose_event };
        let enemy_id = if win {
            &enemy.lose_event
        } else {
            &enemy.win_event
        };
        if own_id.is_empty() && enemy_id.is_empty() && border.transfer_state == 0 {
            continue;
        }
        newline(body);
        body.push('\n');
        body.push_str(&builtin(
            icons,
            loc,
            if win {
                BORDER_CONFLICT_IF_YOU_WIN
            } else {
                BORDER_CONFLICT_IF_YOU_LOSE
            },
            &[],
        )?);
        body.push('\n');
        if border.transfer_state != 0 {
            let (country_id, state_id) = if win {
                (own.country, enemy.state)
            } else {
                (enemy.country, own.state)
            };
            let country = country(country_id, true)?;
            let state = state(state_id)?;
            body.push_str(&builtin(
                icons,
                loc,
                BORDER_CONFLICT_CHANGE_STATE,
                &[
                    ("STATE", Argument::Text(&state)),
                    ("COUNTRY", Argument::Text(&country)),
                ],
            )?);
            body.push('\n');
        }
        for (id, side) in [(own_id, mine), (enemy_id, 1 - mine)] {
            if id.is_empty() {
                continue;
            }
            let text = event(id, side);
            for (why, count) in text.refused {
                *refused.entry(why).or_default() += count;
            }
            if text.body.is_empty() {
                continue;
            }
            if side != mine {
                let country = country(enemy.country, false)?;
                body.push_str(&builtin(
                    icons,
                    loc,
                    BORDER_CONFLICT_FOR_ENEMY,
                    &[("COUNTRY", Argument::Text(&country))],
                )?);
                body.push('\n');
            }
            body.push_str(&text.body);
        }
    }
    Ok(())
}

pub(super) fn append(
    document: &Document,
    frame: &DecodedFrame,
    viewer: u32,
    links: &BTreeMap<u32, u32>,
    border: &LandBorderWarInputs,
    tooltip: &mut Tooltip,
) -> Result<(), String> {
    let mine = if member(border.sides[0].country, viewer, links)? {
        0
    } else if member(border.sides[1].country, viewer, links)? {
        1
    } else {
        return Ok(());
    };
    let loc = &document.names.localisation;
    let icons = &document.icons.combat;
    let flags = super::binding::flags(document, frame).map_err(|error| error.to_string())?;
    let names = installed_content::country_names::CountryNames::read(frame, loc);
    let country = |raw: i32, transfer: bool| {
        let name = names.name(canonical(raw, links)?)?;
        let flag = super::binding::country_flag(&flags, raw as u32)?;
        Ok(country_marker(&name, &flag, transfer))
    };
    let states = rows::<tables::States>(frame).map_err(|error| error.to_string())?;
    let state = |id: i32| {
        let row = states
            .rows()
            .iter()
            .find(|row| row.id == id as u32)
            .ok_or_else(|| format!("States lacks border state {id}"))?;
        super::text::state_name(
            loc,
            &document.recorded_language,
            id as u32,
            &row.custom_name,
            &row.owner_name,
            || {
                let key = document
                    .state_name_keys
                    .get(&(id as u32))
                    .ok_or_else(|| format!("content lacks border state {id}"))?
                    .as_ref()
                    .map_err(Clone::clone)?;
                token(icons, loc, key.as_bytes())
            },
        )
    };
    let event = |id: &[u8], index: usize| {
        let result = (|| {
            let event = icons.border_events.get(id)?;
            let content = document.scripted.as_ref().map_err(Clone::clone)?;
            let own = &border.sides[index];
            let enemy = &border.sides[1 - index];
            let own_country = canonical(own.country, links)?;
            let scopes = scopes(
                own_country,
                own.state as u32,
                canonical(enemy.country, links)?,
                enemy.state as u32,
            );
            let scripted = ScriptedText::new(frame, loc, content, own_country);
            let render = |key: &[u8]| scripted.expand_in(&token(icons, loc, key)?, &scopes);
            Ok::<_, String>(event_description(&event, &render, icons, loc))
        })();
        result.unwrap_or_else(|why| {
            let mut out = Description::default();
            out.refuse(why);
            out
        })
    };
    build(
        icons,
        loc,
        border,
        mine,
        &mut tooltip.body,
        &country,
        &state,
        &event,
        &mut tooltip.refused,
    )
}

#[cfg(test)]
mod tests;
