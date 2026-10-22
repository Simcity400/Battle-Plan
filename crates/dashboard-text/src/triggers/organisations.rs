//! The coded triggers of an industrial organisation's scope, each read from
//! the organisation's recorded details and its definition in the recorded
//! content stack. Every one of them accepts the organisation scope alone, which
//! the shared scope check already decides, and reads the current organisation.
use std::{cell::RefCell, rc::Rc};

use crate::triggers::{Failure, refusal::refused};

use installed_content_core::{
    mio_traits::{Catalog, TraitName},
    parser::{Item, Operator, read_bool, read_bool_field},
};

use super::{
    Eval, Scopes,
    eval::Evaluator,
    flags::FlagTest,
    operands::{compare, scope_selector},
};
use crate::{scripted_text::EventScopes, state_resources::ONE};

/// The text of a token operand: a scalar, bare. A quoted operand loads as a
/// generic string token, not the registered token of the same text, and
/// which recorded token that is, is not reproduced.
fn token_text<'i>(item: &'i Item, name: &str) -> Result<&'i [u8], Failure> {
    if item.quoted() {
        return Err(refused(format!(
            "{name}'s quoted operand loads as a generic string token, whose identity is not reproduced"
        )));
    }
    item.value
        .scalar()
        .map(installed_content_core::parser::c_string)
        .ok_or_else(|| format!("{name} needs a scalar operand").into())
}

/// A trait leaf's operand: a scalar trait token, or a block whose `trait`
/// names it, beside tooltip-only `show_modifiers` and, for trait
/// availability, the two check toggles. A block without `trait` names token
/// zero, which no trait has.
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

/// Which of an organisation's two conditions.
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

/// Why a composition of conditions has no result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Undecided {
    /// The recording does not decide it: a later proven blocker still can.
    Unknown(String),
    /// The game recurses without end in it: nothing after it is reached, so
    /// nothing can decide it.
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

/// A MIO Boolean leaf's expected value. The reader clears it to false, then
/// reads exact `yes` or `no`; a scope selector it recognises instead (or any
/// other text) leaves it false.
fn expected(item: &Item) -> bool {
    if scope_selector(item).is_some() {
        return false;
    }
    item.value.scalar().and_then(read_bool).unwrap_or(false)
}

