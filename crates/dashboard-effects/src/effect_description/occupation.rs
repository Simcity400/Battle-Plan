use super::*;
use content_reader_politics::occupation_modifiers::Definition;

pub fn occupation_modifier_description(
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
    let localise = |key: &[u8]| {
        context
            .localisation
            .localise(key)
            .map_err(|why| why.to_string())
    };
    match localise(definition.token.as_bytes()) {
        Ok(name) => renderer.out.text = format!("\u{11}H{name}\u{11}!\n\n"),
        Err(why) => renderer.refuse("occupation modifier name", why),
    }
    if !definition.on_disable.is_empty() {
        let description = describe(&definition.on_disable, context, scopes);
        for (why, count) in description.refusals {
            *renderer.out.refusals.entry(why).or_default() += count;
        }
        if !description.text.is_empty() {
            section_gap(&mut renderer.out.text);
            match localise(b"OCCUPATION_MODIFIERS_ON_DISABLE") {
                Ok(header) => {
                    renderer.out.text.push_str(&header);
                    renderer.out.text.push('\n');
                }
                Err(why) => renderer.refuse("occupation disable heading", why),
            }
            renderer.out.text.push_str(&description.text);
        }
    }
    let modifiers = (|| {
        let mut modifiers = content_reader_script::modifiers::values(&definition.state_modifier);
        filter_registered(&mut modifiers, context.content.key_names)?;
        Ok::<_, String>(modifiers)
    })();
    match modifiers.and_then(|modifiers| {
        let kept = renderer.char_trait_modifier_kept(&modifiers)?;
        Ok((modifiers, kept))
    }) {
        Ok((modifiers, true)) => {
            section_gap(&mut renderer.out.text);
            match localise(b"OCCUPATION_MODIFIERS_TOOLTIP") {
                Ok(header) => {
                    renderer.out.text.push_str(&header);
                    renderer.out.text.push('\n');
                }
                Err(why) => renderer.refuse("occupation modifiers heading", why),
            }
            match renderer.modifier_description_with(
                &modifiers,
                "",
                ModifierListOptions {
                    category_mask: u32::MAX,
                    ..Default::default()
                },
            ) {
                Ok(text) => renderer.out.text.push_str(&text),
                Err(why) => renderer.refuse("occupation modifiers", why),
            }
        }
        Ok((_, false)) => {}
        Err(why) => renderer.refuse("occupation modifiers", why),
    }
    match localise(definition.tooltip.as_bytes()) {
        Ok(text) if !text.is_empty() => {
            section_gap(&mut renderer.out.text);
            renderer.out.text.push_str(&text);
        }
        Ok(_) => {}
        Err(why) => renderer.refuse("occupation modifier tooltip", why),
    }
    renderer.out
}

fn section_gap(text: &mut String) {
    let held = text.bytes().rev().take_while(|byte| *byte == b'\n').count();
    text.extend(std::iter::repeat_n('\n', 2_usize.saturating_sub(held)));
}

fn filter_registered(
    modifiers: &mut content_reader_script::modifiers::Modifiers,
    names: Option<&content_reader_script::key_names::KeyNames>,
) -> Result<(), String> {
    let mut failure = None;
    modifiers.values.retain(|value| {
        match content_reader_script::modifiers::registered(value.name.as_bytes(), names) {
            Some(registered) => registered,
            None => {
                failure.get_or_insert_with(|| {
                    format!(
                        "occupation modifier {} registry is unavailable (1 modifier)",
                        value.key
                    )
                });
                false
            }
        }
    });
    if let Some(why) = failure {
        return Err(why);
    }
    for child in &mut modifiers.children {
        filter_registered(child, names)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect_description::tests::{Fixture, fresh};

    fn render(definition: &Definition) -> Description {
        let mut fixture = Fixture::new();
        fixture.catalogue.localisation = Arc::new(
            fixture
                .catalogue
                .localisation
                .iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .chain(
                    [
                        ("mod_badge", "Badge"),
                        ("OCCUPATION_MODIFIERS_ON_DISABLE", "Disable"),
                        ("OCCUPATION_MODIFIERS_TOOLTIP", "Modifiers"),
                        ("MOD_RAW", "Raw [Root.GetTag]"),
                        ("MOD_TAIL", "Tail [Root.GetTag]"),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into())),
                )
                .collect(),
        );
        fixture.with_context(Content::default(), |context| {
            occupation_modifier_description(definition, context, &mut fresh())
        })
    }

    #[test]
    fn occupation_badge_active_description_omits_detail_and_enable_effects() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let items = |body: &[u8]| content_reader_parser::parser::parse(body).unwrap().items;
        let definition = Definition {
            token: "mod_badge".into(),
            kind: content_reader_politics::occupation_modifiers::Kind::StateCompliance,
            threshold: 3_100_000,
            margin: 300_000,
            tooltip: "MOD_TAIL".into(),
            state_modifier: items(b"custom_modifier_tooltip=MOD_RAW"),
            on_disable: items(b"custom_effect_tooltip=TIP"),
            on_enable: items(b"custom_effect_tooltip=REFERENCE"),
            enabled: items(b"always=no"),
            visible: items(b"always=no"),
            ..Default::default()
        };
        let result = render(&definition);
        assert_eq!(
            result.text,
            "\u{11}HBadge\u{11}!\n\nDisable\ntip\n\nModifiers\nRaw [Root.GetTag]\n\nTail [Root.GetTag]"
        );
        assert!(result.refusals.is_empty(), "{:?}", result.refusals);
    }

    #[test]
    fn occupation_badge_failed_effect_keeps_independent_modifier_and_tail() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let definition = Definition {
            token: "mod_badge".into(),
            tooltip: "MOD_TAIL".into(),
            state_modifier: content_reader_parser::parser::parse(b"custom_modifier_tooltip=MOD_RAW")
                .unwrap()
                .items,
            on_disable: content_reader_parser::parser::parse(b"unimplemented_effect=yes")
                .unwrap()
                .items,
            ..Default::default()
        };
        let result = render(&definition);
        assert!(result.text.contains("Raw [Root.GetTag]"));
        assert!(result.text.ends_with("Tail [Root.GetTag]"));
        assert_eq!(result.refusals.values().sum::<usize>(), 1);
        assert!(
            result
                .refusals
                .keys()
                .any(|why| why.contains("unimplemented_effect"))
        );
    }
}
