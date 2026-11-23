use super::*;
use content_reader_politics::power_balance::rules;
use content_reader_script::{scope::{Keyword, Scope as Parsed}, scripted_trigger_grammar::Template};

#[cfg(test)]
mod tests;

impl Renderer<'_, '_, '_> {
    pub(in crate::effect_description) fn politics_rules(
        &self,
        item: &Item,
        scopes: Scopes,
        party: bool,
    ) -> Result<String, String> {
        let fields = children(item);
        for item in fields {
            if !(rules().iter().any(|(name, _, _, _)| item.keyword_is(name))
                || ["icon", "desc", "override"]
                    .iter()
                    .any(|name| item.keyword_is(name))
                || (party && item.keyword_is("ideology")))
            {
                return Err(format!("unknown rule {}", key(item)));
            }
        }
        let mut text = if party {
            let ideology = self.politics_ideology(
                field(fields, "ideology").and_then(|item| item.value.scalar()),
                scopes,
            )?;
            let name = self.localised(&ideology, &[], scopes, false)?;
            self.localised(
                b"EFFECT_SET_PARTY_RULE",
                &[("IDEOLOGY", Argument::Text(&name))],
                scopes,
                true,
            )?
        } else {
            self.localised(b"EFFECT_SET_RULE", &[], scopes, true)?
        };
        for &(name, loc, _, fulfilled) in rules() {
            if let Some(item) = field(fields, name) {
                let value = item
                    .value
                    .scalar()
                    .and_then(content_reader_parser::parser::read_bool)
                    .unwrap_or(false);
                text.push_str(&self.rule_prefix(value == fulfilled, scopes)?);
                text.push(' ');
                text.push_str(&self.localised(loc.as_bytes(), &[], scopes, false)?);
                text.push_str(".\n");
            }
        }
        Ok(text)
    }

    pub(in crate::effect_description) fn politics_relation_rule(
        &self,
        item: &Item,
        scopes: Scopes,
        remove: bool,
        template: Option<&Template>,
    ) -> Result<String, String> {
        let fields = children(item);
        let mut rule = &rules()[0];
        let mut value = false;
        for item in fields {
            if let Some(known) = rules().iter().find(|(name, _, _, _)| item.keyword_is(name)) {
                rule = known;
                value = item
                    .value
                    .scalar()
                    .and_then(content_reader_parser::parser::read_bool)
                    .unwrap_or(false);
            } else if !(item.keyword_is("target")
                || item.keyword_is("trigger")
                || (!remove && item.keyword_is("usage_desc")))
            {
                return Err(format!("unknown relation rule {}", key(item)));
            }
        }
        let trigger = field(fields, "trigger");
        let explicit = trigger.is_none() || self.relation_has_country_specifier(fields)?;
        let target = if explicit && field(fields, "target").is_some() {
            self.country_target(item, scopes)?
                .map(|country| self.country_name(country, false))
                .transpose()?
                .unwrap_or_default()
        } else if let Some(item) = trigger {
            let name = item.value.c_str().map_err(|why| why.to_string())?;
            let template = match template {
                Some(template) => template,
                None => {
                    let database = self
                        .politics_content()?
                        .triggers
                        .as_ref()
                        .map_err(Clone::clone)?;
                    match database.lookup(name.as_bytes()) {
                        content_reader_script::scripted_trigger_grammar::Lookup::Template(template) => template,
                        content_reader_script::scripted_trigger_grammar::Lookup::Missing => {
                            return Err(format!(
                                "scripted relation target {name} is absent from recorded content"
                            ));
                        }
                    }
                }
            };
            self.politics_rule_target(template.body.as_ref().map_err(Clone::clone)?, scopes)?
        } else {
            String::new()
        };
        let origin = self.country_name(self.current_country(scopes)?, false)?;
        let header = if remove {
            b"EFFECT_REMOVE_RELATION_RULE_OVERRIDE".as_slice()
        } else {
            b"EFFECT_ADD_RELATION_RULE_OVERRIDE".as_slice()
        };
        let mut text = self.localised(
            header,
            &[
                ("ORIGIN", Argument::Text(&origin)),
                ("TARGET", Argument::Text(&target)),
            ],
            scopes,
            true,
        )?;
        text.push_str(&self.rule_prefix(value, scopes)?);
        text.push_str(&self.localised(rule.1.as_bytes(), &[], scopes, false)?);
        Ok(text)
    }

    fn relation_has_country_specifier(&self, fields: &[Item]) -> Result<bool, String> {
        let mut specified = false;
        for item in fields.iter().filter(|item| item.keyword_is("target")) {
            let operand = content_reader_parser::parser::c_string(item.value.scalar().unwrap_or_default());
            if item.value_keyword().is_some_and(|word| {
                [
                    "root",
                    "this",
                    "prev",
                    "from",
                    "owner",
                    "controller",
                    "occupied",
                    "capital",
                    "random",
                ]
                .contains(&word.as_str())
            }) || operand.starts_with(b"event_target:")
            {
                continue;
            }
            let names = self
                .context
                .content
                .key_names
                .ok_or_else(|| self.content_gap("effect key names"))?;
            specified = names.countries.contains(operand)
                || names.aliases.contains(&operand.to_ascii_lowercase());
        }
        Ok(specified)
    }

    pub(in crate::effect_description) fn politics_rule_target(
        &self,
        body: &[Item],
        scopes: Scopes,
    ) -> Result<String, String> {
        self.rule_trigger_body(body, scopes, false, 0, 2)
    }

    fn rule_prefix(&self, fulfilled: bool, scopes: Scopes) -> Result<String, String> {
        let key = if fulfilled {
            b"TRIGGER_FULLFILLED_PREFIX".as_slice()
        } else {
            b"TRIGGER_UNFULLFILLED_PREFIX".as_slice()
        };
        self.localised(key, &[], scopes, false)
    }

    fn rule_trigger_items<'a>(&self, fields: &'a [Item]) -> Result<Vec<&'a Item>, String> {
        let evaluator = self.evaluator()?;
        let mut items = Vec::new();
        for item in fields {
            if evaluator
                .has_children(std::slice::from_ref(item))
                .map_err(|why| why.to_string())?
            {
                items.push(item);
            }
        }
        Ok(items)
    }

    fn rule_trigger_body(
        &self,
        fields: &[Item],
        scopes: Scopes,
        inverted: bool,
        indent: usize,
        mode: u8,
    ) -> Result<String, String> {
        let mut text = String::new();
        for item in self.rule_trigger_items(fields)? {
            text.push_str(&self.rule_trigger(item, scopes, inverted, indent, mode)?);
        }
        Ok(text)
    }

    fn rule_trigger_prefix(
        &self,
        item: &Item,
        scopes: Scopes,
        inverted: bool,
        indent: usize,
        mode: u8,
    ) -> Result<String, String> {
        let mut text = "   ".repeat(indent);
        if (indent == 0 || mode == 1) && mode != 2 {
            let fulfilled = self
                .evaluator()?
                .evaluate(std::slice::from_ref(item), scopes)
                .decided()?
                != inverted;
            text.push_str(&self.rule_prefix(fulfilled, scopes)?);
        }
        Ok(text)
    }

    fn rule_trigger(
        &self,
        item: &Item,
        scopes: Scopes,
        inverted: bool,
        indent: usize,
        mode: u8,
    ) -> Result<String, String> {
        if item.keyword_is("not") {
            return self.rule_trigger_body(children(item), scopes, !inverted, indent, 0);
        }
        if item.keyword_is("and") || item.keyword_is("or") {
            let count = self.rule_trigger_items(children(item))?.len();
            let or = item.keyword_is("or");
            if !or && (indent == 0 || count < 2) {
                return self.rule_trigger_body(children(item), scopes, inverted, indent, mode);
            }

            let heading = if or != inverted {
                b"TRIGGER_OR_STARTS".as_slice()
            } else {
                b"TRIGGER_AND_STARTS".as_slice()
            };
            let mut text = if or {
                self.rule_trigger_prefix(item, scopes, inverted, indent, mode)?
            } else {
                "   ".repeat(indent)
            };
            text.push_str(&self.localised(heading, &[], scopes, false)?);
            text.push_str(":\n");
            text.push_str(&self.rule_trigger_body(
                children(item),
                scopes,
                inverted,
                indent + 1,
                0,
            )?);
            return Ok(text);
        }
        if item.keyword_is("custom_trigger_tooltip") || item.keyword_is("custom_override_tooltip") {
            let fields = children(item);
            let positive = field(fields, "tooltip")
                .and_then(|item| item.value.scalar())
                .unwrap_or_default();
            let explicit = field(fields, "not_tooltip");
            let negative;
            let operand = if inverted {
                if let Some(item) = explicit {
                    item.value.scalar().ok_or(
                        "custom_override_tooltip not_tooltip bound arguments are not ported",
                    )?
                } else {
                    negative = [positive, b"_NOT"].concat();
                    &negative
                }
            } else {
                if field(fields, "tooltip").is_some_and(|item| item.value.scalar().is_none()) {
                    return Err(
                        "custom_override_tooltip tooltip bound arguments are not ported".into(),
                    );
                }
                positive
            };
            let own = self.localised(operand, &[], scopes, true)?;
            if own.is_empty() {
                return Ok(own);
            }
            let mut text = self.rule_trigger_prefix(item, scopes, inverted, indent, mode)?;
            text.push_str(&own);
            if !text.ends_with('\n') {
                text.push('\n');
            }
            return Ok(text);
        }
        if item.keyword_is("has_country_flag") {
            let flag = if let Some(fields) = item.value.items() {
                if let Some(item) = fields.iter().find(|item| !item.keyword_is("flag")) {
                    return Err(format!(
                        "has_country_flag field {} description is unavailable",
                        key(item)
                    ));
                }
                field(fields, "flag")
                    .and_then(|item| item.value.scalar())
                    .unwrap_or_default()
            } else {
                item.value.scalar().unwrap_or_default()
            };
            if flag
                .iter()
                .position(|&byte| byte == b'@')
                .is_some_and(|at| at > 0)
            {
                return Err(format!(
                    "has_country_flag dynamic flag {} description is unavailable",
                    String::from_utf8_lossy(flag)
                ));
            }
            let country = self.expanded("[THIS.GetNameWithFlag]", scopes)?;
            let prefix = if inverted {
                b"TRIGGER_COUNTRY_FLAG_NOT_SET".as_slice()
            } else {
                b"TRIGGER_COUNTRY_FLAG_SET".as_slice()
            };
            let mut own = self.localised(
                prefix,
                &[("COUNTRY", Argument::Text(&country))],
                scopes,
                true,
            )?;
            let flag = self.localised(flag, &[], scopes, true)?;
            own.push_str(&self.localised(
                b"TRIGGER_FLAG_LIST_ENTRY",
                &[("FLAG", Argument::Text(&flag))],
                scopes,
                true,
            )?);
            own.push('\n');
            let mut text = self.rule_trigger_prefix(item, scopes, inverted, indent, mode)?;
            text.push_str(&own);
            return Ok(text);
        }
        if item.keyword_is("is_on_continent") {
            let operand = item.value.c_str().map_err(|why| why.to_string())?;
            let content = self.politics_content()?;
            let continent = content
                .continents
                .as_ref()
                .map_err(Clone::clone)?
                .iter()
                .find(|name| name.eq_ignore_ascii_case(operand))
                .ok_or_else(|| {
                    format!("is_on_continent continent {operand} is absent from recorded content")
                })?;
            let prefixed = format!("{}_{continent}", self.tag(self.context.perspective)?);
            let name = self.localised(
                if self.context.localisation.get(&prefixed).is_some() {
                    prefixed.as_bytes()
                } else {
                    continent.as_bytes()
                },
                &[],
                scopes,
                false,
            )?;
            let own = self.line(
                if inverted {
                    b"TRIGGER_IS_NOT_ON_CONTINENT"
                } else {
                    b"TRIGGER_IS_ON_CONTINENT"
                },
                &[("CONTINENT", Argument::Text(&name))],
                scopes,
                false,
            )?;
            let mut text = self.rule_trigger_prefix(item, scopes, inverted, indent, mode)?;
            text.push_str(&own);
            return Ok(text);
        }
        let evaluator = self.evaluator()?;
        let parsed =
            content_reader_script::scope::parse(&key(item), item.key_quoted, evaluator.world().recorded());
        if item.value.items().is_some() && !matches!(parsed, Parsed::Unknown(_)) {
            let entered = evaluator
                .enter_scope(&parsed, scopes)
                .map_err(|why| why.to_string())?
                .ok_or_else(|| format!("context trigger target {} resolves no scope", key(item)))?;
            let name = match parsed {
                Parsed::Keyword(
                    keyword @ (Keyword::Owner
                    | Keyword::Controller
                    | Keyword::CapitalScope
                    | Keyword::Overlord
                    | Keyword::FactionLeader
                    | Keyword::Occupied),
                ) => {
                    let key = match keyword {
                        Keyword::Owner => b"TRIGGER_OWNER".as_slice(),
                        Keyword::Controller => b"TRIGGER_CONTROLLER".as_slice(),
                        Keyword::CapitalScope => b"TRIGGER_CAPITAL".as_slice(),
                        Keyword::Overlord => b"TRIGGER_OVERLORD".as_slice(),
                        Keyword::FactionLeader => b"TRIGGER_FACTION_LEADER".as_slice(),
                        Keyword::Occupied => b"TRIGGER_OCCUPIED".as_slice(),
                        _ => unreachable!(),
                    };
                    self.localised(key, &[], scopes, true)?
                }
                _ => {
                    let name = match entered.this {
                        Scope::Country(country) | Scope::CountryLeader { country, .. } => {
                            self.country_name(country, false)?
                        }
                        Scope::State(state) => self.state_name(state)?,
                        _ => {
                            return Err(format!(
                                "context trigger target {} name is not ported",
                                key(item)
                            ));
                        }
                    };
                    format!("\u{11}Y{name}\u{11}!")
                }
            };
            let count = self.rule_trigger_items(children(item))?.len();
            let mut text = self.rule_trigger_prefix(item, scopes, inverted, indent, mode)?;
            text.push_str(&name);
            if count < 2 {
                text.push_str(": ");
                text.push_str(&self.rule_trigger_body(children(item), entered, inverted, 0, 2)?);
            } else {
                text.push_str(":\n");
                text.push_str(&self.rule_trigger_body(
                    children(item),
                    entered,
                    inverted,
                    indent + 1,
                    0,
                )?);
            }
            return Ok(text);
        }
        Err(format!(
            "target trigger {} description is unavailable",
            key(item)
        ))
    }
}
