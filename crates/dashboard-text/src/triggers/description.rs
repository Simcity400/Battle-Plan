
use content_reader_core::{parser::Item, parser::Operator, scope::Keyword, scope::Scope as Parsed};
use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use std::{cell::RefCell, collections::BTreeMap};

use super::{
    Evaluator, Scope, Scopes,
    eval::{Child, key_text},
    event_scopes::event_object,
    load::Unit,
    trigger_boolean,
};
use crate::scripted_text::{EventScope, EventScopes, Link, ScriptedText};

pub type TacticName<'a> = dyn Fn(&[u8]) -> Result<Vec<u8>, String> + 'a;

pub trait Host<'a> {
    fn country_name(&self, country: u32) -> Result<String, String>;
    fn country_tag(&self, country: u32) -> Result<String, String>;
    fn state_name(&self, state: u32) -> Result<String, String>;
    fn scripted_text(&self) -> Result<&ScriptedText<'a>, String>;
    fn expand(
        &self,
        text: &str,
        scopes: Scopes,
        evaluator: &Evaluator<'_, 'a>,
    ) -> Result<String, String>;
    fn project_name(&self, _name: &str, _country: u32) -> Result<String, String> {
        Err("trigger is_special_project_completed description is not reproduced".into())
    }
    fn phases(&self) -> &[Vec<u8>] {
        &[]
    }
    fn tactic_name(&self, _stored: &[u8]) -> Result<Vec<u8>, String> {
        Err("trigger has_tactic description is not reproduced".into())
    }
}

struct ScriptedHost<'r, 'a> {
    text: &'r ScriptedText<'a>,
    phases: &'r [Vec<u8>],
    tactic_name: &'r TacticName<'r>,
}

impl<'a> Host<'a> for ScriptedHost<'_, 'a> {
    fn country_name(&self, country: u32) -> Result<String, String> {
        self.text.country_name(country, false)
    }

    fn country_tag(&self, country: u32) -> Result<String, String> {
        self.text.country_tag(country).map(str::to_owned)
    }

    fn state_name(&self, state: u32) -> Result<String, String> {
        self.text.state_name(state)
    }

    fn scripted_text(&self) -> Result<&ScriptedText<'a>, String> {
        Ok(self.text)
    }

    fn expand(
        &self,
        text: &str,
        scopes: Scopes,
        evaluator: &Evaluator<'_, 'a>,
    ) -> Result<String, String> {
        if let Some(at) = scopes.cursor
            && let Some(graph) = evaluator.event_graph.borrow().as_ref()
        {
            return self
                .text
                .expand_in_session(text, &graph.borrow(), at, evaluator);
        }
        let graph = default_graph(scopes)?;
        self.text.expand_in_session(text, &graph, 0, evaluator)
    }

    fn phases(&self) -> &[Vec<u8>] {
        self.phases
    }

    fn tactic_name(&self, stored: &[u8]) -> Result<Vec<u8>, String> {
        (self.tactic_name)(stored)
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keeps the evaluator, scripted text and tactic metadata borrowed separately at the description entrypoint"
)]
pub fn describe<'a>(
    items: &[Item],
    evaluator: &Evaluator<'_, 'a>,
    text: &ScriptedText<'a>,
    localisation: &Localisation,
    scopes: Scopes,
    phases: &[Vec<u8>],
    tactic_name: &TacticName<'_>,
    refusals: &mut BTreeMap<String, usize>,
) -> Result<String, String> {
    describe_at(items, evaluator, text, localisation, scopes, phases, tactic_name, 0, 1, refusals)
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keeps the evaluator, scripted text and tactic metadata borrowed separately at the description entrypoint"
)]
pub fn describe_at<'a>(
    items: &[Item],
    evaluator: &Evaluator<'_, 'a>,
    text: &ScriptedText<'a>,
    localisation: &Localisation,
    scopes: Scopes,
    phases: &[Vec<u8>],
    tactic_name: &TacticName<'_>,
    indent: usize,
    mode: u8,
    refusals: &mut BTreeMap<String, usize>,
) -> Result<String, String> {
    let host = ScriptedHost {
        text,
        phases,
        tactic_name,
    };
    evaluator.begin_description_in(scopes);
    let result = Formatter {
        evaluator,
        host: &host,
        localisation,
        refusals: RefCell::new(refusals),
    }
    .and(&evaluator.units(items), scopes, false, indent, mode);
    let ended = evaluator.end_description(scopes);
    match (result, ended) {
        (Err(why), _) => Err(why),
        (_, Err(why)) => Err(why.to_string()),
        (Ok(text), Ok(())) => Ok(text),
    }
}

