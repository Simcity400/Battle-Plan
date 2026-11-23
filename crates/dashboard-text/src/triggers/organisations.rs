use std::{cell::RefCell, rc::Rc};

use crate::triggers::{Failure, refusal::refused};

use content_reader_military::mio_traits::{Catalog, TraitName};
use content_reader_parser::parser::{Item, Operator, read_bool, read_bool_field};

use super::{
    Eval, Scopes,
    eval::Evaluator,
    flags::FlagTest,
    operands::{compare, scope_selector},
};
use crate::{scripted_text::EventScopes, state_resources::ONE};

fn token_text<'i>(item: &'i Item, name: &str) -> Result<&'i [u8], Failure> {
    if item.quoted() {
        return Err(refused(format!(
            "{name}'s quoted operand loads as a generic string token, whose identity is not reproduced"
        )));
    }
    item.value
        .scalar()
        .map(content_reader_parser::parser::c_string)
        .ok_or_else(|| format!("{name} needs a scalar operand").into())
}

fn trait_operand<'i>(
    item: &'i Item,
    name: &str,
    toggles: &[&str],
) -> Result<Option<&'i [u8]>, Failure> {
    let Some(block) = item.value.items() else {
        return token_text(item, name).map(Some);
    };
    let mut wanted = None;
    for field in block {
        if field.keyword_is("trait") {
            if wanted.is_some() {
                return Err(refused(format!(
                    "{name} names `trait` more than once, and which one the reader keeps is not reproduced"
                )));
            }
            wanted = Some(token_text(field, name)?);
        } else if !field.keyword_is("show_modifiers")
            && !toggles.iter().any(|toggle| field.keyword_is(toggle))
        {
            return Err(refused(format!(
                "{name} field {} takes the reader's error path, whose recovery is not reproduced",
                String::from_utf8_lossy(field.key.as_deref().unwrap_or_default())
            )));
        }
    }
    Ok(wanted)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrganisationCondition {
    Available,
    Visible,
}

impl OrganisationCondition {
    fn name(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Visible => "visible",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Undecided {
    Unknown(String),
    Diverged(String),
}

impl std::fmt::Display for Undecided {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(why) => f.write_str(why),
            Self::Diverged(why) => write!(f, "the game never returns: {why}"),
        }
    }
}

impl From<String> for Undecided {
    fn from(why: String) -> Self {
        Self::Unknown(why)
    }
}

impl From<&str> for Undecided {
    fn from(why: &str) -> Self {
        Self::Unknown(why.to_owned())
    }
}

impl From<Undecided> for String {
    fn from(why: Undecided) -> Self {
        why.to_string()
    }
}

fn expected(item: &Item) -> bool {
    if scope_selector(item).is_some() {
        return false;
    }
    item.value.scalar().and_then(read_bool).unwrap_or(false)
}

impl Evaluator<'_, '_> {
    pub(super) fn organisation_trigger(
        &self,
        name: &str,
        item: &Item,
        reference: u64,
        scopes: Scopes,
    ) -> Result<Option<Eval>, Failure> {
        let world = self.world();
        Ok(Some(match name {
            "has_mio_size" => {
                if item.kind == content_reader_parser::parser::Kind::Open {
                    return Err(refused(
                        "has_mio_size takes a block's opening brace as its operand, and how the reader recovers is not reproduced",
                    ));
                }
                let actual = world.industrial_organisation_size(reference)?;
                let wanted = if scope_selector(item).is_some() {
                    0
                } else {
                    let bytes = item.value.scalar().unwrap_or_default();
                    self.required(bytes, item.quoted(), false, scopes)?
                };
                Eval::from_bool(compare(
                    i64::from(actual) * ONE,
                    wanted,
                    item.operator,
                    false,
                ))
            }
            "has_mio_number_of_completed_traits" => {
                self.completed_trait_count(item, reference, scopes)?
            }
            "has_mio_policy" => {
                let token = world.token_of(token_text(item, name)?)?;
                let (allowed, _) = world.organisation_policies(reference)?;
                Eval::from_bool(token.is_some_and(|token| allowed.contains(&token)))
            }
            "has_mio_policy_active" => {
                let token = world.token_of(token_text(item, name)?)?;
                let (_, attached) = world.organisation_policies(reference)?;
                Eval::from_bool(token == Some(attached))
            }
            "is_military_industrial_organization" => {
                let token = world.token_of(token_text(item, name)?)?;
                let definition = world.industrial_organisation(reference)?.token;
                Eval::from_bool(token == Some(definition))
            }
            "is_mio_assigned_to_task" => {
                let assigned = !world.organisation_tasks(reference)?.is_empty();
                Eval::from_bool(assigned == expected(item))
            }
            "has_mio_trait" => {
                let wanted = trait_operand(item, name, &[])?;
                let (template, catalog) = self.organisation_definition(reference)?;
                Eval::from_bool(match wanted {
                    Some(wanted) => ordinary_trait(catalog, &template, wanted)?.is_some(),
                    None => false,
                })
            }
            "is_mio_trait_completed" => {
                let Some(wanted) = trait_operand(item, name, &[])? else {
                    return Ok(Some(Eval::False));
                };
                let (template, catalog) = self.organisation_definition(reference)?;
                if ordinary_trait(catalog, &template, wanted)?.is_none() {
                    return Ok(Some(Eval::False));
                }
                Eval::from_bool(self.unlocked(reference, wanted)?)
            }
            "is_mio_trait_available" => self.trait_available(item, reference)?,
            "has_mio_equipment_type" => {
                let wanted = content_reader_parser::parser::c_string(
                    item.value
                        .scalar()
                        .ok_or("has_mio_equipment_type needs a scalar operand")?,
                );
                let (template, catalog) = self.organisation_definition(reference)?;
                let tree = organisation_tree(catalog, &template)?;
                let groups = world.equipment_groups.map_err(Failure::from)?;
                let members = groups.expand_inclusive(tree.equipment_types)?;
                Eval::from_bool(
                    members
                        .iter()
                        .any(|member| member.eq_ignore_ascii_case(wanted)),
                )
            }
            "has_mio_research_category" => {
                let wanted = content_reader_parser::parser::c_string(
                    item.value
                        .scalar()
                        .ok_or("has_mio_research_category needs a scalar operand")?,
                );
                let (template, catalog) = self.organisation_definition(reference)?;
                let tree = organisation_tree(catalog, &template)?;
                Eval::from_bool(
                    tree.research_categories
                        .iter()
                        .any(|category| category.eq_ignore_ascii_case(wanted)),
                )
            }
            "has_mio_flag" => self.organisation_flag(item, reference, scopes)?,
            "is_mio_available" | "is_mio_visible" => {
                let (template, catalog) = self.organisation_definition(reference)?;
                let tree = organisation_tree(catalog, &template)?;
                let visible = (OrganisationCondition::Visible, tree.visible);
                let value = if name == "is_mio_available" {
                    self.organisation_conditions_value(
                        reference,
                        &[(OrganisationCondition::Available, tree.available), visible],
                    )
                } else {
                    self.organisation_conditions_value(reference, &[visible])
                };
                expect(value, expected(item))
            }
            _ => return Ok(None),
        }))
    }

