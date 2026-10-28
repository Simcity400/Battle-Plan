//! Combat tactic requirements: describe the embedded conjunction
//! with inversion=false, indent=0, mode=1.
//! The caller supplies the heading and two-level line indentation.

use installed_content_core::{parser::Item, parser::Operator, scope::Keyword, scope::Scope as Parsed};
use installed_content_localisation::{localisation::Argument, localisation::Localisation};
use std::{cell::RefCell, collections::BTreeMap};

use super::{
    Evaluator, Scope, Scopes,
    eval::{Child, key_text},
    event_scopes::event_object,
    load::Unit,
    trigger_boolean,
};
use crate::scripted_text::{EventScope, EventScopes, Link, ScriptedText};

/// Resolves a stored tactic operand through the caller's native tactic database.
pub type TacticName<'a> = dyn Fn(&[u8]) -> Result<Vec<u8>, String> + 'a;

/// Describes the loaded requirement children using the caller's World and
/// Combat. Unknown native descriptions are refused by name; no predicate is
/// implemented here. `phases` is the matching tactic database's phase order,
/// since its native reader has already replaced phase operands with IDs.
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
    evaluator.begin_description_in(scopes);
    let result = Formatter {
        evaluator,
        text,
        localisation,
        phases,
        tactic_name,
        refusals: RefCell::new(refusals),
    }
    .and(&evaluator.units(items), scopes, false, 0, 1);
    let ended = evaluator.end_description(scopes);
    match (result, ended) {
        (Err(why), _) => Err(why),
        (_, Err(why)) => Err(why.to_string()),
        (Ok(text), Ok(())) => Ok(text),
    }
}

struct Formatter<'r, 'e, 'a> {
    evaluator: &'r Evaluator<'e, 'a>,
    text: &'r ScriptedText<'a>,
    localisation: &'r Localisation,
    phases: &'r [Vec<u8>],
    tactic_name: &'r TacticName<'r>,
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
        if let Some(at) = scopes.cursor
            && let Some(graph) = self.evaluator.event_graph.borrow().as_ref()
        {
            return self
                .text
                .expand_in_session(&text, &graph.borrow(), at, self.evaluator);
        }
        let graph = self.graph(scopes)?;
        self.text
            .expand_in_session(&text, &graph, 0, self.evaluator)
    }

    // The bare combat event has no carried country;
    // a promoted country clears Combat, retaining linked scopes.
    fn graph(&self, scopes: Scopes) -> Result<EventScopes, String> {
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

    // A conjunction flattens at indent0 or fewer than two
    // loaded children; otherwise its header precedes child tooltips.
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

    // Obtain own text before checking,
    // skip checking when own text is empty, then visit descendants in mode0.
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
            let result = (|| -> Result<(), String> {
                match unit {
                    Unit::Unknown(why) => return Err(why.clone().to_string()),
                    Unit::Chain(chain) => {
                        // Check each limit and only
                        // describe the first selected branch, as a conjunction.
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
                    Unit::One(item, child) => {
                        match child {
                            Child::Keyless => {
                                return Err("keyless trigger description is not reproduced".into());
                            }
                            Child::Scripted(template) => {
                                // A scripted trigger's own text is
                                // empty, the template receives inversion == operand.
                                let frame = installed_content_core::trigger_keys::frame(template);
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
                                // A disjunction includes its own check.
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
                                    text.push_str(
                                        &self.prefix(unit, scopes, inverted, indent, mode)?,
                                    );
                                    text.push_str(&own);
                                    if !own.ends_with('\n') {
                                        text.push('\n');
                                    }
                                }
                                // Custom tooltip classes suppress descendant text
                                // while checking still sees it.
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
                        }
                    }
                }
                Ok(())
            })();
            if let Err(why) = result {
                text.truncate(start);
                // Own text can read scoped operands before Check; an unknown
                // limit can select a body whose TLS effects we cannot follow.
                // Preserve later pure siblings, refuse dependent temp reads.
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
        // Enter the context scope and format its target heading.
        let entered = self
            .evaluator
            .enter_scope(parsed, scopes)
            .map_err(|why| why.to_string())?
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
            _ if entered.this == Scope::Combat => String::new(), // The fallback carries neither state nor country.
            _ => {
                let name = match entered.this {
                    Scope::Country(country) | Scope::CountryLeader { country, .. } => {
                        self.text.country_name(country, false)?
                    }
                    Scope::State(state) => self.text.state_name(state)?,
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
        text.push_str(&heading);
        if units.len() < 2 {
            text.push_str(": ");
            text.push_str(&self.body(&units, entered, inverted, 0, 2)?);
        } else {
            text.push_str(":\n");
            text.push_str(&self.body(
                &units,
                entered,
                inverted,
                indent + 1,
                if mode == 2 { 2 } else { 0 },
            )?);
        }
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
        let operand = installed_content_core::parser::c_string(item.value.scalar().unwrap_or_default());
        let key = match name.as_str() {
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
            "phase" => {
                // Use the stored ID, not authored spelling.
                let id = installed_content_core::fixed5::decimal_i32(operand);
                let key = if id < 0 {
                    b"PHASE_DEFAULT".as_slice()
                } else {
                    self.phases.get(id as usize).ok_or_else(|| {
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
                // These own methods take the greater wording only for >
                // normally or < under inversion; equality uses the other arm.
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
                // Format the comparison with its evaluated prefix in mode1.
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
                // Try unit traits before country traits.
                let (unit, country) = self
                    .evaluator
                    .world
                    .character_traits
                    .map_err(str::to_owned)?;
                let name =
                    std::str::from_utf8(operand).map_err(|_| "trait operand is not UTF-8")?;
                // Country localisation uses the GUI's local
                // country, not the side currently being described.
                let label = if let Some(spelling) = unit
                    .spellings()
                    .find(|known| known.eq_ignore_ascii_case(name))
                {
                    let tag = self.text.country_tag(self.text.viewing_country())?;
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
                let region = installed_content_core::crt::scan_i32(operand)
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
                // Localise the terrain's stored identifier.
                let terrain = self.text.content().environment()?.terrain()?;
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
                // Localise the stored string before scope expansion.
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
                // Resolve the combat modifier's display token.
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
                // Tactic text uses its localised identifier.
                let token = self
                    .evaluator
                    .world
                    .token_of(operand)
                    .map_err(|why| why.to_string())?
                    .ok_or("tactic operand is absent from LexicalTokens")?;
                let stored = self
                    .evaluator
                    .world
                    .token(token)
                    .map_err(|why| why.to_string())?
                    .ok_or("tactic token has no LexicalTokens literal")?;
                let resolved = (self.tactic_name)(stored)?; // Resolve a named tactic or the null tactic.
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
                // Combat carries country, no unit leader.
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
                let label = self.text.country_marked_name(country)?;
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
                // Same country getter and marked name as tag, without its unit-leader branch.
                let country = self
                    .evaluator
                    .country_operand(item, scopes)
                    .map_err(|why| format!("trigger has_volunteers: {why}"))?
                    .ok_or("trigger has_volunteers: the operand resolves no country; the description is not reproduced")?;
                let label = self.text.country_marked_name(country)?;
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
                // Render own text with empty descendants.
                let fields = children(item);
                let positive = field(fields, "tooltip");
                let negative = field(fields, "not_tooltip");
                let selected = if inverted {
                    negative.or(positive)
                } else {
                    positive
                };
                let Some(selected) = selected else {
                    return Ok(String::new());
                }; // Empty bound localisation.
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
