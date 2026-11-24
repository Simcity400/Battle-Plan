use dashboard_effects::{ScriptedEffects, effect_description::{self, Context as EffectContext}};
use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, Link, ScriptedText};
use content_reader_parser::parser::Item;
use content_reader_events::events::Event;
use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::{rows::LandBorderWarInputs, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
use viewer_foundation::tooltip_text::state_name;
use viewer_foundation::tooltip_text::tooltip_keys;
use {
    super::INLINE_FLAG,
    viewer_foundation::tooltip_text::{TextIcons, builtin, token},
};
use super::content::Content as StaticContent;
use dashboard_text::table_access::rows;
use viewer_foundation::tooltip_text::Tooltip;

tooltip_keys! {
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

fn tail_side(
    border: &LandBorderWarInputs,
    viewer: u32,
    links: &BTreeMap<u32, u32>,
) -> Result<Option<usize>, String> {
    if border.transfer_state == 0
        && border
            .sides
            .iter()
            .all(|side| side.win_event.is_empty() && side.lose_event.is_empty())
    {
        return Ok(None);
    }
    if member(border.sides[0].country, viewer, links)? {
        Ok(Some(0))
    } else if member(border.sides[1].country, viewer, links)? {
        Ok(Some(1))
    } else {
        Ok(None)
    }
}
fn newline(body: &mut String) {
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
}

fn country_marker(name: &str, flag: &str, transfer: bool) -> String {
    if transfer {
        format!("{name} ({INLINE_FLAG}{flag})")
    } else {
        format!("{name} {INLINE_FLAG}{flag}")
    }
}

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

fn event_description(
    event: &Event,
    enemy_win: bool,
    describe: &impl Fn(&[Item], &mut EventScopes) -> effect_description::Description,
    scopes: &mut EventScopes,
    render: &impl Fn(&[u8], &EventScopes) -> Result<String, String>,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Description {
    let mut out = Description::default();
    if let Some(why) = &event.refused {
        out.refuse(why.clone());
        return out;
    }
    if enemy_win && event.options.len() >= 2 {
        return out;
    }
    if event.options.len() < 2 {
        for items in std::iter::once(&event.immediate).chain(event.options.first()) {
            let description = describe(items, scopes);
            for (why, count) in description.refusals {
                *out.refused.entry(why).or_default() += count;
            }
            out.body
                .push_str(effect_description::trim_description(&description.text));
            newline(&mut out.body);
        }
    }
    if event.options.len() >= 2 || (enemy_win && event.options.is_empty()) {
        let title = event.sole_title_key().and_then(|key| match key {
            Some(key) => render(key, scopes),
            None => Ok(String::new()),
        });
        match title.and_then(|title| {
            builtin(
                icons,
                loc,
                EFFECT_COUNTRY_EVENT,
                &[("EVENT", Argument::Text(&title))],
            )
        }) {
            Ok(body) => out.body.push_str(&body),
            Err(why) => out.refuse(why),
        }
    }
    out
}

#[expect(clippy::too_many_arguments, reason = "the text resources, the border, the name lookups and the outputs of one marker")]
fn build(
    icons: &dyn TextIcons,
    loc: &Localisation,
    border: &LandBorderWarInputs,
    mine: usize,
    body: &mut String,
    country: &impl Fn(i32, bool) -> Result<String, String>,
    state: &impl Fn(i32) -> Result<String, String>,
    event: &impl Fn(&[u8], usize, bool) -> Description,
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
            let text = event(id, side, !win && side != mine);
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
    static_content: &StaticContent,
    frame: &DecodedFrame,
    viewer: u32,
    links: &BTreeMap<u32, u32>,
    border: &LandBorderWarInputs,
    nato: bool,
    tooltip: &mut Tooltip,
) -> Result<(), String> {
    let Some(mine) = tail_side(border, viewer, links)? else {
        return Ok(());
    };
    let loc = &static_content.localisation;
    let icons = static_content.assets.as_ref();
    let flags = super::binding::flags(static_content, frame).map_err(|error| error.to_string())?;
    let names = content_reader_country_names::country_names::CountryNames::read(frame, loc);
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
        state_name(
            loc,
            &static_content.recorded_language,
            id as u32,
            &row.custom_name,
            &row.owner_name,
            || {
                let key = static_content
                    .state_name_keys
                    .get(&(id as u32))
                    .ok_or_else(|| format!("content lacks border state {id}"))?
                    .as_ref()
                    .map_err(Clone::clone)?;
                token(icons, loc, key.as_bytes())
            },
        )
    };
    let scripted = static_content
        .scripted
        .as_ref()
        .as_ref()
        .map(|content| ScriptedText::new(frame, loc, content, viewer).viewing(viewer).with_nato_counters(nato));
    let event = |id: &[u8], index: usize, enemy_win: bool| {
        let result = (|| {
            let event = icons.border_events.get(id)?;
            let content = static_content
                .scripted
                .as_ref()
                .as_ref()
                .map_err(Clone::clone)?;
            let own = &border.sides[index];
            let enemy = &border.sides[1 - index];
            let own_country = canonical(own.country, links)?;
            let mut scopes = scopes(
                own_country,
                own.state as u32,
                canonical(enemy.country, links)?,
                enemy.state as u32,
            );
            let scripted = scripted.as_ref().map_err(|why| (*why).clone())?;
            let event_loader = |id: &[u8]| icons.border_events.get(id);
            let describe = |items: &[Item], scopes: &mut EventScopes| {
                let result = (|| {
                    let context = EffectContext::new(
                        scripted,
                        loc,
                        viewer,
                        &event.source,
                        content.effects()?.view(Some(&event_loader)),
                    );
                    Ok::<_, String>(effect_description::describe(items, &context, scopes))
                })();
                result.unwrap_or_else(|why| effect_description::Description {
                    refusals: [(why, 1)].into_iter().collect(),
                    ..Default::default()
                })
            };
            let render = |key: &[u8], scopes: &EventScopes| {
                scripted.expand_in(&token(icons, loc, key)?, scopes)
            };
            Ok::<_, String>(event_description(
                &event,
                enemy_win,
                &describe,
                &mut scopes,
                &render,
                icons,
                loc,
            ))
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
