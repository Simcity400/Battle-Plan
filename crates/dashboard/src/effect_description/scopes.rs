use super::*;
use crate::triggers::{Eval, Scope};
use installed_content::scope::{self, Scope as Parsed};

impl Renderer<'_, '_, '_> {
    pub(super) fn is_context(&self, item: &Item) -> bool {
        let name = key(item);
        !installed_content::effect_keys::REGISTERED_EFFECTS
            .iter()
            .any(|(_, native)| native.eq_ignore_ascii_case(&name))
            && item.value.items().is_some()
            && !matches!(
                scope::parse(&name, item.key_quoted, self.context.text.recorded()),
                Parsed::Unknown(_)
            )
    }

    pub(super) fn context_header(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<Option<(Scopes, String)>, String> {
        let parsed = scope::parse(&key(item), item.key_quoted, self.context.text.recorded());
        let Some(selected) = self.evaluator()?.enter_scope(&parsed, scopes)? else {
            return Ok(None);
        };
        let header = match selected.this {
            Scope::Country(country) => {
                if country == self.context.perspective
                    && self.origin_country(scopes)? == Some(country)
                {
                    String::new()
                } else {
                    let tag = self.tag(country)?;
                    let name = self.country_name(country, false)?;
                    self.localised(
                        b"EFFECT_CONTEXT_COUNTRY",
                        &[
                            ("TAG", Argument::Text(tag)),
                            ("COUNTRY", Argument::Text(&name)),
                        ],
                        selected,
                        false,
                    )?
                }
            }
            Scope::State(state) => {
                let name = self.state_name(state)?;
                self.localised(
                    b"EFFECT_CONTEXT_STATE",
                    &[("COUNTRY", Argument::Text(&name))],
                    selected,
                    false,
                )?
            }
            _ => {
                return Err(
                    "context header kind outside the reached country/state subset (141382590)"
                        .into(),
                );
            }
        };
        Ok(Some((selected, header)))
    }

    // 14052B240 reads ROOT's country or state owner, falling back to THIS only
    // when ROOT carries neither; 141382590 uses it for perspective suppression.
    fn origin_country(&self, scopes: Scopes) -> Result<Option<u32>, String> {
        match scopes.root {
            Scope::Country(country) => Ok((country > 0).then_some(country)),
            Scope::State(state) if state != 0 && state != crate::triggers::NULL_STATE => {
                let state = self
                    .context
                    .world()?
                    .states
                    .get(&state)
                    .ok_or("ROOT state owner is not recorded (14052B240)")?;
                self.context
                    .world()?
                    .recorded()
                    .slot(state.owner)
                    .map(Some)
                    .ok_or_else(|| format!("CountryLinks lacks ROOT state owner {}", state.owner))
            }
            _ => Ok(match scopes.this {
                Scope::Country(country) if country > 0 => Some(country),
                _ => None,
            }),
        }
    }

    pub(super) fn context_effect(&mut self, item: &Item, scopes: Scopes, indent: usize) -> String {
        let (selected, mut header) = match self.context_header(item, scopes) {
            Ok(Some(value)) => value,
            Ok(None) => return String::new(),
            Err(why) => {
                self.refuse(&key(item), why);
                return String::new();
            }
        };
        let body = self.attached(children(item), true);
        let child_indent = if header.is_empty() {
            0
        } else {
            let before = self.out.refusals.clone();
            let multiple = self.multiple(children(item), selected);
            let failed = self.out.refusals != before;
            self.out.refusals = before; // Native layout probes are not description occurrences.
            if failed {
                self.refuse(&key(item), "context header layout is unknown because a native description probe failed (140533D80)".into());
                return String::new();
            }
            if multiple {
                header.push_str(":\n");
                indent + 1
            } else {
                header.push(' ');
                0
            }
        };
        if body.first().is_some_and(|(child, _)| !self.indents(child)) {
            header.clear();
        }
        let mut text = String::new();
        for (child, branches) in body {
            let line = if child.keyword_is("if") {
                self.if_chain(child, &branches, selected, child_indent, false)
                    .unwrap_or_default()
            } else {
                self.builder(child, selected, child_indent)
            };
            text.push_str(&line); // 141381200 concatenates children directly.
        }
        if text.is_empty() {
            text
        } else {
            header + &text
        }
    }

    pub(super) fn every_state(&mut self, item: &Item, scopes: Scopes, indent: usize) -> String {
        let items = children(item);
        for name in [
            "display_individual_scopes",
            "visible_when_empty",
            "random_select_amount",
            "tooltip",
        ] {
            if field(items, name).is_some() {
                self.refuse(
                    "every_state",
                    format!("{name} is outside the reached default form (140312880)"),
                );
                return String::new();
            }
        }
        let world = match self.context.world() {
            Ok(world) => world,
            Err(why) => {
                self.refuse("every_state", why);
                return String::new();
            }
        };
        if let Err(why) = world.frame().complete::<replay::rows::tables::States>() {
            self.refuse("every_state", why.to_string());
            return String::new();
        }
        let mut selected = Vec::new();
        for &id in world.states.keys() {
            let entered = match self
                .evaluator()
                .and_then(|evaluator| evaluator.enter_scope(&Parsed::State(id as i32), scopes))
            {
                Ok(Some(value)) => value,
                Ok(None) => continue,
                Err(why) => {
                    self.refuse("every_state", why);
                    return String::new();
                }
            };
            match self.condition(items, entered) {
                Eval::True => selected.push((id, entered)),
                Eval::False => {}
                Eval::Unknown(why) => {
                    self.refuse(
                        "every_state",
                        why.into_iter().collect::<Vec<_>>().join("; "),
                    );
                    return String::new();
                }
            }
        }
        let Some((_, first)) = selected.first().copied() else {
            return String::new();
        };
        let mut labels = Vec::new();
        for (id, _) in selected.iter().take(if selected.len() < 5 { 4 } else { 3 }) {
            match self.state_name(*id) {
                Ok(name) => labels.push(format!("\u{11}Y{name}\u{11}!")),
                Err(why) => {
                    self.refuse("every_state", why);
                    return String::new();
                }
            }
        }
        let mut list = labels.join(", ");
        if selected.len() >= 5 {
            list = match self.localised(
                b"EFFECT_LIST_OTHERS",
                &[
                    ("LIST", Argument::Text(&list)),
                    ("NUMBER", Argument::Int32((selected.len() - 3) as i32)),
                ],
                scopes,
                false,
            ) {
                Ok(text) => text,
                Err(why) => {
                    self.refuse("every_state", why);
                    return String::new();
                }
            };
        }
        let mut text = if selected.len() == 1 {
            format!("{list}: \n")
        } else {
            format!("({list}):\n")
        };
        text.push_str(&self.group(items, first, indent + 1));
        indent_nonempty_lines(&text, indent)
    }
}
