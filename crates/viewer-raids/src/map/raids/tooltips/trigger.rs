//! Raid diagnostic mode 1: condition traversal, native leaf keys and evaluator joins.
//! Kept until the unified trigger evaluator after #52; localisation, arguments
//! and scoped custom-tooltip expansion use the shared text code.
use super::super::rules::Lines;
use super::*;
use installed_content_core::{parser::Item, scripted_triggers::Lookup};
use std::collections::BTreeSet;
use viewer_foundation::tooltip_text::ensure_newlines;

impl Context<'_, '_> {
    pub(super) fn trigger_text(&self, triggers: &[Item], scopes: Scopes, name: &str) -> Lines {
        let mut out = Vec::new();
        self.trigger_children(
            triggers,
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
        triggers: &[Item],
        scopes: Scopes,
        mode: (bool, usize),
        path: &mut BTreeSet<String>,
        out: &mut Lines,
        name: &str,
    ) {
        let (invert, indent) = mode;
        for trigger in triggers {
            let key = key_text(trigger);
            let line = format!("{name}: {key}");
            let children = trigger.value.items();
            match key.as_str() {
                "hidden_trigger" => {} // Hidden triggers emit no text.
                "AND" | "and" | "OR" | "or" | "NOT" | "not" => {
                    let Some(children) = children else {
                        out.push((line, Err("logical trigger requires block".into())));
                        continue;
                    };
                    let not = key.eq_ignore_ascii_case("NOT");
                    let or = key.eq_ignore_ascii_case("OR");
                    let heading = or || (!not && indent > 0 && children.len() >= 2);
                    if heading {
                        // OR always has a heading; AND has one only with positive indentation
                        // and multiple children.
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
                    // NOT toggles inversion without adding indentation.
                    self.trigger_children(
                        children,
                        scopes,
                        (invert ^ not, indent + usize::from(heading)),
                        path,
                        out,
                        name,
                    );
                }
                key if matches!(
                    self.assets
                        .trigger_content
                        .triggers
                        .scripted
                        .lookup(key.as_bytes()),
                    Lookup::Template(_)
                ) =>
                {
                    // Choose template slot 12 when inversion equals the expected value.
                    let expected = match trigger
                        .value
                        .scalar()
                        .and_then(installed_content_core::parser::read_bool)
                    {
                        Some(expected) => expected,
                        None => {
                            out.push((line, Err(format!("scripted trigger {key} is not yes/no"))));
                            continue;
                        }
                    };
                    let Lookup::Template(template) = self
                        .assets
                        .trigger_content
                        .triggers
                        .scripted
                        .lookup(key.as_bytes())
                    else {
                        continue;
                    };
                    let body = match &template.body {
                        Ok(body) => body,
                        Err(why) => {
                            out.push((line, Err(format!("scripted trigger {key}: {why}"))));
                            continue;
                        }
                    };
                    if !path.insert(key.to_owned()) {
                        out.push((line, Err(format!("recursive scripted trigger {key}"))));
                        continue;
                    }
                    self.trigger_children(
                        body,
                        scopes,
                        (invert == expected, indent),
                        path,
                        out,
                        name,
                    );
                    path.remove(key);
                }
                _ => {
                    let text = self
                        .trigger_leaf(trigger, scopes, invert)
                        .and_then(|mut text| {
                            if text.is_empty() {
                                return Ok(text);
                            }
                            // Append only a missing trailing newline, including when custom text
                            // contains an effect description.
                            ensure_newlines(&mut text, 1);
                            self.trigger_prefix(trigger, scopes, invert, indent)
                                .map(|prefix| format!("{prefix}{text}"))
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
        trigger: &Item,
        scopes: Scopes,
        invert: bool,
        indent: usize,
    ) -> Result<String> {
        let mut text = "   ".repeat(indent);
        if indent == 0 {
            let value = known(self.world.evaluate(std::slice::from_ref(trigger), scopes))?;
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
    fn trigger_leaf(&self, trigger: &Item, scopes: Scopes, invert: bool) -> Result<String> {
        let scalar = trigger.value.scalar_str();
        let key = key_text(trigger);
        match key.as_str() {
            "always" | "has_war" => {
                let expected = trigger
                    .value
                    .scalar()
                    .and_then(installed_content_core::parser::read_bool)
                    .ok_or("boolean trigger has unsupported value")?;
                // Choose by whether inversion equals the authored boolean.
                let key = match (key.as_str(), invert == expected) {
                    ("always", true) => "TRIGGER_ALWAYS_FALSE",
                    ("always", false) => "TRIGGER_ALWAYS_TRUE",
                    (_, true) => "TRIGGER_HAS_NOT_WAR",
                    (_, false) => "TRIGGER_HAS_WAR",
                };
                localise(self.loc, key, &[])
            }
            "has_war_with" => {
                // Bind TAG and COUNTRY as strings.
                scalar.ok_or("has_war_with requires country")?;
                let country = self
                    .world
                    .country_of(trigger, scopes)?
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
                        ("TAG", Arg::Text(&tag)),
                        ("COUNTRY", Arg::Text(&self.countries.name(country)?)),
                    ],
                )
            }
            "is_special_project_completed" => {
                let name = scalar
                    .and_then(|value| value.strip_prefix("sp:"))
                    .ok_or("special project tooltip requires an sp: project operand")?;
                let Scope::Country(country) = scopes.this else {
                    return Err("special project tooltip THIS is not a country".into());
                };
                // Read the project name record and bind PROJECT as a string; the
                // negative key follows inversion, not completion.
                localise(
                    self.loc,
                    if invert {
                        "TRIGGER_IS_NOT_SPECIAL_PROJECT_COMPLETED"
                    } else {
                        "TRIGGER_IS_SPECIAL_PROJECT_COMPLETED"
                    },
                    &[("PROJECT", Arg::Text(&self.project_name(name, country)?))],
                )
            }
            "custom_override_tooltip" | "custom_trigger_tooltip" => {
                let Some(children) = trigger.value.items() else {
                    return Err("custom trigger tooltip requires block".into());
                };
                let field = if invert { "not_tooltip" } else { "tooltip" };
                // Default the negative binding to the positive key with suffix _NOT.
                let explicit = children.iter().rev().find(|c| key_text(c) == field);
                let fallback = children.iter().rev().find(|c| key_text(c) == "tooltip");
                let Some(t) = explicit.or(fallback) else {
                    return Ok(String::new());
                };
                let key = t
                    .value
                    .scalar_str()
                    .ok_or("bound tooltip requires scalar key")?;
                let key = if invert && explicit.is_none() {
                    format!("{key}_NOT")
                } else {
                    key.into()
                };
                // Localize the scope name using the event scope.
                self.scoped_text(&key, scopes)
            }
            name => Err(format!("unsupported native trigger text form {name}")),
        }
    }

    fn project_name(&self, name: &str, country: u32) -> Result<String> {
        // Look up identity without a started/completed gate.
        let token = self
            .world
            .token_of(name.as_bytes())
            .map_err(|why| why.to_string())?;
        let rows = self
            .world
            .frame()
            .complete::<tables::Projects>()
            .map_err(|e| e.to_string())?;
        if token.is_none()
            || !rows.iter().any(|row| {
                Some(row.definition_id) == token && self.world.slot(row.owner) == Some(country)
            })
        {
            // An unresolved operand uses its stored scope string; an sp: operand
            // leaves this constructor-default string empty.
            return Ok(String::new());
        }
        self.project_tooltip_name(name, country)
    }
}

/// A trigger's key as authored.
fn key_text(item: &Item) -> String {
    String::from_utf8_lossy(item.key.as_deref().unwrap_or_default()).into_owned()
}
