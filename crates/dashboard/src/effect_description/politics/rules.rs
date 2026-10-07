use super::*;
use installed_content::{
    power_balance::RULES,
    scope::{Keyword, Scope as Parsed},
    scripted_triggers::Template,
};

#[cfg(test)]
mod tests;

impl Renderer<'_, '_, '_> {
    // Own 140393260 / 140392590; list 140325CC0 / 140325AF0.
    pub(in crate::effect_description) fn politics_rules(
        &self,
        item: &Item,
        scopes: Scopes,
        party: bool,
    ) -> Result<String, String> {
        let fields = children(item);
        for item in fields {
            if !(RULES.iter().any(|(name, _, _, _)| item.keyword_is(name))
                || ["icon", "desc", "override"]
                    .iter()
                    .any(|name| item.keyword_is(name))
                || (party && item.keyword_is("ideology")))
            {
                return Err(format!("unknown rule {} (14062EDC0)", key(item)));
            }
        }
        let mut text = if party {
            let ideology = self.politics_ideology(
                field(fields, "ideology").and_then(|item| item.value.scalar()),
                scopes,
            )?;
            let name = self.localised(&ideology, &[], scopes, false)?; // 140616BC0.
            self.localised(
                b"EFFECT_SET_PARTY_RULE", // 14275AE80; IDEOLOGY@142750150.
                &[("IDEOLOGY", Argument::Text(&name))],
                scopes,
                true,
            )?
        } else {
            self.localised(b"EFFECT_SET_RULE", &[], scopes, true)? // 14275ACC8.
        };
        // 14062C6C0(index, false, true): ignore desc/override; compare +0x30.
        for &(name, loc, _, fulfilled) in RULES {
            if let Some(item) = field(fields, name) {
                let value = item
                    .value
                    .scalar()
                    .and_then(installed_content::parser::read_bool)
                    .unwrap_or(false);
                text.push_str(&self.rule_prefix(value == fulfilled, scopes)?);
                text.push(' '); // 142700920.
                text.push_str(&self.localised(loc.as_bytes(), &[], scopes, false)?);
                text.push_str(".\n"); // 14271CCC0.
            }
        }
        Ok(text)
    }

    // Own 1403769B0 / 14038AFB0. usage_desc is postload-only (1403A6540).
    pub(in crate::effect_description) fn politics_relation_rule(
        &self,
        item: &Item,
        scopes: Scopes,
        remove: bool,
        template: Option<&Template>,
    ) -> Result<String, String> {
        let fields = children(item);
        let mut rule = &RULES[0];
        let mut value = false; // Constructor 14032C410.
        for item in fields {
            if let Some(known) = RULES.iter().find(|(name, _, _, _)| item.keyword_is(name)) {
                rule = known;
                value = item
                    .value
                    .scalar()
                    .and_then(installed_content::parser::read_bool)
                    .unwrap_or(false);
            } else if !(item.keyword_is("target")
                || item.keyword_is("trigger")
                // Only the add reader consumes usage_desc (+0x98, 1403AF060).
                || (!remove && item.keyword_is("usage_desc")))
            {
                return Err(format!(
                    "unknown relation rule {} (1403AF060/1403B4150)",
                    key(item)
                ));
            }
        }
        let trigger = field(fields, "trigger");
        // Postload 1403A7170 tests the decoded specifier (140BA6220), not
        // target presence. Invalid strings and scope selectors permit a template.
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
                        installed_content::scripted_triggers::Lookup::Template(template) => template,
                        installed_content::scripted_triggers::Lookup::Missing => {
                            return Err(format!(
                                "scripted relation target {name} is absent from recorded content (1403A7170)"
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
            b"EFFECT_REMOVE_RELATION_RULE_OVERRIDE".as_slice() // 14275B158.
        } else {
            b"EFFECT_ADD_RELATION_RULE_OVERRIDE".as_slice() // 14275B020.
        };
        let mut text = self.localised(
            header,
            &[
                ("ORIGIN", Argument::Text(&origin)),
                ("TARGET", Argument::Text(&target)),
            ], // 14275B018/1427528E4.
            scopes,
            true,
        )?;
        text.push_str(&self.rule_prefix(value, scopes)?);
        text.push_str(&self.localised(rule.1.as_bytes(), &[], scopes, false)?); // 14062D120; no separator/LF.
        Ok(text)
    }

    fn relation_has_country_specifier(&self, fields: &[Item]) -> Result<bool, String> {
        let mut specified = false;
        for item in fields.iter().filter(|item| item.keyword_is("target")) {
            let operand = installed_content::parser::c_string(item.value.scalar().unwrap_or_default());
            // ParseScopeSelector 140530BB0 stores flags without touching +0x80.
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
            // AssignCountrySpecifierFromString 140BA53F0: exact tag, then alias
            // token. A missing/empty name resets both selector alternatives.
            specified = names.countries.contains(operand)
                || names.aliases.contains(&operand.to_ascii_lowercase());
        }
        Ok(specified)
    }

    // Template vf12 14053EA10, called with inversion=false, indent=0, mode=2.
    pub(in crate::effect_description) fn politics_rule_target(
        &self,
        body: &[Item],
        scopes: Scopes,
    ) -> Result<String, String> {
        self.rule_trigger_body(body, scopes, false, 0, 2)
    }

    fn rule_prefix(&self, fulfilled: bool, scopes: Scopes) -> Result<String, String> {
        let key = if fulfilled {
            b"TRIGGER_FULLFILLED_PREFIX".as_slice() // 14275B048.
        } else {
            b"TRIGGER_UNFULLFILLED_PREFIX".as_slice() // 14275B068.
        };
        self.localised(key, &[], scopes, false)
    }

    fn rule_trigger_items<'a>(&self, fields: &'a [Item]) -> Result<Vec<&'a Item>, String> {
        let evaluator = self.evaluator()?;
        let mut items = Vec::new();
        for item in fields {
            if evaluator.has_children(std::slice::from_ref(item))? {
                items.push(item);
            }
        }
        Ok(items)
    }

    // CTrigger::BuildChildTooltips 14053FCC0; mode=2 suppresses Check/prefix.
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
        let mut text = "   ".repeat(indent); // 142773CD8.
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
            // CNotTrigger vf12 14053F060 flips inversion and resets mode.
            return self.rule_trigger_body(children(item), scopes, !inverted, indent, 0);
        }
        if item.keyword_is("and") || item.keyword_is("or") {
            let count = self.rule_trigger_items(children(item))?.len();
            let or = item.keyword_is("or");
            if !or && (indent == 0 || count < 2) {
                return self.rule_trigger_body(children(item), scopes, inverted, indent, mode);
            }
            // CAndTrigger 14053EA10 / COrTrigger 14053F450.
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
            // Own 1413931A0; descendants vf12 140176EE0 are empty.
            let fields = children(item);
            let positive = field(fields, "tooltip")
                .and_then(|item| item.value.scalar())
                .unwrap_or_default();
            let explicit = field(fields, "not_tooltip");
            let negative;
            let operand = if inverted {
                if let Some(item) = explicit {
                    item.value.scalar().ok_or("CCustomOverrideTooltipTrigger not_tooltip bound arguments are not ported (141395B30)")?
                } else {
                    negative = [positive, b"_NOT"].concat(); // 141396000.
                    &negative
                }
            } else {
                if field(fields, "tooltip").is_some_and(|item| item.value.scalar().is_none()) {
                    return Err("CCustomOverrideTooltipTrigger tooltip bound arguments are not ported (141395B30)".into());
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
            // CFlagTrigger own 141393260 -> CCountryFlagTrigger 140430D90.
            let flag = if let Some(fields) = item.value.items() {
                if let Some(item) = fields.iter().find(|item| !item.keyword_is("flag")) {
                    return Err(format!(
                        "CCountryFlagTrigger field {} description is not ported (141393260)",
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
                    "CCountryFlagTrigger dynamic flag {} description is not ported (141390890)",
                    String::from_utf8_lossy(flag)
                ));
            }
            let country = self.expanded("[THIS.GetNameWithFlag]", scopes)?; // Shared 140BA59E0/1406E63A0 path.
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
            )?); // FLAG@142943A20.
            own.push('\n');
            let mut text = self.rule_trigger_prefix(item, scopes, inverted, indent, mode)?;
            text.push_str(&own);
            return Ok(text);
        }
        if item.keyword_is("is_on_continent") {
            // Own 14046CBA0; reader 140471500 interns the scalar token.
            let operand = item.value.c_str().map_err(|why| why.to_string())?;
            let content = self.politics_content()?;
            let continent = content.continents.as_ref().map_err(Clone::clone)?.iter()
                .find(|name| name.eq_ignore_ascii_case(operand))
                .ok_or_else(|| format!("CIsOnContinentTrigger continent {operand} is absent from recorded content (140AAE420)"))?;
            // LocalizeForCurrentCountry 140BBA630 -> 140BBA780; no scope expansion.
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
                }, // 14279BC00 / 14279BBE8.
                &[("CONTINENT", Argument::Text(&name))], // 14279BBD8.
                scopes,
                false,
            )?;
            let mut text = self.rule_trigger_prefix(item, scopes, inverted, indent, mode)?;
            text.push_str(&own);
            return Ok(text);
        }
        // CContextTrigger 14138D980; use the one scope parser and evaluator.
        let evaluator = self.evaluator()?;
        let parsed =
            installed_content::scope::parse(&key(item), item.key_quoted, evaluator.world().recorded());
        if item.value.items().is_some() && !matches!(parsed, Parsed::Unknown(_)) {
            let entered = evaluator.enter_scope(&parsed, scopes)?.ok_or_else(|| {
                format!(
                    "CContextTrigger target {} resolves no scope (14146BB30)",
                    key(item)
                )
            })?;
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
                                "CContextTrigger target {} name is not ported (141390A70)",
                                key(item)
                            ));
                        }
                    };
                    format!("\u{11}Y{name}\u{11}!") // FormatEventTargetTooltip 141390A70.
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
            "target trigger {} native description is not ported (CTrigger vf21/vf12 14053FCC0)",
            key(item)
        ))
    }
}
