
use super::*;
use crate::{scripted_text::EventScope, triggers::Eval};
use content_reader_military::doctrines::DefinitionSources;
use content_reader_parser::tokens::Token;

pub fn doctrine_tactic_sources(
    context: &Context<'_, '_>,
    country: u32,
    tactic_id: i32,
    resolve_tactic: impl Fn(&[u8]) -> Result<i32, String>,
    refused: &mut BTreeMap<String, usize>,
) -> Result<Vec<String>, String> {
    let database = context.content.doctrines.ok_or_else(|| {
        context
            .content
            .gaps
            .and_then(|gaps| gaps.get("doctrines"))
            .map(|why| format!("Doctrine tactic sources unavailable: {why}"))
            .unwrap_or_else(|| "Doctrine tactic sources unavailable".into())
    })?;
    let sources = &database.tactic_sources;
    let mut renderer = Renderer {
        context,
        graph: Rc::new(RefCell::new(EventScopes::new(Vec::new()))),
        evaluator: OnceCell::new(),
        out: Description::default(),
        scripted_stack: Vec::new(),
    };
    let mut output = Vec::new();
    let mut gaps = BTreeMap::<String, usize>::new();
    let matches = |tactics: &[Vec<u8>]| -> Result<bool, String> {
        for tactic in tactics {
            if resolve_tactic(tactic)? == tactic_id {
                return Ok(true);
            }
        }
        Ok(false)
    };
    let mut record = |label: &str, value: Result<String, String>| match value {
        Ok(text) => output.push(text),
        Err(why) => {
            *gaps
                .entry(format!("Doctrine tactic source {label}: {why}"))
                .or_default() += 1
        }
    };
    for (grand, order, definitions, names, unread) in [
        (
            true,
            &sources.grand_order,
            &sources.grand,
            &database.grand,
            &sources.grand_unread,
        ),
        (
            false,
            &sources.sub_order,
            &sources.sub,
            &database.sub,
            &sources.sub_unread,
        ),
    ] {
        if let Some(why) = unread {
            record(
                if grand {
                    "grand inventory"
                } else {
                    "sub inventory"
                },
                Err(why.clone()),
            );
            continue;
        }
        for token in order {
            let label = match token {
                Token::Name(name) => String::from_utf8_lossy(name).into_owned(),
                Token::Rejected => "token 0".into(),
            };
            let definition = &definitions[token];
            let track_gap = sources
                .tracks_unread
                .as_ref()
                .filter(|_| definition.tracks_requested || !definition.tracks.is_empty());
            match available(context, country, definition) {
                Ok(false) => continue,
                Err(why) => {
                    record(&label, Err(why));
                    continue;
                }
                Ok(true) => {}
            }
            let name = || -> Result<String, String> {
                let name = names
                    .get(token)
                    .ok_or("doctrine display name object is absent")?;
                if let Some(why) = &name.refusal {
                    return Err(why.clone());
                }
                context
                    .localisation
                    .localise_with(&name.value, &[])
                    .map_err(|why| why.to_string())
            };
            match matches(&definition.tactics) {
                Ok(true) if grand => record(
                    &label,
                    name().and_then(|name| {
                        renderer.localised(
                            b"DOCTRINE_EFFECT_SOURCE_GRAND_DOCTRINE",
                            &[("GRAND_DOCTRINE", Argument::Text(&name))],
                            Scopes::empty(),
                            false,
                        )
                    }),
                ),
                Ok(true) => {
                    if let Some(why) = track_gap {
                        record(&label, Err(why.clone()));
                    } else {
                        for track in &definition.tracks {
                            let result = name().and_then(|name| {
                                tracked_source(
                                    &mut renderer,
                                    b"DOCTRINE_EFFECT_SOURCE_SUBDOCTRINE",
                                    "SUBDOCTRINE",
                                    &name,
                                    &sources.tracks[track],
                                    None,
                                )
                            });
                            record(&label, result);
                        }
                    }
                }
                Err(why) => record(&label, Err(why)),
                Ok(false) => {}
            }
            if grand {
                if let Some(why) = track_gap {
                    record(&label, Err(why.clone()));
                    continue;
                }
                for (index, track) in definition.tracks.iter().enumerate() {
                    let Some(milestone) = definition.milestones.get(index) else {
                        record(
                            &label,
                            Err(format!("milestone {index} is absent for a resolved track")),
                        );
                        continue;
                    };
                    match matches(milestone) {
                        Ok(true) => {
                            let result = name().and_then(|name| {
                                tracked_source(
                                    &mut renderer,
                                    b"DOCTRINE_EFFECT_SOURCE_MILESTONE",
                                    "GRAND_DOCTRINE",
                                    &name,
                                    &sources.tracks[track],
                                    None,
                                )
                            });
                            record(&label, result);
                        }
                        Err(why) => record(&label, Err(why)),
                        Ok(false) => {}
                    }
                }
            } else {
                let mut mastery = Ok(0i64);
                for reward in &definition.rewards {
                    let amount = if reward.mastery == 0 {
                        sources.default_reward_mastery.ok_or_else(|| {
                            "NDoctrines.DEFAULT_REWARD_MASTERY unavailable".to_owned()
                        })
                    } else {
                        Ok(reward.mastery)
                    };
                    mastery = mastery.and_then(|sum| amount.map(|amount| sum.wrapping_add(amount)));
                    match matches(&reward.tactics) {
                        Ok(true) => {
                            if let Some(why) = track_gap {
                                record(&label, Err(why.clone()));
                                continue;
                            }
                            for track in &definition.tracks {
                                let result = mastery.clone().and_then(|mastery| {
                                    name().and_then(|name| {
                                        tracked_source(
                                            &mut renderer,
                                            b"DOCTRINE_EFFECT_SOURCE_REWARD",
                                            "SUBDOCTRINE",
                                            &name,
                                            &sources.tracks[track],
                                            Some(mastery),
                                        )
                                    })
                                });
                                record(&label, result);
                            }
                        }
                        Err(why) => record(&label, Err(why)),
                        Ok(false) => {}
                    }
                }
            }
        }
    }
    let empty_failure = (output.is_empty() && !gaps.is_empty())
        .then(|| gaps.keys().cloned().collect::<Vec<_>>().join("; "));
    for (why, count) in gaps {
        *refused.entry(why).or_default() += count;
    }
    empty_failure.map_or(Ok(output), Err)
}

