//! Ability descriptions, shared by active-map ability hovers
//! and the ability controller. Its caller supplies the fresh country/commander
//! carrier; the communications tail belongs to that caller.

use super::*;
use installed_content_abilities::abilities::Definition;

#[cfg(test)]
mod tests;

fn newline(text: &mut String) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
}

/// Raw modifier/adjuster text followed by independent native effect-description
/// sessions. Known fragments survive failures of other accumulators or effects.
/// The nonempty result includes a leading LF, before the communications
/// helper that the map-counter caller appends.
pub fn ability_description(
    definition: &Definition,
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
    let mut text = String::new();
    // Three direct calls: precision2, no outer colour, include -1,
    // exclude0, scale100000, no prefix, force_plus0, invert0. No scope expansion.
    for (label, modifiers) in ["country", "leader", "unit"]
        .into_iter()
        .zip(&definition.modifiers)
    {
        if modifiers != &installed_content_core::modifiers::Modifiers::default() {
            match renderer.modifier_description_with(
                modifiers,
                "",
                ModifierListOptions {
                    category_mask: u32::MAX,
                    ..Default::default()
                },
            ) {
                Ok(fragment) => text.push_str(&fragment),
                Err(why) => renderer.refuse(&format!("ability {label} modifiers"), why),
            }
        }
        newline(&mut text);
    }
    // Roster order: highlighted token header,
    // then movement/attack/defence lines with zero indentation.
    for adjuster in &definition.adjusters {
        let fragment = (|| {
            let selected = *selected
                .as_ref()
                .map_err(Clone::clone)
                .map_err(|why| why.to_string())?;
            let lines = renderer.char_adjuster_lines(adjuster, selected)?;
            if lines.is_empty() {
                return Ok(String::new());
            }
            let name = renderer.localised(adjuster.key.as_bytes(), &[], selected, false)?;
            Ok::<_, String>(format!("\u{11}H{name}\u{11}!\n{lines}"))
        })();
        match fragment {
            Ok(fragment) => text.push_str(&fragment),
            Err(why) => renderer.refuse(&format!("ability adjuster {}", adjuster.key), why),
        }
        newline(&mut text);
    }
    for (header, effects) in [
        None,
        Some(b"ABILITY_ON_HOURLY".as_slice()),
        Some(b"ABILITY_ON_DAILY".as_slice()),
    ]
    .into_iter()
    .zip(&definition.effects)
    {
        if effects.is_empty() {
            continue;
        }
        if let Some(header) = header {
            let title = selected
                .as_ref()
                .map_err(Clone::clone)
                .map_err(|why| why.to_string())
                .and_then(|selected| renderer.localised(header, &[], *selected, false));
            match title {
                Ok(title) => {
                    text.push_str(&title);
                    text.push('\n'); // Native appends LF even if the key ends in LF.
                }
                Err(why) => renderer.refuse("ability effect heading", why),
            }
        }
        // Each description owns its variable session. In
        // particular one-time writes do not seed the hourly/daily invocation.
        let description = describe(effects, context, scopes);
        text.push_str(&description.text);
        for (why, count) in description.refusals {
            *renderer.out.refusals.entry(why).or_default() += count;
        }
    }
    if !text.is_empty() {
        text.insert(0, '\n');
    }
    renderer.out.text = text;
    renderer.out
}
