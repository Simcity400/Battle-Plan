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

    /// Native attachment follows the last added child, not the last lexical field.
    pub(super) fn attached<'a>(
        &mut self,
        items: &'a [Item],
        report: bool,
    ) -> Vec<(&'a Item, Vec<&'a Item>)> {
        let mut out: Vec<(&Item, Vec<&Item>)> = Vec::new();
        let mut last_added = None;
        for item in items {
            if matches!(
                key(item).to_ascii_lowercase().as_str(),
                "limit"
                    | "tooltip"
                    | "visible_when_empty"
                    | "display_individual_scopes"
                    | "random_select_amount"
            ) {
                continue;
            }
            if item.keyword_is("else") || item.keyword_is("else_if") {
                if let Some(at) = last_added {
                    let (prior, branches): &mut (&Item, Vec<&Item>) = &mut out[at];
                    if prior.keyword_is("if")
                        && !branches.iter().any(|branch| branch.keyword_is("else"))
                    {
                        branches.push(item);
                        continue;
                    }
                }
                if report {
                    self.refuse(
                        &key(item),
                        "else/else_if does not follow an open if chain (14137D840/141385BA0)"
                            .into(),
                    );
                }
                continue;
            }
            let added = self
                .context
                .content
                .key_names
                .map(|names| installed_content::effect_keys::adds_child(item, names).unwrap_or(true))
                .unwrap_or_else(|| {
                    installed_content::effect_keys::REGISTERED_EFFECTS
                        .iter()
                        .any(|(_, name)| name.eq_ignore_ascii_case(&key(item)))
                        || self.is_context(item)
                });
            out.push((item, Vec::new()));
            if added {
                last_added = Some(out.len() - 1);
            }
        }
        out
    }

    fn limit<'a>(&self, items: &'a [Item]) -> Result<&'a [Item], String> {
        // 141385BA0 reads another limit only while the embedded trigger has no children.
        let evaluator = self.evaluator()?;
        for body in items
            .iter()
            .filter(|item| item.keyword_is("limit"))
            .map(children)
        {
            if evaluator.has_children(body)? {
                return Ok(body);
            }
        }
        Ok(&[])
    }

    fn condition(&self, items: &[Item], scopes: Scopes) -> Eval {
        match self
            .limit(items)
            .and_then(|body| Ok(self.evaluator()?.evaluate(body, scopes)))
        {
            Ok(result) => result,
            Err(why) => Eval::Unknown([why].into_iter().collect()),
        }
    }

    pub(super) fn if_chain(
        &mut self,
        item: &Item,
        branches: &[&Item],
        scopes: Scopes,
        indent: usize,
        probe: bool,
    ) -> Option<String> {
        for branch in std::iter::once(item).chain(branches.iter().copied()) {
            let result = if branch.keyword_is("else") {
                Eval::True
            } else {
                self.condition(children(branch), scopes)
            };
            match result {
                Eval::True => {
                    return Some(if probe {
                        if self.multiple(children(branch), scopes) {
                            "multiple".into()
                        } else {
                            String::new()
                        }
                    } else {
                        self.group(children(branch), scopes, indent)
                    });
                }
                Eval::False => {}
                Eval::Unknown(why) => {
                    self.refuse("if", why.into_iter().collect::<Vec<_>>().join("; "));
                    return None;
                }
            }
        }
        Some(String::new())
    }

    // HasMultipleDescriptions@140533D80 invokes own descriptions, then vf11.
    // These probes are deliberately not replaced by a cached body/newline count.
    fn multiple(&mut self, items: &[Item], scopes: Scopes) -> bool {
        let children = self.attached(items, false);
        if let [(item, branches)] = children.as_slice() {
            let own = self.own(item, scopes);
            return own.is_empty() && self.multiple_child(item, branches, scopes);
        }
        let mut count = 0;
        for (item, branches) in children {
            let contributes =
                !self.own(item, scopes).is_empty() || self.multiple_child(item, &branches, scopes);
            if contributes {
                count += 1;
            }
            if count >= 2 {
                return true;
            }
        }
        false
    }

    fn multiple_child(&mut self, item: &Item, branches: &[&Item], scopes: Scopes) -> bool {
        if item.keyword_is("if") {
            self.if_chain(item, branches, scopes, 0, true)
                .is_some_and(|text| !text.is_empty())
        } else if self.is_context(item) {
            // Context vf11 is the base virtual (14299F750): this probe keeps
            // the supplied scope. Its vf8 separately probes the selected scope.
            self.multiple(children(item), scopes)
        } else if matches!(
            key(item).to_ascii_lowercase().as_str(),
            "effect_tooltip" | "hidden_effect" | "every_state"
        ) {
            self.multiple(children(item), scopes)
        } else {
            false
        }
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
                let name = self.context.text.state_name(state)?;
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
            match self.context.text.state_name(*id) {
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