    fn organisation_definition(&self, reference: u64) -> Result<(String, &Catalog), Failure> {
        let world = self.world();
        let token = world.industrial_organisation(reference)?.token;
        let literal = world.token(token)?.ok_or_else(|| -> Failure {
            format!("lexical tokens lacks industrial organisation definition token {token}").into()
        })?;
        let template = String::from_utf8(literal.to_vec())
            .map_err(|_| "the industrial organisation's definition token is not UTF-8")?;
        let catalog = world.mio_traits.map_err(Failure::from)?;
        Ok((template, catalog))
    }

    fn unlocked(&self, reference: u64, wanted: &[u8]) -> Result<bool, Failure> {
        let world = self.world();
        let unlocked = world.organisation_unlocked(reference)?;
        Ok(world
            .token_of(wanted)?
            .is_some_and(|token| unlocked.contains(&token)))
    }

    fn completed_trait_count(
        &self,
        item: &Item,
        reference: u64,
        scopes: Scopes,
    ) -> Result<Eval, Failure> {
        let unlocked = self.world().organisation_unlocked(reference)?;
        let mut seen = std::collections::BTreeSet::new();
        if !unlocked.iter().all(|token| seen.insert(*token)) {
            return Err(refused(
                "the recorded completed traits repeat a trait, which the game's set of completed traits cannot hold",
            ));
        }
        let count = i64::from(unlocked.len() as i32) * ONE;
        let (operand, comparison) = match item.operator {
            Operator::Less | Operator::Greater => (Some(item), item.operator),
            Operator::Equals => {
                let values: Vec<&Item> = item
                    .value
                    .items()
                    .unwrap_or_default()
                    .iter()
                    .filter(|field| field.keyword_is("value"))
                    .collect();
                match values.as_slice() {
                    [] => (None, Operator::Equals),
                    [value] => (Some(*value), value.operator),
                    _ => {
                        return Err(refused(
                            "has_mio_number_of_completed_traits assigns `value` more than once, and what the reused operand keeps is not reproduced",
                        ));
                    }
                }
            }
        };
        let wanted = match operand {
            Some(operand) if scope_selector(operand).is_some() => 0,
            Some(operand) => self.required(
                operand.value.scalar().unwrap_or_default(),
                operand.quoted(),
                false,
                scopes,
            )?,
            None => 0,
        };
        Ok(Eval::from_bool(compare(count, wanted, comparison, false)))
    }