pub fn describe_lines<'a>(
    items: &[Item],
    evaluator: &Evaluator<'_, 'a>,
    host: &dyn Host<'a>,
    localisation: &Localisation,
    scopes: Scopes,
    indent: usize,
    mode: u8,
) -> Vec<(String, Result<String, String>)> {
    evaluator.begin_description_in(scopes);
    let units = evaluator.units(items);
    let mut out = Vec::new();
    for unit in &units {
        let label = unit_key(unit);
        let mut refusals = BTreeMap::new();
        let mut text = String::new();
        let result = {
            let formatter = Formatter {
                evaluator,
                host,
                localisation,
                refusals: RefCell::new(&mut refusals),
            };
            formatter.describe_unit(unit, scopes, false, indent, mode, &mut text)
        };
        match result {
            Ok(()) => {
                if !text.is_empty() {
                    out.push((label.clone(), Ok(text)));
                }
                for (why, count) in refusals {
                    for _ in 0..count {
                        out.push((label.clone(), Err(why.clone())));
                    }
                }
            }
            Err(why) => {
                evaluator.taint_temps(
                    crate::triggers::Failure::from(why.clone())
                        .context(|why| format!("trigger tooltip session is uncertain: {why}")),
                );
                out.push((label, Err(why)));
            }
        }
    }
    if let Err(why) = evaluator.end_description(scopes) {
        out.push(("trigger".into(), Err(why.to_string())));
    }
    out
}

fn unit_key(unit: &Unit<'_, '_>) -> String {
    match unit {
        Unit::One(item, _) => key_text(item),
        Unit::Chain(_) => "if".into(),
        Unit::Unknown(_) => "trigger".into(),
    }
}

fn default_graph(scopes: Scopes) -> Result<EventScopes, String> {
    let mut graph = EventScopes::new(vec![EventScope::of(
        event_object(scopes.this).map_err(|why| why.to_string())?,
    )]);
    for (index, (value, unknown)) in [
        (Some(scopes.root), scopes.unknown.root),
        (scopes.from, scopes.unknown.from),
        (scopes.prev, scopes.unknown.prev),
    ]
    .into_iter()
    .enumerate()
    {
        let link = if unknown {
            Link::Unknown
        } else if let Some(value) = value {
            if value == scopes.this {
                Link::Itself
            } else {
                let to = graph.scopes.len();
                graph.scopes.push(EventScope::of(
                    event_object(value).map_err(|why| why.to_string())?,
                ));
                Link::Scope(to)
            }
        } else {
            Link::Unknown
        };
        match index {
            0 => graph.scopes[0].root = link,
            1 => graph.scopes[0].from = link,
            _ => graph.scopes[0].prev = link,
        }
    }
    graph.variables.target_state = scopes.target_state;
    graph.variables.target_province = scopes.target_province;
    graph.variables.target_leader = scopes.target_leader;
    Ok(graph)
}

struct Formatter<'r, 'e, 'a> {
    evaluator: &'r Evaluator<'e, 'a>,
    host: &'r dyn Host<'a>,
    localisation: &'r Localisation,
    refusals: RefCell<&'r mut BTreeMap<String, usize>>,
}