impl Evaluator<'_, '_> {
    /// A coded trigger in an industrial organisation's scope; `None` for one
    /// this evaluator does not reproduce, which is refused by name.
    pub(super) fn organisation_trigger(
        &self,
        name: &str,
        item: &Item,
        reference: u64,
        scopes: Scopes,
    ) -> Result<Option<Eval>, Failure> {
        let world = self.world();
        Ok(Some(match name {
            // The organisation's size, compared as an integer trigger compares (the class of
            // the controlled-state count): `=` is equality, a variable operand is truncated
            // to whole units, and a scope selector leaves the threshold zero. The reader has
            // no block form; how it recovers from one is not reproduced.
            "has_mio_size" => {
                // The game takes an opening brace as a scalar, then reads the block's fields.
                if item.kind == installed_content_core::parser::Kind::Open {
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
            // The operand's lexer token among the organisation's allowed policies; the
            // attached policy and the policies' own conditions take no part.
            "has_mio_policy" => {
                let token = world.token_of(token_text(item, name)?)?;
                let (allowed, _) = world.organisation_policies(reference)?;
                Eval::from_bool(token.is_some_and(|token| allowed.contains(&token)))
            }
            // The attached policy's token is the operand's, compared raw: the no-selection
            // value is a token like any other, and allowed membership is not required.
            "has_mio_policy_active" => {
                let token = world.token_of(token_text(item, name)?)?;
                let (_, attached) = world.organisation_policies(reference)?;
                Eval::from_bool(token == Some(attached))
            }
            // The organisation's definition token is the operand's: any instance of that
            // definition passes.
            "is_military_industrial_organization" => {
                let token = world.token_of(token_text(item, name)?)?;
                let definition = world.industrial_organisation(reference)?.token;
                Eval::from_bool(token == Some(definition))
            }
            // Any task slot, as the game counts its task array: a retained empty slot counts.
            "is_mio_assigned_to_task" => {
                let assigned = !world.organisation_tasks(reference)?.is_empty();
                Eval::from_bool(assigned == expected(item))
            }
            // The definition holds an ordinary trait of that token (first match); completion
            // and the separate initial trait take no part.
            "has_mio_trait" => {
                let wanted = trait_operand(item, name, &[])?;
                let (template, catalog) = self.organisation_definition(reference)?;
                Eval::from_bool(match wanted {
                    Some(wanted) => ordinary_trait(catalog, &template, wanted)?.is_some(),
                    None => false,
                })
            }
            // An ordinary trait of that token exists and its token is completed.
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
            // The operand among the definition's equipment types, expanded through the
            // equipment groups with the group names kept, compared without case.
            "has_mio_equipment_type" => {
                let wanted = installed_content_core::parser::c_string(
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
            // The operand among the definition's research categories, without case.
            "has_mio_research_category" => {
                let wanted = installed_content_core::parser::c_string(
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
            "has_mio_flag" => self.organisation_flag(item, reference)?,
            // `available`, then `visible` only if it holds.
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

    /// The organisation's definition token's spelling and the recorded stack's
    /// organisation definitions.
    fn organisation_definition(&self, reference: u64) -> Result<(String, &Catalog), Failure> {
        let world = self.world();
        let token = world.industrial_organisation(reference)?.token;
        let literal = world.token(token)?.ok_or_else(|| -> Failure {
            format!("LexicalTokens lacks industrial organisation definition token {token}").into()
        })?;
        let template = String::from_utf8(literal.to_vec())
            .map_err(|_| "the industrial organisation's definition token is not UTF-8")?;
        let catalog = world.mio_traits.map_err(Failure::from)?;
        Ok((template, catalog))
    }

    /// Whether the organisation's completed traits hold the trait of this text.
    fn unlocked(&self, reference: u64, wanted: &[u8]) -> Result<bool, Failure> {
        let world = self.world();
        let unlocked = world.organisation_unlocked(reference)?;
        Ok(world
            .token_of(wanted)?
            .is_some_and(|token| unlocked.contains(&token)))
    }

    /// The completed-trait count, an integer comparison: `<` and `>` read a
    /// scalar operand, while `=` reads a block whose `value` field, by its own
    /// operator, sets the threshold and the test (`=` is equality). Outer `=`
    /// with a scalar, or a block without `value`, keeps threshold zero and
    /// tests equality; a scope selector operand leaves the threshold zero. The
    /// count is the game's set of completed traits, whose members are unique.
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
            // The integer reader clears its threshold to zero; a scope selector it
            // recognises leaves it there.
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

    /// Trait availability, in the game's order: the ordinary trait exists; with
    /// `check_mio_parent_completed` (default yes) every parent group is met;
    /// with `check_mio_mutually_exclusive` (default yes) no exclusive trait is
    /// completed; then the trait's `visible`, then its `available`. Whether the
    /// trait itself is completed, and the organisation's own conditions, take
    /// no part.
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

    /// Every parent group is met: walking its parents in order, a completed
    /// one (each occurrence) adds one to the count, and the group is met once
    /// the count equals the number needed after any occurrence, or ends equal
    /// to it.
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

    /// Whether the organisation's conditions in `order` hold, as the game
    /// composes them ([`Self::conditions`]): the row gate takes `available`
    /// then `visible`, the attach quote's traits `visible` then `available`.
    /// A condition the game never returns from is [`Undecided::Diverged`],
    /// which no later part of a caller's composition can decide.
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

    /// [`Self::organisation_conditions`] as a trigger value.
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

    /// Conditions of the organisation, each named for its frame, evaluated in
    /// order while the ones before hold: a false one decides, an undecided
    /// one lets a later false one decide, and a reached endless recursion
    /// decides nothing, since the game never returns from it.
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

    /// One of the organisation's or a trait's conditions, as the game
    /// evaluates it: an empty body holds; any other is evaluated in a fresh
    /// pair of scopes, the organisation (ROOT and PREV itself, FROM its owner,
    /// unknown when the owner does not resolve) and its owner country (every
    /// link itself), whatever the caller's. The game guards no depth, so a
    /// body reached inside itself recurses without end ([`Self::diverge`]).
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

    /// `has_mio_flag`: the shared flag test ([`FlagTest`]) on the
    /// organisation's own flags.
    fn organisation_flag(&self, item: &Item, reference: u64) -> Result<Eval, Failure> {
        let test = FlagTest::read(item, "has_mio_flag")?;
        let world = self.world();
        let Some(token) = world.token_of(test.name)? else {
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

/// A body's result compared with the trigger's expected yes or no.
fn expect(value: Eval, expected: bool) -> Eval {
    if expected { value } else { value.negate() }
}

/// The organisation's resolved definition; a definition the recorded stack
/// lacks is a missing input, not the game's empty fallback.
fn organisation_tree<'c>(
    catalog: &'c Catalog,
    template: &str,
) -> Result<installed_content_core::mio_traits::Tree<'c>, Failure> {
    catalog.tree(template)?.ok_or_else(|| {
        format!("industrial organisation {template} is not defined in the recorded content stack")
            .into()
    })
}

/// The definition's first ordinary trait of this token.
fn ordinary_trait<'c>(
    catalog: &'c Catalog,
    template: &str,
    wanted: &[u8],
) -> Result<Option<&'c TraitName>, Failure> {
    organisation_tree(catalog, template)?;
    let wanted = std::str::from_utf8(wanted).map_err(|_| "the trait operand is not UTF-8")?;
    Ok(catalog.trait_definition(template, wanted)?)
}