    fn trait_available(&self, item: &Item, reference: u64) -> Result<Eval, Failure> {
        const PARENTS: &str = "check_mio_parent_completed";
        const EXCLUSIVE: &str = "check_mio_mutually_exclusive";
        let name = "is_mio_trait_available";
        let Some(wanted) = trait_operand(item, name, &[PARENTS, EXCLUSIVE])? else {
            return Ok(Eval::False);
        };
        let block = item.value.items().unwrap_or_default();
        let (template, catalog) = self.organisation_definition(reference)?;
        let Some(definition) = ordinary_trait(catalog, &template, wanted)? else {
            return Ok(Eval::False);
        };
        if read_bool_field(block, PARENTS, true) && !self.parents_completed(definition, reference)? {
            return Ok(Eval::False);
        }
        if read_bool_field(block, EXCLUSIVE, true) {
            for exclusive in &definition.mutually_exclusive {
                if self.unlocked(reference, exclusive.as_bytes())? {
                    return Ok(Eval::False);
                }
            }
        }
        let what = |condition: OrganisationCondition| {
            format!(
                "trait {} of industrial organisation {reference:#x}'s {}",
                definition.token,
                condition.name()
            )
        };
        Ok(self.conditions(
            reference,
            [
                (what(OrganisationCondition::Visible), definition.visible()),
                (what(OrganisationCondition::Available), definition.available()),
            ],
        ))
    }

    fn parents_completed(&self, definition: &TraitName, reference: u64) -> Result<bool, Failure> {
        for group in &definition.parents {
            let mut count = 0_i32;
            let mut met = false;
            for parent in &group.traits {
                if self.unlocked(reference, parent.as_bytes())? {
                    count = count.wrapping_add(1);
                }
                if count == group.required {
                    met = true;
                    break;
                }
            }
            if !met && count != group.required {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn organisation_conditions(
        &self,
        reference: u64,
        order: &[(OrganisationCondition, &[Item])],
    ) -> Result<bool, Undecided> {
        let decided = self.organisation_conditions_value(reference, order).decided();
        match self.divergence() {
            Some(why) => Err(Undecided::Diverged(why.to_string())),
            None => decided.map_err(Undecided::Unknown),
        }
    }

    fn organisation_conditions_value(
        &self,
        reference: u64,
        order: &[(OrganisationCondition, &[Item])],
    ) -> Eval {
        self.conditions(
            reference,
            order.iter().map(|&(condition, body)| {
                (
                    format!(
                        "industrial organisation {reference:#x}'s {}",
                        condition.name()
                    ),
                    body,
                )
            }),
        )
    }

    fn conditions<'b>(
        &self,
        reference: u64,
        parts: impl IntoIterator<Item = (String, &'b [Item])>,
    ) -> Eval {
        self.begin_composition();
        let mut result = Eval::True;
        for (what, body) in parts {
            let value = self.organisation_condition(reference, body, what);
            if let Some(why) = self.divergence() {
                if let Eval::Unknown(reasons) = &result {
                    for failure in reasons {
                        failure.consumed();
                    }
                }
                return Eval::unknown(why);
            }
            result = result.and(|| value);
            if result == Eval::False {
                break;
            }
        }
        result
    }

    fn organisation_condition(&self, reference: u64, body: &[Item], what: String) -> Eval {
        if body.is_empty() {
            return Eval::True;
        }
        let run = || -> Result<Eval, Failure> {
            let world = self.world();
            let owner = world.slot(world.industrial_organisation(reference)?.owner);
            if let Some(why) = self.enter_frame(what.clone().into_bytes()) {
                return Ok(self.diverge(refused(why)));
            }
            let graph = Rc::new(RefCell::new(EventScopes::organisation(reference, owner)));
            let _bound = self.bind_event_scopes(graph);
            let value = self
                .event_scopes(0)
                .map(|scopes| self.evaluate(body, scopes));
            self.leave_frame();
            value
        };
        run().unwrap_or_else(|why| Eval::unknown(why.context(|why| format!("{what}: {why}"))))
    }

    fn organisation_flag(
        &self,
        item: &Item,
        reference: u64,
        scopes: Scopes,
    ) -> Result<Eval, Failure> {
        let test = FlagTest::read(item, "has_mio_flag")?;
        let name = self.flag_name(test.name, scopes)?;
        let world = self.world();
        let Some(token) = world.token_of(&name)? else {
            return Ok(Eval::False);
        };
        let flags = world.organisation_flags(reference)?;
        Ok(Eval::from_bool(
            flags
                .iter()
                .find(|record| record.token == token)
                .is_some_and(|record| test.holds(record.value, record.set_hour, || world.clock())),
        ))
    }
}

fn expect(value: Eval, expected: bool) -> Eval {
    if expected { value } else { value.negate() }
}

fn organisation_tree<'c>(
    catalog: &'c Catalog,
    template: &str,
) -> Result<content_reader_military::mio_traits::Tree<'c>, Failure> {
    catalog.tree(template)?.ok_or_else(|| {
        format!("industrial organisation {template} is not defined in the recorded content stack")
            .into()
    })
}

fn ordinary_trait<'c>(
    catalog: &'c Catalog,
    template: &str,
    wanted: &[u8],
) -> Result<Option<&'c TraitName>, Failure> {
    organisation_tree(catalog, template)?;
    let wanted = std::str::from_utf8(wanted).map_err(|_| "the trait operand is not UTF-8")?;
    Ok(catalog.trait_definition(template, wanted)?)
}