impl<'a> Formatter<'_, '_, 'a> {
    fn localised(&self, key: &[u8], arguments: &[(&str, Argument<'_>)]) -> Result<String, String> {
        self.localisation
            .localise_with(key, arguments)
            .map_err(|why| why.to_string())
    }

    fn scoped(
        &self,
        key: &[u8],
        arguments: &[(&str, Argument<'_>)],
        scopes: Scopes,
    ) -> Result<String, String> {
        let text = self.localised(key, arguments)?;
        if !text.contains('[') {
            return Ok(text);
        }
        self.host.expand(&text, scopes, self.evaluator)
    }

    fn prefix(
        &self,
        unit: &Unit<'_, 'a>,
        scopes: Scopes,
        inverted: bool,
        indent: usize,
        mode: u8,
    ) -> Result<String, String> {
        let mut text = "   ".repeat(indent);
        if (indent == 0 || mode == 1) && mode != 2 {
            let fulfilled = self.evaluator.unit(unit, scopes).decided()? != inverted;
            text.push_str(&self.localised(
                if fulfilled {
                    b"TRIGGER_FULLFILLED_PREFIX"
                } else {
                    b"TRIGGER_UNFULLFILLED_PREFIX"
                },
                &[],
            )?);
        }
        Ok(text)
    }

    fn and(
        &self,
        units: &[Unit<'_, 'a>],
        scopes: Scopes,
        inverted: bool,
        indent: usize,
        mode: u8,
    ) -> Result<String, String> {
        let mut text = String::new();
        let grouped = indent != 0 && units.len() >= 2;
        if grouped {
            text.push_str(&"   ".repeat(indent));
            text.push_str(&self.localised(
                if inverted {
                    b"TRIGGER_OR_STARTS"
                } else {
                    b"TRIGGER_AND_STARTS"
                },
                &[],
            )?);
            text.push_str(":\n");
        }
        text.push_str(&self.body(
            units,
            scopes,
            inverted,
            indent + usize::from(grouped),
            if grouped { 0 } else { mode },
        )?);
        Ok(text)
    }

    fn body(
        &self,
        units: &[Unit<'_, 'a>],
        scopes: Scopes,
        inverted: bool,
        indent: usize,
        mode: u8,
    ) -> Result<String, String> {
        let mut text = String::new();
        for unit in units {
            let start = text.len();
            let result = self.describe_unit(unit, scopes, inverted, indent, mode, &mut text);
            if let Err(why) = result {
                text.truncate(start);
                self.evaluator.taint_temps(
                    crate::triggers::Failure::from(why.clone())
                        .context(|why| format!("trigger tooltip session is uncertain: {why}")),
                );
                *self.refusals.borrow_mut().entry(why).or_default() += 1;
            }
        }
        Ok(text)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Trigger unit formatting keeps independent inversion, indentation, display mode and output buffer"
    )]
    fn describe_unit(
        &self,
        unit: &Unit<'_, 'a>,
        scopes: Scopes,
        inverted: bool,
        indent: usize,
        mode: u8,
        text: &mut String,
    ) -> Result<(), String> {
        match unit {
            Unit::Unknown(why) => return Err(why.clone().to_string()),
            Unit::Chain(chain) => {
                for branch in chain {
                    if let Some(why) = &branch.refused {
                        return Err(why.clone().to_string());
                    }
                    let selected = self.evaluator.all(&branch.limit, scopes).decided()?;
                    if selected {
                        text.push_str(&self.and(
                            &branch.body,
                            scopes,
                            inverted,
                            indent,
                            if mode == 2 { 2 } else { 0 },
                        )?);
                        break;
                    }
                }
            }
            Unit::One(item, child) => match child {
                Child::Keyless => {
                    return Err("keyless trigger description is not reproduced".into());
                }
                Child::Scripted(template) => {
                    let frame = content_reader_core::trigger_keys::frame(template);
                    if let Some(why) = self.evaluator.enter_frame(frame) {
                        return Err(why);
                    }
                    let result =
                        template
                            .body
                            .as_ref()
                            .map_err(Clone::clone)
                            .and_then(|body| {
                                self.and(
                                    &self.evaluator.units(body),
                                    scopes,
                                    inverted
                                        == trigger_boolean(item)
                                            .map_err(|why| why.to_string())?,
                                    indent,
                                    if mode == 2 { 2 } else { 0 },
                                )
                            });
                    self.evaluator.leave_frame();
                    text.push_str(&result?);
                }
                Child::Scope(parsed) => text.push_str(
                    &self.scope(unit, item, parsed, scopes, inverted, indent, mode)?,
                ),
                Child::Coded(_, name) if name == "hidden_trigger" => {}
                Child::Coded(_, name) if name == "and" => text.push_str(&self.and(
                    &self.evaluator.units(children(item)),
                    scopes,
                    inverted,
                    indent,
                    if mode == 2 { 2 } else { 0 },
                )?),
                Child::Coded(_, name) if name == "not" => text.push_str(&self.body(
                    &self.evaluator.units(children(item)),
                    scopes,
                    !inverted,
                    indent,
                    if mode == 2 { 2 } else { 0 },
                )?),
                Child::Coded(_, name) if name == "or" => {
                    text.push_str(&self.prefix(
                        unit,
                        scopes,
                        inverted,
                        indent,
                        if mode == 2 { 2 } else { 0 },
                    )?);
                    text.push_str(&self.localised(
                        if inverted {
                            b"TRIGGER_AND_STARTS"
                        } else {
                            b"TRIGGER_OR_STARTS"
                        },
                        &[],
                    )?);
                    text.push_str(":\n");
                    text.push_str(&self.body(
                        &self.evaluator.units(children(item)),
                        scopes,
                        inverted,
                        indent + 1,
                        if mode == 2 { 2 } else { 0 },
                    )?);
                }
                _ => {
                    let own = self.own(item, child, scopes, inverted)?;
                    if !own.is_empty() {
                        text.push_str(&self.prefix(unit, scopes, inverted, indent, mode)?);
                        text.push_str(&own);
                        if !own.ends_with('\n') {
                            text.push('\n');
                        }
                    }
                    if !item.keyword_is("custom_trigger_tooltip")
                        && !item.keyword_is("custom_override_tooltip")
                    {
                        text.push_str(&self.body(
                            &self.evaluator.units(children(item)),
                            scopes,
                            inverted,
                            indent,
                            if mode == 2 { 2 } else { 0 },
                        )?);
                    }
                }
            },
        }
        Ok(())
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Scope tooltip inputs keep independent inversion, indentation and display mode"
    )]
    fn scope(
        &self,
        unit: &Unit<'_, 'a>,
        item: &Item,
        parsed: &Parsed,
        scopes: Scopes,
        inverted: bool,
        indent: usize,
        mode: u8,
    ) -> Result<String, String> {
        let entered = self
            .evaluator
            .enter_scope(parsed, scopes)
            .map_err(|why| why.to_string())?
            .filter(|entered| entered.this != Scope::State(super::NULL_STATE))
            .ok_or_else(|| format!("trigger scope {} resolves no target", key_text(item)))?;
        let heading = match parsed {
            Parsed::Keyword(
                keyword @ (Keyword::Owner
                | Keyword::Controller
                | Keyword::CapitalScope
                | Keyword::Overlord
                | Keyword::FactionLeader
                | Keyword::Occupied),
            ) => self.localised(
                match keyword {
                    Keyword::Owner => b"TRIGGER_OWNER",
                    Keyword::Controller => b"TRIGGER_CONTROLLER",
                    Keyword::CapitalScope => b"TRIGGER_CAPITAL",
                    Keyword::Overlord => b"TRIGGER_OVERLORD",
                    Keyword::FactionLeader => b"TRIGGER_FACTION_LEADER",
                    Keyword::Occupied => b"TRIGGER_OCCUPIED",
                    _ => unreachable!(),
                },
                &[],
            )?,
            _ if entered.this == Scope::Combat => String::new(),
            _ => {
                let name = match entered.this {
                    Scope::Country(country) | Scope::CountryLeader { country, .. } => {
                        self.host.country_name(country)?
                    }
                    Scope::State(state) => self.host.state_name(state)?,
                    _ => {
                        return Err(format!(
                            "trigger scope {} name is not reproduced",
                            key_text(item)
                        ));
                    }
                };
                format!("\u{11}Y{name}\u{11}!")
            }
        };
        let units = self.evaluator.units(children(item));
        let mut text = self.prefix(
            unit,
            scopes,
            inverted,
            indent,
            if mode == 2 { 2 } else { 0 },
        )?;
        let inner = if units.len() < 2 {
            self.body(&units, entered, inverted, 0, 2)?
        } else {
            self.body(
                &units,
                entered,
                inverted,
                indent + 1,
                if mode == 2 { 2 } else { 0 },
            )?
        };
        if inner.is_empty() {
            return Ok(String::new());
        }
        text.push_str(&heading);
        if units.len() < 2 {
            text.push_str(": ");
        } else {
            text.push_str(":\n");
        }
        text.push_str(&inner);
        Ok(text)
    }

    fn own(
        &self,
        item: &Item,
        child: &Child<'_>,
        scopes: Scopes,
        inverted: bool,
    ) -> Result<String, String> {
        let Child::Coded(_, name) = child else {
            return Err(format!(
                "trigger {} description is not reproduced",
                key_text(item)
            ));
        };
        let operand = content_reader_core::parser::c_string(item.value.scalar().unwrap_or_default());
        let key = match name.as_str() {
            "always" => Some(
                if trigger_boolean(item).map_err(|why| why.to_string())? != inverted {
                    b"TRIGGER_ALWAYS_TRUE".as_slice()
                } else {
                    b"TRIGGER_ALWAYS_FALSE"
                },
            ),
            "has_war" => Some(
                if trigger_boolean(item).map_err(|why| why.to_string())? != inverted {
                    b"TRIGGER_HAS_WAR".as_slice()
                } else {
                    b"TRIGGER_HAS_NOT_WAR"
                },
            ),
            "is_attacker" => Some(
                if trigger_boolean(item).map_err(|why| why.to_string())? != inverted {
                    b"IS_ATTACKER".as_slice()
                } else {
                    b"IS_NOT_ATTACKER"
                },
            ),
            "frontage_full" => Some(
                if trigger_boolean(item).map_err(|why| why.to_string())? != inverted {
                    b"TRIGGER_IS_FRONTAGE_FULL".as_slice()
                } else {
                    b"TRIGGER_IS_NOT_FRONTAGE_FULL"
                },
            ),
            "has_reserves" => Some(
                if trigger_boolean(item).map_err(|why| why.to_string())? != inverted {
                    b"TRIGGER_HAS_RESERVES".as_slice()
                } else {
                    b"TRIGGER_DOES_NOT_HAVE_RESERVES"
                },
            ),
            _ => None,
        };
        if let Some(key) = key {
            return self.localised(key, &[]);
        }
        match name.as_str() {
            "has_war_with" => {
                if item.value.scalar_str().is_none() {
                    return Err("has_war_with requires country".into());
                }
                let country = self
                    .evaluator
                    .country_operand(item, scopes)
                    .map_err(|why| why.to_string())?
                    .ok_or("has_war_with country does not resolve")?;
                let tag = self.host.country_tag(country)?;
                let name = self.host.country_name(country)?;
                self.localised(
                    if inverted {
                        b"TRIGGER_HAS_NOT_WAR_WITH"
                    } else {
                        b"TRIGGER_HAS_WAR_WITH"
                    },
                    &[
                        ("TAG", Argument::Text(&tag)),
                        ("COUNTRY", Argument::Text(&name)),
                    ],
                )
            }
            "is_special_project_completed" => {
                let name = item
                    .value
                    .scalar_str()
                    .and_then(|value| value.strip_prefix("sp:"))
                    .ok_or("special project tooltip requires an sp: project operand")?;
                let Scope::Country(country) = scopes.this else {
                    return Err("special project tooltip THIS is not a country".into());
                };
                let project = self.host.project_name(name, country)?;
                self.localised(
                    if inverted {
                        b"TRIGGER_IS_NOT_SPECIAL_PROJECT_COMPLETED"
                    } else {
                        b"TRIGGER_IS_SPECIAL_PROJECT_COMPLETED"
                    },
                    &[("PROJECT", Argument::Text(&project))],
                )
            }
            "phase" => {
                let id = content_reader_core::fixed5::decimal_i32(operand);
                let key = if id < 0 {
                    b"PHASE_DEFAULT".as_slice()
                } else {
                    self.host.phases().get(id as usize).ok_or_else(|| {
                        format!("phase ID {id} is outside the recorded-content tactic phases")
                    })?
                };
                let phase = self.localised(key, &[])?;
                self.localised(
                    if inverted {
                        b"IS_NOT_IN_PHASE"
                    } else {
                        b"IS_IN_PHASE"
                    },
                    &[("PHASE", Argument::Text(&phase))],
                )
            }
            "skill_advantage" | "hardness" | "has_artillery_ratio" => {
                let fixed = name != "skill_advantage";
                let value = self
                    .evaluator
                    .required(operand, item.quoted(), fixed, scopes)
                    .map_err(|why| why.to_string())?;
                let greater = item.operator
                    == if inverted {
                        Operator::Less
                    } else {
                        Operator::Greater
                    };
                let (key, argument) = match name.as_str() {
                    "skill_advantage" => (
                        if greater {
                            b"TRIGGER_SKILL_ADVANTAGE_GREATER_THEN".as_slice()
                        } else {
                            b"TRIGGER_SKILL_ADVANTAGE_LESS_THEN"
                        },
                        ("VALUE", Argument::Int32((value / 100_000) as i32)),
                    ),
                    "hardness" => (
                        if greater {
                            b"TRIGGER_HARDNESS_GREATER_THAN".as_slice()
                        } else {
                            b"TRIGGER_HARDNESS_LESS_THAN"
                        },
                        ("VALUE", Argument::Fixed5(value)),
                    ),
                    _ => (
                        if greater {
                            b"TRIGGER_MORE_THAN_ARTILLERY_RATIO".as_slice()
                        } else {
                            b"TRIGGER_LESS_THAN_ARTILLERY_RATIO"
                        },
                        ("NUM", Argument::Fixed5(value)),
                    ),
                };
                self.localised(key, &[argument])
            }
            "skill" => {
                let value = self
                    .evaluator
                    .required(operand, item.quoted(), false, scopes)
                    .map_err(|why| why.to_string())?;
                let value = ((value / 100_000) as i32).to_string();
                let comparison = match (item.operator, inverted) {
                    (Operator::Greater, false) => b"TRIGGER_MORE_THAN".as_slice(),
                    (Operator::Greater, true) => b"TRIGGER_AT_MOST",
                    (Operator::Less, false) => b"TRIGGER_LESS_THAN",
                    (Operator::Less, true) => b"TRIGGER_AT_LEAST",
                    (Operator::Equals, _) => b"",
                };
                let comparison = self.localised(comparison, &[])?;
                let kind = self.localised(b"TRIGGER_COMPARE_LEVEL", &[])?;
                self.localised(
                    b"TRIGGER_TYPE_COMPARISON_VALUE",
                    &[
                        ("COMPARISON", Argument::Text(&comparison)),
                        ("TYPE", Argument::Text(&kind)),
                        ("VALUE", Argument::Text(&value)),
                    ],
                )
            }
            "has_trait" => {
                let (unit, country) = self
                    .evaluator
                    .world
                    .character_traits
                    .map_err(str::to_owned)?;
                let name =
                    std::str::from_utf8(operand).map_err(|_| "trait operand is not UTF-8")?;
                let label = if let Some(spelling) = unit
                    .spellings()
                    .find(|known| known.eq_ignore_ascii_case(name))
                {
                    let text = self.host.scripted_text()?;
                    let tag = text.country_tag(text.viewing_country())?;
                    let prefixed = format!("{tag}_{spelling}");
                    self.localised(
                        if self.localisation.get(&prefixed).is_some() {
                            prefixed.as_bytes()
                        } else {
                            spelling.as_bytes()
                        },
                        &[],
                    )?
                } else if let Some(found) = country
                    .get(name)
                    .map_err(|why| format!("trait {name}: {why:?}"))?
                {
                    self.localised(found.token.as_bytes(), &[])?
                } else {
                    return Err(format!(
                        "trait {name} scientist fallback description is not reproduced"
                    ));
                };
                self.scoped(
                    if inverted {
                        b"TRIGGER_HAS_NOT_TRAIT"
                    } else {
                        b"TRIGGER_HAS_TRAIT"
                    },
                    &[("NAME", Argument::Text(&label))],
                    scopes,
                )
            }
            "is_deployed" => self
                .scoped(
                    if trigger_boolean(item).map_err(|why| format!("trigger is_deployed: {why}"))?
                        != inverted
                    {
                        b"TRIGGER_IS_DEPLOYED"
                    } else {
                        b"TRIGGER_IS_NOT_DEPLOYED"
                    },
                    &[],
                    scopes,
                )
                .map_err(|why| format!("trigger is_deployed: {why}")),
            "is_fighting_in_strategic_region" => {
                let region = content_reader_core::crt::scan_i32(operand)
                    .stored()
                    .ok_or("trigger is_fighting_in_strategic_region: the operand does not assign a region ID; the retained value is not reproduced")?
                    .to_string();
                self.localised(
                    if inverted {
                        b"TRIGGER_NOT_FIGHTING_IN_STRATEGIC_REGION"
                    } else {
                        b"TRIGGER_FIGHTING_IN_STRATEGIC_REGION"
                    },
                    &[("REGION", Argument::Text(&region))],
                )
            }
            "is_fighting_in_terrain" => {
                let terrain = self.host.scripted_text()?.content().environment()?.terrain()?;
                let index = terrain.category(operand).ok_or_else(|| {
                    format!(
                        "terrain {} is absent from recorded content",
                        String::from_utf8_lossy(operand)
                    )
                })?;
                let label = self.localised(&terrain.categories[index].name, &[])?;
                self.localised(
                    if inverted {
                        b"TRIGGER_FIGHTING_NOT_ON_TERRAIN"
                    } else {
                        b"TRIGGER_FIGHTING_ON_TERRAIN"
                    },
                    &[("TYPE", Argument::Text(&label))],
                )
            }
            "has_unit_type" => {
                let label = self.localised(operand, &[])?;
                self.scoped(
                    if inverted {
                        b"TRIGGER_COMBATANT_DO_NOT_HAVE_UNIT_TYPE"
                    } else {
                        b"TRIGGER_COMBATANT_HAVE_UNIT_TYPE"
                    },
                    &[("TYPE", Argument::Text(&label))],
                    scopes,
                )
            }
            "has_combat_modifier" => {
                if !operand.eq_ignore_ascii_case(b"river_crossing") {
                    return Err(format!(
                        "combat modifier {} display token is not reproduced",
                        String::from_utf8_lossy(operand)
                    ));
                }
                let label = self.localised(b"BM_RIVER_PENALTY", &[])?;
                self.localised(
                    if inverted {
                        b"TRIGGER_HAS_NOT_COMBAT_MODIFIER"
                    } else {
                        b"TRIGGER_HAS_COMBAT_MODIFIER"
                    },
                    &[("MODIFIER", Argument::Text(&label))],
                )
            }
            "has_tactic" => {
                let token = self
                    .evaluator
                    .world
                    .token_of(operand)
                    .map_err(|why| why.to_string())?
                    .ok_or("tactic operand is absent from lexical tokens")?;
                let stored = self
                    .evaluator
                    .world
                    .token(token)
                    .map_err(|why| why.to_string())?
                    .ok_or("tactic token has no lexical tokens literal")?;
                let resolved = self.host.tactic_name(stored)?;
                let label = self.localised(&resolved, &[])?;
                self.localised(
                    if inverted {
                        b"HAS_NOT_TACTIC_TRIGGER"
                    } else {
                        b"HAS_TACTIC_TRIGGER"
                    },
                    &[("NAME", Argument::Text(&label))],
                )
            }
            "tag" => {
                if matches!(
                    scopes.this,
                    Scope::Leader(_) | Scope::CountryLeader { .. } | Scope::Character(_)
                ) {
                    return Err("tag unit-leader adjective description is not reproduced".into());
                }
                let country = self
                    .evaluator
                    .country_operand(item, scopes)
                    .map_err(|why| why.to_string())?
                    .ok_or("tag description resolves null country")?;
                let label = self.host.scripted_text()?.country_marked_name(country)?;
                self.scoped(
                    if inverted {
                        b"TRIGGER_COUNTRY_IS_NOT"
                    } else {
                        b"TRIGGER_COUNTRY_IS"
                    },
                    &[("NAME", Argument::Text(&label))],
                    scopes,
                )
            }
            "has_volunteers" => {
                let country = self
                    .evaluator
                    .country_operand(item, scopes)
                    .map_err(|why| format!("trigger has_volunteers: {why}"))?
                    .ok_or("trigger has_volunteers: the operand resolves no country; the description is not reproduced")?;
                let label = self.host.scripted_text()?.country_marked_name(country)?;
                self.scoped(
                    if inverted {
                        b"TRIGGER_HAS_NO_VOLUNTEERS_IN_TARGET"
                    } else {
                        b"TRIGGER_HAS_VOLUNTEERS_IN_TARGET"
                    },
                    &[("COUNTRY", Argument::Text(&label))],
                    scopes,
                )
            }
            "custom_trigger_tooltip" | "custom_override_tooltip" => {
                let Some(fields) = item.value.items() else {
                    return Err("custom trigger tooltip requires block".into());
                };
                let positive = field(fields, "tooltip");
                let negative = field(fields, "not_tooltip");
                let selected = if inverted {
                    negative.or(positive)
                } else {
                    positive
                };
                let Some(selected) = selected else {
                    return Ok(String::new());
                };
                let key = selected
                    .value
                    .scalar()
                    .ok_or("custom trigger bound-localisation arguments are not reproduced")?;
                let negative_default;
                let key = if inverted && negative.is_none() {
                    negative_default = [key, b"_NOT"].concat();
                    &negative_default
                } else {
                    key
                };
                self.scoped(key, &[], scopes)
            }
            _ => Err(format!("trigger {name} description is not reproduced")),
        }
    }
}

fn children(item: &Item) -> &[Item] {
    item.value.items().unwrap_or_default()
}
fn field<'a>(items: &'a [Item], name: &str) -> Option<&'a Item> {
    items.iter().rev().find(|item| item.keyword_is(name))
}

#[cfg(test)]
mod tests;
