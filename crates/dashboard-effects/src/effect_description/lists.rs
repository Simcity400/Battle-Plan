use super::*;
use crate::triggers::Eval;

#[cfg(test)]
mod tests;

impl Renderer<'_, '_, '_> {
    fn adds_child(&self, item: &Item) -> bool {
        self.context
            .content
            .key_names
            .map(|names| content_reader_script::effect_keys::adds_child(item, names).unwrap_or(true))
            .unwrap_or_else(|| self.effect_class(item).is_some())
    }

    pub(super) fn attached<'a>(
        &mut self,
        items: &'a [Item],
        report: bool,
    ) -> Vec<(&'a Item, Vec<&'a Item>)> {
        let mut out: Vec<(&Item, Vec<&Item>)> = Vec::new();
        let mut last_added = None;
        for item in items {
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
                    self.refuse(&key(item), "else/else_if does not follow an open if chain");
                }
                continue;
            }
            let added = self.adds_child(item);
            out.push((item, Vec::new()));
            if added {
                last_added = Some(out.len() - 1);
            }
        }
        out
    }

    pub(super) fn limit<'a>(&self, items: &'a [Item]) -> Result<&'a [Item], String> {
        let evaluator = self.evaluator()?;
        for body in items
            .iter()
            .filter(|item| item.keyword_is("limit"))
            .map(children)
        {
            if evaluator
                .has_children(body)
                .map_err(|why| why.to_string())?
            {
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
            Err(why) => Eval::unknown(why),
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
                    let body = self.effect_children(branch);
                    return Some(if probe {
                        if self.multiple(&body, scopes) {
                            "multiple".into()
                        } else {
                            String::new()
                        }
                    } else {
                        self.group(&body, scopes, indent)
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

    pub(super) fn multiple(&mut self, items: &[Item], scopes: Scopes) -> bool {
        let children = self.attached(items, false);
        if let [(item, branches)] = children.as_slice() {
            let Some(own) = self.own(item, scopes) else {
                return false;
            };
            return own.is_empty() && self.multiple_child(item, branches, scopes);
        }
        let mut count = 0;
        for (item, branches) in children {
            let contributes = self
                .own(item, scopes)
                .is_some_and(|own| !own.is_empty() || self.multiple_child(item, &branches, scopes));
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
        if self.iterator_multiple(item) {
            return true;
        }
        match self.effect_class(item).map(|class| class.vtable) {
            Some(0x1427f3a70) => field(children(item), "tooltip")
                .and_then(|item| item.value.scalar())
                .is_some_and(|key| !key.is_empty()),
            Some(0x1428527b0) => self
                .if_chain(item, branches, scopes, 0, true)
                .is_some_and(|text| !text.is_empty()),
            _ => {
                let body = self.effect_children(item);
                self.multiple(&body, scopes)
            }
        }
    }

    pub(super) fn tooltip_group(&mut self, item: &Item, scopes: Scopes, indent: usize) -> String {
        let body = self.effect_children(item);
        self.group(&body, scopes, indent)
    }

    pub(super) fn hidden_group(&mut self, item: &Item, scopes: Scopes, indent: usize) -> String {
        let body = self.effect_children(item);
        self.group(&body, scopes, indent);
        String::new()
    }
}

pub(super) fn own() -> Result<String, String> {
    Ok(String::new())
}

impl Renderer<'_, '_, '_> {
    pub(super) fn list_builder(
        &mut self,
        item: &Item,
        scopes: Scopes,
        indent: usize,
    ) -> Option<String> {
        let class = self.effect_class(item)?;
        let result = match class.vtable {
            effect_identity::SCRIPTED => self.scripted_effect(item, scopes, indent),
            0x142A321B0 => self.random_effect(item, scopes, indent),
            0x142A32320 => self
                .random_list(item, scopes)
                .map(|text| format!("{}{text}", "   ".repeat(indent))),
            0x1427f3a70 => self.scope_loop(item, scopes, indent),
            0x1427F5440 => self.meta_effect(item, scopes, indent),
            0x142A320B8 => self.override_tooltip(item, scopes, indent),
            _ => return None,
        };
        Some(match result {
            Ok(text) => text,
            Err(why) => {
                self.refuse(&key(item), why);
                String::new()
            }
        })
    }

    fn scripted_effect(
        &mut self,
        item: &Item,
        scopes: Scopes,
        indent: usize,
    ) -> Result<String, String> {
        if self.scripted_stack.len() >= 64 {
            return Err(
                "scripted-effect description depth exceeds the 64-level stack limit".into(),
            );
        }
        let name = content_reader_parser::parser::c_string(item.key.as_deref().unwrap_or_default())
            .to_ascii_lowercase();
        if self
            .scripted_stack
            .iter()
            .any(|(active, this)| *active == name && *this == scopes.this)
        {
            return Err(format!(
                "scripted effect {} recursively calls an active template in the same THIS scope; this recursive expansion is unsupported",
                String::from_utf8_lossy(&name)
            ));
        }
        let database = self
            .context
            .content
            .scripted_effects
            .ok_or_else(|| self.content_gap("scripted effects"))?;
        let body = database
            .bodies
            .get(&name)
            .ok_or("registered scripted-effect template has no body")?
            .as_ref()
            .map_err(Clone::clone)?;
        self.scripted_stack.push((name, scopes.this));
        let text = self.group(body, scopes, indent);
        self.scripted_stack.pop();
        Ok(text)
    }

    fn random_effect(
        &mut self,
        item: &Item,
        scopes: Scopes,
        indent: usize,
    ) -> Result<String, String> {
        let fields = children(item);
        let value = field(fields, "chance")
            .and_then(|item| item.value.scalar())
            .unwrap_or(b"0");
        let chance = crate::triggers::fixed_round(self.scoped_value(value, scopes)?) as i32;
        let mut text = "   ".repeat(indent);
        text.push_str(&self.localised(
            b"RANDOM_EFFECT",
            &[("CHANCE", Argument::Fixed5(i64::from(chance) * 1000))],
            scopes,
            false,
        )?);
        let body = self.effect_children(item);
        let count = self
            .attached(&body, false)
            .iter()
            .filter(|(child, _)| self.adds_child(child))
            .count();
        if count < 2 {
            text.push_str(&self.group(&body, scopes, 0));
        } else {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&self.group(&body, scopes, indent + 1));
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(text)
    }

    pub(super) fn random_list(&mut self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let members = content_reader_script::effect_keys::random_list_members(item)?;
        let mut weights = Vec::new();
        let mut total = 0_i64;
        for member in &members {
            let operand = member.key.as_deref().unwrap_or_default();
            let mut weight = if self
                .evaluator()?
                .scoped_exists(operand, scopes)
                .map_err(|why| why.to_string())?
            {
                self.scoped_value(operand, scopes)?
            } else {
                0
            };
            for modifier in children(member)
                .iter()
                .filter(|item| item.keyword_is("modifier"))
            {
                let fields = children(modifier);
                let condition = without(fields, &["factor", "add"]);
                match self.evaluator()?.evaluate(&condition, scopes) {
                    Eval::True => {
                        let factor = field(fields, "factor")
                            .and_then(|item| item.value.scalar())
                            .unwrap_or(b"1");
                        let add = field(fields, "add")
                            .and_then(|item| item.value.scalar())
                            .unwrap_or(b"0");
                        weight = weight.wrapping_mul(self.scoped_value(factor, scopes)?) / 100_000;
                        weight = weight.wrapping_add(self.scoped_value(add, scopes)?);
                    }
                    Eval::False => {}
                    Eval::Unknown(why) => {
                        return Err(why.into_iter().collect::<Vec<_>>().join("; "));
                    }
                }
            }
            weights.push(weight);
            total = total.wrapping_add(weight);
        }
        if total < 1 {
            return Ok(String::new());
        }
        let size = members.len().to_string();
        let mut text = self.localised(
            b"RANDOM_LIST_EFFECT",
            &[("SIZE", Argument::Text(&size))],
            scopes,
            false,
        )?;
        let cursor = scopes
            .cursor
            .ok_or("random_list needs an event graph cursor")?;
        let seed = self.graph.borrow().scopes[cursor];
        for (member, weight) in members.iter().zip(weights) {
            let chance = if matches!(weight, i64::MAX | i64::MIN) || weight == i64::MIN + 1 {
                0xffff_ffff
            } else {
                weight.wrapping_mul(100_000) / total
            };
            text.push_str(&self.localised(
                b"RANDOM_EFFECT",
                &[("CHANCE", Argument::Fixed5(chance))],
                scopes,
                false,
            )?);
            {
                let mut graph = self.graph.borrow_mut();
                graph.scopes[cursor].random_seed = seed.random_seed;
                graph.scopes[cursor].random_counter = seed.random_counter;
                graph.scopes[cursor].random_seed_refusal = seed.random_seed_refusal;
            }
            let body = self.group(
                &content_reader_script::effect_keys::random_member_fields(member),
                scopes,
                0,
            );
            let body = trim_description(&body);
            if body.len() < 2 {
                text.push_str(&self.localised(b"NO_EFFECT", &[], scopes, false)?);
            } else {
                text.push_str(body);
            }
            text.push('\n');
        }
        Ok(text)
    }

    fn scope_loop(&mut self, item: &Item, scopes: Scopes, indent: usize) -> Result<String, String> {
        let fields = children(item);
        let Some(tooltip) = field(fields, "tooltip")
            .and_then(|item| item.value.read_string())
            .filter(|key| !key.is_empty())
        else {
            return Ok(String::new());
        };
        let array = field(fields, "array")
            .and_then(|item| item.value.scalar())
            .unwrap_or_default();
        let Some(target) = self
            .evaluator()?
            .scoped_array_scopes(array, scopes)
            .map_err(|why| why.to_string())?
            .into_iter()
            .flatten()
            .next()
        else {
            return Ok(String::new());
        };
        let object = match target {
            Scope::Country(id) | Scope::CountryLeader { country: id, .. } => {
                crate::scripted_text::EventObject::Country(id)
            }
            Scope::State(id) => crate::scripted_text::EventObject::State(id),
            Scope::Province(id) => crate::scripted_text::EventObject::Province(id),
            Scope::Character(id) => crate::scripted_text::EventObject::Character(id),
            Scope::Leader(id) => crate::scripted_text::EventObject::UnitLeader(id),
            Scope::IndustrialOrganisation(id) => {
                crate::scripted_text::EventObject::IndustrialOrganisation(id)
            }
            Scope::Project { owner, id } => {
                crate::scripted_text::EventObject::Project { owner, id }
            }
            Scope::Raid(id) => crate::scripted_text::EventObject::Raid(id),
            Scope::Unit(id) => crate::scripted_text::EventObject::Unit(id),
            Scope::Combat => return Err("array combat scope is unavailable".into()),
        };
        let cursor = scopes.cursor.ok_or("scope loop needs a graph cursor")?;
        let at = {
            let mut graph = self.graph.borrow_mut();
            let caller = *graph
                .scopes
                .get(cursor)
                .ok_or("scope loop caller is absent")?;
            let mut selected = crate::scripted_text::EventScope::of(object);
            selected.root = caller.root;
            selected.from = caller.from;
            selected.prev = crate::scripted_text::Link::Scope(cursor);
            let at = graph.scopes.len();
            if selected.root == crate::scripted_text::Link::Itself {
                selected.root = crate::scripted_text::Link::Scope(cursor);
            }
            if selected.from == crate::scripted_text::Link::Itself {
                selected.from = crate::scripted_text::Link::Scope(cursor);
            }
            graph.scopes.push(selected);
            at
        };
        let selected = self
            .evaluator()?
            .event_scopes(at)
            .map_err(|why| why.to_string())?;
        let mut text = "   ".repeat(indent);
        text.push_str(&self.line(tooltip, &[], scopes, true)?);
        let body = self.effect_children(item);
        text.push_str(&self.group(&body, selected, indent + 1));
        Ok(text)
    }

    fn meta_effect(
        &mut self,
        item: &Item,
        scopes: Scopes,
        indent: usize,
    ) -> Result<String, String> {
        let fields = children(item);
        let raw = match field(fields, "text") {
            Some(item) => item
                .value
                .scalar()
                .ok_or("meta_effect text must be read as raw text")?,
            None => b"".as_slice(),
        };
        let mut parameters = BTreeMap::new();
        for parameter in fields
            .iter()
            .filter(|item| !item.keyword_is("text") && !item.keyword_is("debug"))
        {
            let name = parameter.key.as_deref().unwrap_or_default();
            let value = parameter
                .value
                .scalar()
                .ok_or("meta_effect parameter is not scalar")?;
            let value = if parameter.quoted() {
                let text = std::str::from_utf8(content_reader_parser::parser::c_string(value))
                    .map_err(|_| "meta_effect parameter is not UTF-8")?;
                if text.contains('[') {
                    self.expanded(text, scopes)?
                } else {
                    text.to_owned()
                }
            } else if value
                .iter()
                .enumerate()
                .all(|(i, byte)| byte.is_ascii_digit() || i == 0 && matches!(byte, b'+' | b'-'))
            {
                integer(value).to_string()
            } else {
                content_reader_parser::fixed5::fixed5_to_string(self.scoped_value(value, scopes)?, -1)
            };
            parameters.entry(name.to_vec()).or_insert(value);
        }
        let mut generated = Vec::new();
        let mut remaining = raw;
        while let Some(start) = remaining.iter().position(|byte| *byte == b'[') {
            generated.extend_from_slice(&remaining[..start]);
            let end = remaining[start + 1..]
                .iter()
                .position(|byte| *byte == b']')
                .ok_or("meta_effect parameter bracket is not closed")?
                + start
                + 1;
            if let Some(value) = parameters.get(&remaining[start + 1..end]) {
                generated.extend_from_slice(value.as_bytes());
            }
            remaining = &remaining[end + 1..];
        }
        generated.extend_from_slice(remaining);
        let names = self
            .context
            .content
            .key_names
            .ok_or_else(|| self.content_gap("effect key names"))?;
        let body = content_reader_script::effect_keys::read_body(&generated, names)?;
        Ok(self.group(&body, scopes, indent))
    }

    pub(super) fn override_own(&mut self, item: &Item, scopes: Scopes) -> Result<String, String> {
        field(children(item), "tooltip")
            .filter(|item| !item.value.read_string().is_some_and(|key| key.is_empty()))
            .map(|tooltip| self.custom_tooltip(tooltip, scopes))
            .unwrap_or_else(|| Ok(String::new()))
    }

    fn override_tooltip(
        &mut self,
        item: &Item,
        scopes: Scopes,
        indent: usize,
    ) -> Result<String, String> {
        let own = self.override_own(item, scopes)?;
        let mut text = if own.len() > 1 {
            format!("{}{own}", "   ".repeat(indent))
        } else {
            String::new()
        };
        let children = self.effect_children(item);
        let body = self.group(&children, scopes, indent);
        append_description(&mut text, &body);
        Ok(text)
    }
}