fn tracked_source(
    renderer: &mut Renderer<'_, '_, '_>,
    key: &[u8],
    doctrine_argument: &str,
    name: &str,
    track: &content_reader_script::localization_binding::BoundLocalization,
    mastery: Option<i64>,
) -> Result<String, String> {
    let track = renderer.bound_localisation(track, Scopes::empty(), false)?;
    let mut args = vec![
        (doctrine_argument, Argument::Text(name)),
        ("TRACK", Argument::Text(&track)),
    ];
    if let Some(mastery) = mastery {
        args.push(("MASTERY", Argument::Fixed5(mastery)));
    }
    renderer.localised(key, &args, Scopes::empty(), false)
}

fn available(
    context: &Context<'_, '_>,
    country: u32,
    definition: &DefinitionSources,
) -> Result<bool, String> {
    if definition.available.is_empty() {
        return Ok(true);
    }
    let graph = Rc::new(RefCell::new(EventScopes::new(vec![EventScope::of(
        EventObject::Country(country),
    )])));
    let evaluator = Evaluator::new(context.world()?, None).with_event_scopes(graph);
    match evaluator.evaluate(
        &definition.available,
        evaluator.event_scopes(0).map_err(|why| why.to_string())?,
    ) {
        Eval::True => Ok(true),
        Eval::False => Ok(false),
        Eval::Unknown(reasons) => Err(reasons.into_iter().collect::<Vec<_>>().join("; ")),
    }
}

#[cfg(test)]
mod tests;

pub fn doctrine_benefits(
    benefits: &content_reader_military::doctrines::Benefits,
    context: &Context<'_, '_>,
    scopes: &mut EventScopes,
) -> Description {
    let mut renderer = Renderer {
        context,
        graph: Rc::new(RefCell::new(scopes.clone())),
        evaluator: OnceCell::new(),
        out: Description::default(),
        scripted_stack: Vec::new(),
    };
    let selected = crate::triggers::event_scopes::project(scopes, 0);
    let mut output = String::new();
    match selected {
        Ok(selected) => {
            if let Some(prose) = &benefits.prose {
                match renderer.bound_localisation(prose, selected, false) {
                    Ok(text) => {
                        output.push_str(&text);
                        output.push('\n');
                    }
                    Err(why) => renderer.refuse("doctrine description", why),
                }
            }
            match renderer.units_bonus_description(&benefits.units, selected) {
                Ok(text) => output.push_str(&text),
                Err(why) => renderer.refuse("doctrine unit bonuses", why),
            }
            for tactic in &benefits.tactics {
                match renderer.localised(tactic, &[], selected, false) {
                    Ok(text) => {
                        output.push_str(&text);
                        output.push('\n');
                    }
                    Err(why) => renderer.refuse("doctrine tactic", why),
                }
            }
        }
        Err(why) => renderer.refuse("doctrine bonus scope", why.to_string()),
    }
    if benefits.modifiers != content_reader_script::modifiers::Modifiers::default() {
        match renderer.modifier_description_with(
            &benefits.modifiers,
            "",
            ModifierListOptions {
                category_mask: u32::MAX,
                ..Default::default()
            },
        ) {
            Ok(text) => {
                output.push_str(&text);
                output.push('\n');
            }
            Err(why) => renderer.refuse("doctrine modifiers", why),
        }
    }
    if !benefits.effects.is_empty() {
        let description = describe(&benefits.effects, context, scopes);
        output.push_str(&description.text);
        for (why, count) in description.refusals {
            *renderer.out.refusals.entry(why).or_default() += count;
        }
    }
    renderer.out.text = output;
    renderer.out
}
