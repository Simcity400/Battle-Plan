//! Narrow adapter for raid diagnostic mode1. L4's shared trigger text can
//! replace this module without changing the recorded schema or line rules.
use super::super::rules::Lines;
use super::*;
use installed_content::triggers::{Trigger, TriggerValue, Triggers};
use std::collections::BTreeSet;

impl Context<'_, '_> {
    pub(super) fn trigger_text(&self, triggers: &Triggers, scopes: Scopes, name: &str) -> Lines {
        let mut out = Vec::new();
        self.trigger_children(
            &triggers.0,
            scopes,
            (false, 0),
            &mut BTreeSet::new(),
            &mut out,
            name,
        );
        out
    }
    fn trigger_children(
        &self,
        triggers: &[Trigger],
        scopes: Scopes,
        mode: (bool, usize),
        path: &mut BTreeSet<String>,
        out: &mut Lines,
        name: &str,
    ) {
        let (invert, indent) = mode;
        for trigger in triggers {
            let line = format!("{name}: {}", trigger.name);
            let children = match &trigger.value {
                TriggerValue::Block(b) => Some(b.0.as_slice()),
                _ => None,
            };
            match trigger.name.as_str() {
                "hidden_trigger" => {} // CHiddenTrigger slot 12 emits no text.
                "AND" | "and" | "OR" | "or" | "NOT" | "not" => {
                    let Some(children) = children else {
                        out.push((line, Err("logical trigger requires block".into())));
                        continue;
                    };
                    let not = trigger.name.eq_ignore_ascii_case("NOT");
                    let or = trigger.name.eq_ignore_ascii_case("OR");
                    let heading = or || (!not && indent > 0 && children.len() >= 2);
                    if heading {
                        // 14053F450 OR always has a heading; 14053EA10 AND
                        // only at positive indentation with multiple children.
                        let key = if or != invert {
                            "TRIGGER_OR_STARTS"
                        } else {
                            "TRIGGER_AND_STARTS"
                        };
                        let result = self
                            .trigger_prefix(trigger, scopes, invert, indent)
                            .and_then(|prefix| {
                                localise(self.loc, key, &[])
                                    .map(|text| format!("{prefix}{text}:\n"))
                            });
                        out.push((line, result));
                    }
                    // NOT 14053F060 toggles inversion without adding indentation.
                    self.trigger_children(
                        children,
                        scopes,
                        (invert ^ not, indent + usize::from(heading)),
                        path,
                        out,
                        name,
                    );
                }
                key if self.assets.catalog.scripted_triggers.contains_key(key) => {
                    // 14138E230: inversion == expected, then template slot 12.
                    let expected = match trigger.value.scalar() {
                        Some("yes") => true,
                        Some("no") => false,
                        _ => {
                            out.push((line, Err(format!("scripted trigger {key} is not yes/no"))));
                            continue;
                        }
                    };
                    if !path.insert(key.into()) {
                        out.push((line, Err(format!("recursive scripted trigger {key}"))));
                        continue;
                    }
                    let body = &self.assets.catalog.scripted_triggers[key];
                    self.trigger_children(
                        &body.0,
                        scopes,
                        (invert == expected, indent),
                        path,
                        out,
                        name,
                    );
                    path.remove(key);
                }
                _ => {
                    let text = self.trigger_leaf(trigger, scopes, invert).and_then(|text| {
                        if text.is_empty() {
                            return Ok(text);
                        }
                        self.trigger_prefix(trigger, scopes, invert, indent)
                            .map(|prefix| format!("{prefix}{text}\n"))
                    });
                    out.push((line, text));
                }
            }
        }
    }
    /// Only an OR heading appears at indentation zero, so every line there
    /// takes the fulfilled mark.
    fn trigger_prefix(
        &self,
        trigger: &Trigger,
        scopes: Scopes,
        invert: bool,
        indent: usize,
    ) -> Result<String> {
        let mut text = "   ".repeat(indent); // 142773CD8, 14053FCC0.
        if indent == 0 {
            let value = known(
                self.world
                    .evaluate(&Triggers(vec![trigger.clone()]), scopes),
            )?;
            text.push_str(&localise(
                self.loc,
                if value == invert {
                    "TRIGGER_UNFULLFILLED_PREFIX"
                } else {
                    "TRIGGER_FULLFILLED_PREFIX"
                },
                &[],
            )?);
        }
        Ok(text)
    }
    fn trigger_leaf(&self, trigger: &Trigger, scopes: Scopes, invert: bool) -> Result<String> {
        let scalar = trigger.value.scalar();
        match trigger.name.as_str() {
            "always" | "has_war" => {
                let expected = match scalar {
                    Some("yes") => true,
                    Some("no") => false,
                    _ => return Err("boolean trigger has unsupported value".into()),
                };
                // 1403F0820/140419F50 select by inversion==authored bool.
                let key = match (trigger.name.as_str(), invert == expected) {
                    ("always", true) => "TRIGGER_ALWAYS_FALSE",
                    ("always", false) => "TRIGGER_ALWAYS_TRUE",
                    (_, true) => "TRIGGER_HAS_NOT_WAR",
                    (_, false) => "TRIGGER_HAS_WAR",
                };
                localise(self.loc, key, &[])
            }
            "has_war_with" => {
                // 14041A470: string TAG@142738440 and COUNTRY@142728968.
                let country = self
                    .world
                    .country_of(scalar.ok_or("has_war_with requires country")?, &scopes)?
                    .ok_or("has_war_with country does not resolve")?;
                let tag = self
                    .countries
                    .operands(country)
                    .ok_or("trigger country naming operands absent")?
                    .as_ref()
                    .map_err(Clone::clone)?
                    .tag
                    .clone();
                localise(
                    self.loc,
                    if invert {
                        "TRIGGER_HAS_NOT_WAR_WITH"
                    } else {
                        "TRIGGER_HAS_WAR_WITH"
                    },
                    &[
                        ("TAG", Arg::Text(tag)),
                        ("COUNTRY", Arg::Text(self.countries.name(country)?)),
                    ],
                )
            }
            "custom_override_tooltip" | "custom_trigger_tooltip" => {
                let TriggerValue::Block(children) = &trigger.value else {
                    return Err("custom trigger tooltip requires block".into());
                };
                let field = if invert { "not_tooltip" } else { "tooltip" };
                // 141396000 defaults the negative binding to positive + "_NOT".
                let explicit = children.0.iter().rev().find(|c| c.name == field);
                let fallback = children.0.iter().rev().find(|c| c.name == "tooltip");
                let Some(t) = explicit.or(fallback) else {
                    return Ok(String::new());
                };
                let key = t
                    .value
                    .scalar()
                    .ok_or("bound tooltip requires scalar key")?;
                let key = if invert && explicit.is_none() {
                    format!("{key}_NOT")
                } else {
                    key.into()
                };
                localise(self.loc, &key, &[])
            }
            name => Err(format!("unsupported native trigger text form {name}")),
        }
    }
}
