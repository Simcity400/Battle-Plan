use super::*;
use crate::triggers::Eval;

impl Renderer<'_, '_, '_> {
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

    pub(super) fn condition(&self, items: &[Item], scopes: Scopes) -> Eval {
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
    pub(super) fn multiple(&mut self, items: &[Item], scopes: Scopes) -> bool {
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

    pub(super) fn tooltip_group(&mut self, item: &Item, scopes: Scopes, indent: usize) -> String {
        self.group(children(item), scopes, indent)
    }

    pub(super) fn hidden_group(&mut self, item: &Item, scopes: Scopes, indent: usize) -> String {
        self.group(children(item), scopes, indent);
        String::new()
    }
}

// The control/list own-description virtual is empty (140176EE0).
pub(super) fn own() -> Result<String, String> {
    Ok(String::new())
}
