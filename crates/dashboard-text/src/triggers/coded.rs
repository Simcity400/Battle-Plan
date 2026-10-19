//! The coded triggers of country, state and unit-leader scopes, each read
//! from the table the game itself reads.
use crate::triggers::Failure;

use std::collections::BTreeSet;

use installed_content_core::{parser::Item, parser::Operator};
use replay_data::{rows::UnitLeaderRow, rows::tables};

use super::{
    Eval, Scope, Scopes,
    eval::Evaluator,
    operands::{compare, trigger_boolean},
    variables::Value,
    world::{LeaderVisibility, NULL_STATE, PartyLeader, table},
};
use crate::state_resources::ONE;

/// The folded keyword of a child of a trigger's block, `None` for a quoted
/// or anonymous key.
fn field_name(item: &Item) -> Option<String> {
    item.keyword()
        .map(|keyword| String::from_utf8_lossy(&keyword.folded()).into_owned())
}

impl Evaluator<'_, '_> {
    /// The country-scope triggers (accepted scope 4)
    /// this evaluator reproduces, on the
    /// scope's country; `None` for any other.
    pub(super) fn country_trigger(
        &self,
        name: &str,
        item: &Item,
        slot: u32,
        scopes: Scopes,
    ) -> Result<Option<Eval>, Failure> {
        let world = self.world();
        let scalar = item.value.scalar_str();
        let named = || self.country_operand(item, scopes);
        if name == "original_tag" {
            // Original-tag comparison uses the selected raw tag;
            // differing nonzero words compare their canonical mappings.
            // Resolve static specifiers with country substitution
            // and aliases before selecting originals.
            let operand = named()?.unwrap_or(0);
            // A known-null resolved specifier falls back to raw zero,
            // then resolves the authenticated reserved country.
            let wanted = world.original_reference(operand)?;
            return Ok(Some(Eval::from_bool(
                world.same_tag(world.original_reference(slot)?, wanted)?,
            )));
        }
        if matches!(
            name,
            "has_country_flag"
                | "has_completed_focus"
                | "has_government"
                | "has_country_leader_ideology"
        ) {
            // These getters read country identities and their own tables,
            // independently of the map.
            world.table_status(replay_data::rows::TableId::Countries)?;
            world.table_status(replay_data::rows::TableId::CountryLinks)?;
            let raw = world.raw_country(slot).ok_or_else(|| -> Failure {
                format!("Countries lacks country slot {slot}").into()
            })?;
            if world.slot(raw) != Some(slot) {
                return Err(world.unlinked("Countries", raw));
            }
            return Ok(Some(match name {
                "has_country_flag" => self.flag_trigger(item, 1, Some(raw))?,
                // Test the recorded completed-focus set.
                "has_completed_focus" => Eval::from_bool(
                    world.has_focus(
                        slot,
                        scalar
                            .ok_or("has_completed_focus needs a focus")?
                            .as_bytes(),
                    )?,
                ),
                "has_government" => {
                    // Compare IDs after ideology-specifier resolution;
                    // FROM/ROOT/tags and var: operands are reached.
                    let ideologies = world.ideologies.map_err(Failure::from)?;
                    let declared = |token: &[u8]| -> Result<bool, Failure> {
                        let (groups, gap) = ideologies.groups();
                        if groups
                            .iter()
                            .any(|group| group.token.as_bytes().eq_ignore_ascii_case(token))
                        {
                            return Ok(true);
                        }
                        match gap {
                            Some(gap) => Err(gap.to_string().into()),
                            None => Ok(false),
                        }
                    };
                    let (wanted, _) =
                        self.ideology_specifier(item.value.scalar(), scopes, declared)?;
                    let actual = world
                        .government(slot)?
                        .ok_or("PoliticalParties lacks ruling ideology")?;
                    Eval::from_bool(actual == wanted)
                }
                _ => {
                    // Read the ruling party's first leader's ideology,
                    // captured by CountryAppearance.
                    // Subtype tokens are interned globally.
                    let wanted = scalar
                        .ok_or("has_country_leader_ideology needs a subtype")?
                        .as_bytes();
                    let ideologies = world.ideologies.map_err(Failure::from)?;
                    let declared = |token: &[u8]| -> Result<(), Failure> {
                        let (groups, gap) = ideologies.groups();
                        if groups.iter().any(|group| {
                            group
                                .types
                                .iter()
                                .any(|kind| kind.as_bytes().eq_ignore_ascii_case(token))
                        }) {
                            return Ok(());
                        }
                        if let Some(gap) = gap {
                            return Err(gap.to_string().into());
                        }
                        Err(crate::triggers::refusal::refused(format!(
                            "leader ideology {} has no recorded-content declaration",
                            String::from_utf8_lossy(token)
                        )))
                    };
                    declared(wanted)?;
                    let actual = &world.appearance(slot)?.leader_ideology;
                    if !actual.is_empty() {
                        declared(actual)?;
                    }
                    Eval::from_bool(actual.eq_ignore_ascii_case(wanted))
                }
            }));
        }
        self.core()?;
        let is = |value: bool| -> Option<Eval> {
            // Simple boolean assignment
            // retains false on a failed read.
            Some(match trigger_boolean(item) {
                Ok(expect) => Eval::from_bool(value == expect),
                Err(reason) => Eval::unknown(reason),
            })
        };
        Ok(match name {
            "has_stability" | "has_war_support" => {
                let actual = self.country_stability_value(slot, name == "has_war_support")?;
                let wanted = self.required(
                    item.value.scalar().unwrap_or_default(),
                    item.quoted(),
                    true,
                    scopes,
                )?;
                Some(Eval::from_bool(compare(
                    actual,
                    wanted,
                    item.operator,
                    true,
                )))
            }
            "has_war" => is(self.country_has_war(slot)?),
            "is_ai" => is(self.country_is_ai(slot)?),
            "has_idea_with_trait" => Some(Eval::from_bool(
                self.idea_has_trait(slot, scalar.ok_or("has_idea_with_trait needs a trait")?)?,
            )),
            "is_in_tech_sharing_group" => Some(Eval::from_bool(
                self.in_tech_sharing_group(slot, item.value.scalar().unwrap_or_default())?,
            )),
            "is_neighbor_of" => {
                let wanted = named()?.unwrap_or(0);
                Some(Eval::from_bool(
                    self.country_neighbors(slot)?
                        .into_iter()
                        .any(|neighbor| neighbor == wanted),
                ))
            }
            "has_tech" => Some(Eval::from_bool(world.has_tech(
                slot,
                scalar.ok_or("has_tech needs a technology")?.as_bytes(),
            )?)),
            "has_war_with" => Some(Eval::from_bool(match named()? {
                Some(other) => world.at_war(slot, other)?,
                None => false,
            })),
            // Both countries' factions must be set and the same pointer,
            // the scope country itself included.
            "is_in_faction_with" => Some(Eval::from_bool(match named()? {
                Some(other) => world.same_faction(slot, other)?,
                None => false,
            })),
            // Compare the faction's first member with THIS;
            // having no faction is false
            // before comparing with its yes/no operand. The class accepts
            // country scopes only (0x4).
            "is_faction_leader" => {
                let leader = world.recorded().faction_leader_raw(slot)?;
                let matches = match leader {
                    None | Some(0) => false,
                    Some(raw) => world.recorded().raw_slot(raw)? == slot,
                };
                is(matches)
            }
            "is_subject" => is(world.overlord(slot)?.is_some()),
            // Elections are a boolean trigger in country scopes (mask 0x4).
            // The recorded political status supplies the result.
            "has_elections" => is(world.has_elections(slot)?),
            "is_special_project_completed" => {
                let name = scalar
                    .and_then(|value| value.strip_prefix("sp:"))
                    .ok_or("is_special_project_completed needs an sp: project")?;
                Some(Eval::from_bool(
                    world.completed_project(slot, name.as_bytes())?,
                ))
            }
            "num_of_nukes" => {
                let wanted: i64 = scalar
                    .and_then(|value| value.parse().ok())
                    .ok_or("num_of_nukes needs an integer")?;
                let actual = world.nukes(slot)?;
                Some(Eval::from_bool(match item.operator {
                    Operator::Less => actual < wanted,
                    Operator::Greater => actual > wanted,
                    Operator::Equals => actual == wanted,
                }))
            }
            "has_template" => Some(Eval::from_bool(
                world.has_template(slot, item.value.scalar().unwrap_or_default())?,
            )),
            "has_country_leader" => Some(self.has_country_leader(item, slot)?),
            // Controlled-state count uses an integer comparison.
            "num_of_controlled_states" => {
                let actual = world.controlled_state_count(slot)?;
                let bytes = item.value.scalar().unwrap_or_default();
                let wanted = self.required(bytes, item.quoted(), false, scopes)?;
                Some(Eval::from_bool(compare(
                    i64::from(actual as i32) * ONE,
                    wanted,
                    item.operator,
                    false,
                )))
            }
            "has_country_leader_with_trait" => {
                Some(self.has_country_leader_with_trait(item, slot)?)
            }
            "has_civil_war" => is(world.has_civil_war(slot)?),
            "has_idea" => Some(Eval::from_bool(
                world.has_idea(slot, scalar.ok_or("has_idea needs an idea")?)?,
            )),
            // The country's cosmetic tag equals the operand,
            // byte for byte.
            "has_cosmetic_tag" => Some(Eval::from_bool(
                scalar.ok_or("has_cosmetic_tag needs a tag")?.as_bytes()
                    == world.appearance(slot)?.cosmetic_tag.as_slice(),
            )),
            "has_dynamic_modifier" => {
                let (name, target) = self.modifier_operand(item, scopes)?;
                Some(Eval::from_bool(
                    world.has_country_modifier(slot, name, target)?,
                ))
            }
            "exists" => is(world.exists(slot)?),
            "is_ally_with" => Some(Eval::from_bool(world.is_ally_with(slot, named()?)?)),
            // The scope country's relation status toward the operand,
            // indexed by its canonical slot, holds a subject relation
            // whose overlord is the operand. The relation leaves that
            // slot when it ends, as `Subjects` rows end.
            "is_subject_of" => Some(Eval::from_bool(match named()? {
                Some(other) => world.overlord(slot)? == Some(other),
                None => false,
            })),
            // Captured-general evaluation asks for captured and visible
            // army leaders and never reads its operand: loading
            // stores the yes/no byte, and neither evaluation nor checking
            // reads it. So `= no` evaluates exactly as `= yes`.
            "has_any_captured_general" => {
                trigger_boolean(item)?;
                Some(self.any_captured_general(slot)?)
            }
            // The operand's unit leader, visible to its own country.
            // The operand is a scoped variable, decoded to its unit leader
            // after evaluation.
            "is_leader_visible" => {
                let operand = item.value.scalar().unwrap_or_default();
                Some(match self.scoped_scope(operand, scopes)? {
                    None => Eval::False,
                    Some(Scope::Leader(leader)) => match world.unit_leader(leader)? {
                        None => Eval::False,
                        Some(row) => self.leader_visible(row),
                    },
                    Some(other) => {
                        return Err(format!(
                            "the operand names {}, and what the trigger reads from a scope that is no unit leader is not reproduced",
                            self.describe(other)
                        ).into());
                    }
                })
            }
            // Compare the country's focus tree by its id, byte for byte; a country without one is
            // false. Focus holds the game's value every hour (replay_data::cadence).
            "has_focus_tree" => {
                let wanted = scalar.ok_or("has_focus_tree needs a focus tree")?;
                let raw = world.raw_country(slot).ok_or_else(|| -> Failure {
                    format!("country slot {slot} has no Countries row").into()
                })?;
                let tree = world.focus_tree(raw)?;
                Some(Eval::from_bool(tree.as_deref() == Some(wanted.as_bytes())))
            }
            "has_naval_invasion_against_state" => {
                Some(self.naval_invasion_against(item, slot, scopes)?)
            }
            _ => None,
        })
    }

    /// The state-scope triggers (accepted scope 2)
    /// this evaluator reproduces; `None` for any
    /// other. The null state holds no state: every class with a nonzero mask
    /// was already false there.
    pub(super) fn state_trigger(
        &self,
        name: &str,
        item: &Item,
        state: u32,
        scopes: Scopes,
    ) -> Result<Option<Eval>, Failure> {
        let world = self.world();
        if name == "is_on_continent" {
            // Compare the retained continent pointer;
            // template reload and province edits do not refresh it.
            // Resolve the operand through the continent database.
            let wanted = item
                .value
                .scalar_str()
                .ok_or("is_on_continent needs a continent")?
                .to_ascii_lowercase();
            let continents = world.continents.map_err(Failure::from)?;
            if !continents.contains(&wanted) {
                return Err(crate::triggers::refusal::refused(format!(
                    "continent {wanted} has no recorded-content declaration"
                )));
            }
            let token = world.state_continent_token(state)?;
            let actual = if token == 0 {
                None
            } else {
                let literal = world.token(token)?.ok_or_else(|| -> Failure {
                    format!("continent token {token} has no recorded lexer literal").into()
                })?;
                let name = std::str::from_utf8(literal)
                    .map_err(|_| "recorded continent token is not UTF-8")?
                    .to_ascii_lowercase();
                if !continents.contains(&name) {
                    return Err(crate::triggers::refusal::refused(format!(
                        "recorded continent {name} has no recorded-content declaration"
                    )));
                }
                Some(literal)
            };
            return Ok(Some(Eval::from_bool(actual.is_some_and(|actual| {
                actual.eq_ignore_ascii_case(wanted.as_bytes())
            }))));
        }
        self.core()?;
        if matches!(name, "is_fully_controlled_by" | "impassable" | "is_coastal") {
            world.state_content.map_err(Failure::from)?;
        }
        let (owner, controller) = world
            .state_row(state)
            .ok_or_else(|| -> Failure { format!("state {state} is not recorded").into() })?;
        let country = || self.country_operand(item, scopes);
        let provinces = world
            .state_provinces()
            .get(&state)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let is = |value: bool| {
            // These assign triggers share boolean reading.
            Some(match trigger_boolean(item) {
                Ok(expect) => Eval::from_bool(value == expect),
                Err(reason) => Eval::unknown(reason),
            })
        };
        Ok(match name {
            // Evaluate the operand as a state id.
            "state" => Some(Eval::from_bool(
                self.scoped_state(item.value.scalar().unwrap_or_default(), scopes)? == state,
            )),
            "is_core_of" => Some(world.is_core_of(state, country()?)?),
            "is_claimed_by" => Some(world.is_claimed_by(state, country()?)?),
            "is_in_home_area" => Some(Eval::from_bool(self.state_in_home_area(state)?)),
            "is_owned_by" => Some(Eval::from_bool(
                world.slot(owner).is_some() && world.slot(owner) == country()?,
            )),
            "is_controlled_by" => Some(Eval::from_bool(
                world.slot(controller).is_some() && world.slot(controller) == country()?,
            )),
            "is_owned_and_controlled_by" => {
                let wanted = country()?;
                Some(Eval::from_bool(
                    wanted.is_some()
                        && world.slot(owner) == wanted
                        && world.slot(controller) == wanted,
                ))
            }
            "is_fully_controlled_by" => {
                let wanted = country()?;
                Some(Eval::from_bool(
                    wanted.is_some()
                        && !provinces.is_empty()
                        && provinces
                            .iter()
                            .all(|&province| world.province_controller(province) == wanted),
                ))
            }
            "impassable" => is(world.impassable.contains(&state)),
            // A state is coastal when a
            // template province's definition has the coastal terrain bit,
            // from static content
            // ([`World::coastal`](super::World::coastal)).
            "is_coastal" => {
                let mut coastal = false;
                for &province in provinces {
                    match world.coastal(province) {
                        Ok(value) => coastal |= value,
                        Err(reason) => return Ok(Some(Eval::unknown(reason))),
                    }
                }
                is(coastal)
            }
            "has_state_flag" => Some(self.flag_trigger(item, 2, Some(state))?),
            "has_dynamic_modifier" => {
                let (name, target) = self.modifier_operand(item, scopes)?;
                Some(Eval::from_bool(
                    world.has_state_modifier(state, name, target)?,
                ))
            }
            _ => None,
        })
    }

    /// A dynamic modifier's name and auxiliary target:
    /// accept a scalar name or load the block; read only `modifier`
    /// as a string and `scope` as a scoped variable. The last name wins;
    /// unknown keys are logged and skipped by the shared content reader.
    /// Evaluate the scope operand (default 0), then
    /// decode it as a fresh auxiliary scope.
    fn modifier_operand<'i>(
        &self,
        item: &'i Item,
        scopes: Scopes,
    ) -> Result<(&'i [u8], Option<Scope>), Failure> {
        let Some(block) = item.value.items() else {
            let name = item
                .value
                .scalar()
                .ok_or("has_dynamic_modifier name is not a scalar")?;
            // The scalar loader stops at the first NUL; the block's string field
            // also checks LF, through the shared read_string below.
            return Ok((installed_content_core::parser::c_string(name), None));
        };
        let mut name = b"".as_slice();
        for field in block {
            if field.keyword_is("modifier") {
                name = field
                    .value
                    .read_string()
                    .ok_or("has_dynamic_modifier modifier is not a string token")?;
            } else if field.keyword_is("scope") && field.value.scalar().is_none() {
                return Err(crate::triggers::refusal::refused(
                    "has_dynamic_modifier scope is not a scalar scoped-variable token",
                ));
            }
        }
        // This is the same persistent scoped-variable value reader used by
        // MTTH operands, including its named refusal for a repeated parse
        // whose retained fields are not reproduced.
        let raw = self.mtth_operand("has_dynamic_modifier", block, "scope", 0, scopes)?;
        // Only the fresh auxiliary scope's positive country/state fields
        // matter to modifier lookup. Setting character (also used for a
        // decoded unit leader) carries neither field.
        let target = match self.decoded(Value::Raw(raw), scopes)? {
            Some(Scope::Country(slot) | Scope::CountryLeader { country: slot, .. }) => {
                let raw = self.world().raw_country(slot).ok_or_else(|| -> Failure {
                    format!("has_dynamic_modifier target country slot {slot} has no Countries row")
                        .into()
                })?;
                ((raw as i32) > 0).then_some(Scope::Country(slot))
            }
            Some(Scope::State(state)) if (state as i32) > 0 => Some(Scope::State(state)),
            _ => None,
        };
        Ok((name, target))
    }

    /// A flag trigger (`has_country_flag`, `has_state_flag`,
    /// `has_global_flag`, `has_character_flag`): the flag set, or its value
    /// compared by the `value` operator.
    pub(super) fn flag_trigger(
        &self,
        item: &Item,
        kind: u8,
        id: Option<u32>,
    ) -> Result<Eval, Failure> {
        let (name, test) = match item.value.items() {
            None => (
                item.value
                    .scalar_str()
                    .ok_or("the flag name is not text")?
                    .to_owned(),
                None,
            ),
            Some(block) => {
                let mut name = None;
                let mut test = None;
                for entry in block {
                    match field_name(entry).as_deref() {
                        Some("flag") => name = entry.value.scalar_str().map(str::to_owned),
                        Some("value") => {
                            test = Some((
                                entry.operator,
                                entry
                                    .value
                                    .scalar_str()
                                    .and_then(|value| value.parse::<i16>().ok())
                                    .ok_or("flag value is not an integer")?,
                            ))
                        }
                        _ => {
                            return Err(format!(
                                "flag field {} is not supported",
                                String::from_utf8_lossy(entry.key.as_deref().unwrap_or_default())
                            )
                            .into());
                        }
                    }
                }
                (name.ok_or("flag trigger names no flag")?, test)
            }
        };
        let id = id.ok_or("the flag's scope has no recorded id")?;
        let value = self.world().flag(kind, id, name.as_bytes())?;
        Ok(Eval::from_bool(match (value, test) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(value), Some((comparison, wanted))) => match comparison {
                Operator::Less => value < wanted,
                Operator::Greater => value > wanted,
                Operator::Equals => value == wanted,
            },
        }))
    }

    /// A state operand as scoped-variable state-id evaluation reads
    /// it, `None` for a state that does not exist (the trigger then logs and
    /// is false).
    fn state_operand(&self, value: &str, scopes: Scopes) -> Result<Option<u32>, Failure> {
        let state = self.scoped_state(value.as_bytes(), scopes)?;
        Ok((state != NULL_STATE).then_some(state))
    }

    /// Some invasion order of the country (`World::invasion_orders`),
    /// filtered by
    /// `activated` (if set, the order's active byte must equal its operand),
    /// has a path province whose state
    /// is the operand, and its preparation passes the comparison.
    ///
    /// Preparation is `World::invasion_preparation`, compared with its threshold
    /// by its operator (`>`: threshold <= preparation, `<`: preparation <
    /// threshold). Loading keeps any other operator
    /// after reporting an invalid comparison, and it then never passes.
    /// The scalar form leaves threshold 0 and `>`. The
    /// operands come from `InvasionPreparations`; without that table a
    /// `preparation` bound is unknown, and the scalar form reads any order as
    /// passing, since progress only accumulates from zero.
    fn naval_invasion_against(
        &self,
        item: &Item,
        slot: u32,
        scopes: Scopes,
    ) -> Result<Eval, Failure> {
        let world = self.world();
        let name = "has_naval_invasion_against_state";
        // (comparison, threshold fixed5); the scalar form's is (`>`, 0).
        let mut bound: Option<(Operator, i64)> = None;
        let (operand, activated) = match item.value.items() {
            None => (
                item.value
                    .scalar_str()
                    .ok_or_else(|| -> Failure { format!("{name} operand is not text").into() })?,
                None,
            ),
            Some(block) => {
                let mut operand = None;
                let mut activated = None;
                for entry in block {
                    match (field_name(entry).as_deref(), entry.value.scalar_str()) {
                        (Some("state"), Some(value)) => operand = Some(value),
                        (Some("activated"), Some("yes")) => activated = Some(true),
                        (Some("activated"), Some("no")) => activated = Some(false),
                        // Read fixed5 preparation.
                        (Some("preparation"), Some(value)) => {
                            let threshold = installed_content_core::fixed5::script_fixed5(value.as_bytes())
                                .ok_or_else(|| -> Failure {
                                    format!("{name} preparation {value} is not a number").into()
                                })?;
                            bound = Some((entry.operator, threshold));
                        }
                        _ => {
                            return Err(format!(
                                "{name} field {} is not supported",
                                String::from_utf8_lossy(entry.key.as_deref().unwrap_or_default())
                            )
                            .into());
                        }
                    }
                }
                (
                    operand
                        .ok_or_else(|| -> Failure { format!("{name} names no state").into() })?,
                    activated,
                )
            }
        };
        let Some(state) = self.state_operand(operand, scopes)? else {
            return Ok(Eval::False);
        };
        let explicit = bound.is_some();
        let (comparison, threshold) = bound.unwrap_or((Operator::Greater, 0));
        let passes = |preparation: i64| match comparison {
            Operator::Greater => threshold <= preparation,
            Operator::Less => preparation < threshold,
            Operator::Equals => false,
        };
        Ok(world.invasion_orders(slot)?.any(|order| {
            if activated.is_some_and(|wanted| (order.active != 0) != wanted) {
                return Eval::False;
            }
            let touches = order.path.iter().any(|province| {
                world
                    .provinces()
                    .get(province)
                    .is_some_and(|row| row.state == state)
            });
            if !touches {
                return Eval::False;
            }
            match world.invasion_preparation(order.key) {
                Ok(preparation) => Eval::from_bool(passes(preparation)),
                Err(_) if !explicit => Eval::True,
                Err(reason) => {
                    Eval::unknown(reason.context(|reason| format!("{name} preparation: {reason}")))
                }
            }
        }))
    }

    /// Build the list from the `array` operand's `^num` and `^<i>` entries
    /// ([`Evaluator::array`]), each read as a state,
    /// and return true once `count` (default 1) of them pass the body
    /// in state scope. A `count` below one is met by the first entry,
    /// whatever the body says.
    pub(super) fn any_state_in(&self, block: &[Item], scopes: Scopes) -> Eval {
        let mut array = None;
        let mut count: i64 = 1;
        let mut body = Vec::new();
        for item in block {
            match field_name(item).as_deref() {
                Some("array") => array = item.value.scalar(),
                Some("count") => match item.value.scalar_str().and_then(|v| v.parse().ok()) {
                    Some(value) => count = value,
                    None => {
                        return Eval::unknown("any_state_in count is not an integer");
                    }
                },
                Some("tooltip") => {}
                _ => body.push(item.clone()),
            }
        }
        let Some(array) = array else {
            return Eval::unknown("any_state_in names no array");
        };
        let shown = String::from_utf8_lossy(array);
        let values = match self
            .operand(array)
            .and_then(|operand| self.array(&operand, scopes))
        {
            Ok(values) => values,
            Err(reason) => {
                return Eval::unknown(
                    reason.context(|reason| format!("any_state_in array {shown}: {reason}")),
                );
            }
        };
        if values.is_empty() {
            return Eval::False;
        }
        // A null entry is visited too: none are skipped,
        // and state 0 is installed in the child scope.
        let world = self.world();
        let states = values.into_iter().enumerate().map(|(index, value)| {
            match value {
                Value::Object(Scope::State(id)) => Ok(id),
                value => self.raw(value).and_then(|raw| world.state_of_value(raw)),
            }
            .map_err(|reason| {
                reason.context(|reason| format!("any_state_in: {shown}^{index}: {reason}"))
            })
        });
        self.count_passing(states, count, &body, scopes)
    }

    fn state_iteration_order_sensitive(
        &self,
        body: &[Item],
        seen: &mut BTreeSet<Vec<u8>>,
    ) -> Option<String> {
        for item in body {
            let key = field_name(item).unwrap_or_default();
            if super::variables::TEMP_WRITERS.contains(&key.as_str()) || key.contains("random") {
                return Some(format!("order-sensitive trigger {key}"));
            }
            if let Some(bytes) = item.value.scalar()
                && let Ok(operand) = self.operand(bytes)
                && operand.random
            {
                return Some(format!("random operand {}", String::from_utf8_lossy(bytes)));
            }
            if let Some(children) = item.value.items()
                && let Some(why) = self.state_iteration_order_sensitive(children, seen)
            {
                return Some(why);
            }
            if let Ok(Some(super::eval::Child::Scripted(template))) = self.child(item)
                && seen.insert(template.name.clone())
                && let Ok(children) = &template.body
                && let Some(why) = self.state_iteration_order_sensitive(children, seen)
            {
                return Some(why);
            }
        }
        None
    }

    /// Owned- and controlled-state iteration:
    /// only nonempty states enter either stored roster.
    pub(super) fn any_country_state(&self, name: &str, block: &[Item], scopes: Scopes) -> Eval {
        let controlled = name.ends_with("controlled_state");
        let all = name.starts_with("all_");
        let run = || -> Result<Eval, Failure> {
            let (Scope::Country(country) | Scope::CountryLeader { country, .. }) = scopes.this
            else {
                return Ok(Eval::False);
            };
            let recorded = self.world().country_state_roster(country, controlled)?;
            if recorded.is_none() {
                self.core()?;
            }
            if recorded.is_none()
                && let Some(why) = self.state_iteration_order_sensitive(block, &mut BTreeSet::new())
            {
                return self
                    .world()
                    .ordered_country_states(country, controlled)
                    .map(|_| unreachable!("an absent optional roster cannot have recorded order"))
                    .map_err(|failure| {
                        failure.context(|reason| {
                            format!("{name}: {why}; the roster order is required; {reason}")
                        })
                    });
            }
            let mut count = 1_i32;
            let mut body = Vec::new();
            for item in block {
                match field_name(item).as_deref() {
                    Some("count") => {
                        count = (self
                            .scoped_value(item.value.scalar().unwrap_or_default(), scopes)?
                            / ONE) as i32
                    }
                    Some("tooltip") => {}
                    _ => body.push(item.clone()),
                }
            }
            if recorded.is_none() && count == i32::MIN {
                return self.world().ordered_country_states(country, controlled)
                    .map(|_| unreachable!("an absent optional roster cannot have recorded order"))
                    .map_err(|failure| failure.context(|reason| format!("{name}: count wraps from i32::MIN on a passing first body, so the roster order is required; {reason}")));
            }
            let mut undecided = 0_i64;
            let mut reasons = BTreeSet::new();
            let derived;
            let states = if let Some(recorded) = recorded {
                recorded
            } else {
                let provinces = table::<tables::Provinces>(self.world().frame())?;
                let nonempty: BTreeSet<_> =
                    provinces.iter().map(|province| province.state).collect();
                derived = table::<tables::States>(self.world().frame())?
                    .iter()
                    .filter_map(|state| {
                        let raw = if controlled {
                            state.controller
                        } else {
                            state.owner
                        };
                        if raw == 0 || !nonempty.contains(&state.id) {
                            return None;
                        }
                        match self.world().slot(raw) {
                            Some(owner) if owner == country => Some(state.id),
                            Some(_) => None,
                            None => {
                                undecided += 1;
                                let why = format!(
                                    "state {} {} {raw} has no CountryLinks row",
                                    state.id,
                                    if controlled { "controller" } else { "owner" }
                                );
                                reasons.insert(Failure::from(why));
                                None
                            }
                        }
                    })
                    .collect::<Vec<_>>();
                derived.as_slice()
            };
            for &state in states {
                let entered = self.with_this(scopes, Scope::State(state))?;
                match self.evaluate(&body, entered) {
                    Eval::True if !all => count = count.wrapping_sub(1),
                    Eval::True => {}
                    Eval::False if all => return Ok(Eval::False),
                    Eval::False => {}
                    Eval::Unknown(why) => {
                        undecided += 1;
                        reasons.extend(why);
                        // At MIN a passing body wraps to MAX, while a false
                        // body leaves the count below one.
                        if count == i32::MIN {
                            return Ok(Eval::Unknown(reasons));
                        }
                    }
                }
                // Unlike any_state_in, this checks count AFTER the visited body.
                if !all && count < 1 {
                    return Ok(Eval::True);
                }
            }
            Ok(if undecided > 0 && (all || i64::from(count) <= undecided) {
                Eval::Unknown(reasons)
            } else if all {
                Eval::True
            } else {
                Eval::False
            })
        };
        run().unwrap_or_else(Eval::unknown)
    }

    /// The native `any_*` loop over a non-empty state list: `count` falls by
    /// one for each state passing the body in state scope, and the trigger is
    /// true once it is below one, so a `count` below one is met by the first
    /// state whatever the body says. An empty list is false; a state that
    /// cannot be read counts as undecided.
    fn count_passing(
        &self,
        states: impl IntoIterator<Item = Result<u32, Failure>>,
        count: i64,
        body: &[Item],
        scopes: Scopes,
    ) -> Eval {
        let (mut passed, mut undecided) = (0_i64, 0_i64);
        let mut reasons = BTreeSet::new();
        for state in states {
            if count < 1 {
                return Eval::True;
            }
            let value = match state {
                Ok(state) => match self.with_this(scopes, Scope::State(state)) {
                    Ok(entered) => self.evaluate(body, entered),
                    Err(why) => Eval::unknown(why),
                },
                Err(reason) => Eval::unknown(reason),
            };
            match value {
                Eval::True => passed += 1,
                Eval::False => {}
                Eval::Unknown(why) => {
                    undecided += 1;
                    reasons.extend(why);
                }
            }
            if passed >= count {
                return Eval::True;
            }
        }
        if undecided > 0 && passed + undecided >= count {
            Eval::Unknown(reasons)
        } else {
            Eval::False
        }
    }

    /// Any neighbouring state (true once `count`, default
    /// 1, neighbours pass the body) and all neighbouring states
    /// (false at the first neighbour that fails; its `count` is read and
    /// ignored) over the scope state's
    /// template list, which content decides
    /// ([`ProvinceGraph::neighbor_states`](installed_content_core::province_graph::ProvinceGraph::neighbor_states)).
    pub(super) fn neighbor_state(&self, name: &str, block: &[Item], scopes: Scopes) -> Eval {
        let mut count: i64 = 1;
        let mut body = Vec::new();
        for item in block {
            match field_name(item).as_deref() {
                Some("count") => match item.value.scalar_str().and_then(|v| v.parse().ok()) {
                    Some(value) => count = value,
                    None => {
                        return Eval::unknown(format!("{name} count is not an integer"));
                    }
                },
                Some("tooltip") => {}
                _ => body.push(item.clone()),
            }
        }
        let Scope::State(state) = scopes.this else {
            return Eval::unknown(format!("{name} is read in state scope only"));
        };
        let world = self.world();
        if let Err(why) = self.core() {
            return Eval::unknown(why.context(|why| format!("{name}: {why}")));
        }
        let lists = match world.neighbor_states() {
            Ok(lists) => lists,
            Err(reason) => {
                return Eval::unknown(
                    Failure::from(reason).context(|reason| format!("{name}: {reason}")),
                );
            }
        };
        let Some(neighbours) = lists.get(&state) else {
            return Eval::unknown(format!("{name}: state {state} is not in the content"));
        };
        let states = neighbours.iter().map(|&neighbour| {
            if world.state_row(neighbour).is_some() {
                Ok(neighbour)
            } else {
                Err(format!("{name}: neighbour state {neighbour} is not recorded").into())
            }
        });
        if name == "any_neighbor_state" {
            return self.count_passing(states, count, &body, scopes);
        }
        let mut result = Eval::True;
        for state in states {
            let value = match state {
                Ok(state) => match self.with_this(scopes, Scope::State(state)) {
                    Ok(entered) => self.evaluate(&body, entered),
                    Err(why) => Eval::unknown(why),
                },
                Err(reason) => Eval::unknown(reason),
            };
            result = result.and(|| value);
            if result == Eval::False {
                break;
            }
        }
        result
    }

    /// Leader visibility in a scope whose country is the leader's owner:
    /// both callers set country in a default event scope, either
    /// to the owner or to the roster's country, which owns every leader
    /// it lists, so ROOT and THIS
    /// are that country, there is no FROM or PREV and no character.
    ///
    /// Evaluate the stored template role first; otherwise use the linked
    /// character's template role when present, else the leader is visible.
    /// Both roles come from the character's retained template. Building a
    /// leader from that template is the only writer of its stored role;
    /// linking the character records whether it has a template. Role effects
    /// apply only skills and traits of their own block, and neither they nor
    /// legacy leader creation store a role, so an effect's `visible` is never
    /// read. The predicate is the template role's `visible` conjunction.
    pub(crate) fn leader_visible(&self, row: &UnitLeaderRow) -> Eval {
        self.decide_leader_visibility(row, self.world().leader_visibility(row), false)
    }

    /// Roster traversal keeps the country when setting the character.
    /// Its captured-role gate is applied by the roster caller before this lookup.
    /// The roster caches the first step itself; tests compare against this.
    #[cfg(test)]
    pub(crate) fn leader_roster_visible(&self, row: &UnitLeaderRow) -> Eval {
        self.decide_leader_visibility(row, self.world().leader_visibility(row), true)
    }

    /// A leader's [`LeaderVisibility`] decided in its owner's scope: the
    /// second step of [`Self::leader_visible`], or with `roster` of
    /// [`Self::leader_roster_visible`].
    pub fn decide_leader_visibility(
        &self,
        row: &UnitLeaderRow,
        visibility: LeaderVisibility,
        roster: bool,
    ) -> Eval {
        let world = self.world();
        let visible: &[Item] = match visibility {
            LeaderVisibility::Decided(value) => return value,
            LeaderVisibility::Unconditioned => &[],
            LeaderVisibility::Conditioned(index) => world.role_visible(index),
        };
        let Some(slot) = world.slot(row.owner) else {
            return Eval::unknown(world.unlinked("UnitLeaders", row.owner));
        };
        let frame = format!("unit leader {}'s role", row.id).into_bytes();
        if let Some(why) = self.enter_frame(frame) {
            return Eval::unknown(why);
        }
        let scope = if roster {
            Scope::CountryLeader {
                country: slot,
                leader: u64::from(row.id) << 32 | u64::from(row.reference_type),
            }
        } else {
            Scope::Country(slot)
        };
        let value = self.evaluate(visible, Scopes::of(scope));
        self.leave_frame();
        value
    }

    /// Country leader selection reads
    /// `character` (a token), `id`,
    /// `name` and `ruling_only`. False when the ruling party
    /// has no leaders. The leaders are the ruling party's first
    /// with `ruling_only`, else every party's. The first
    /// operand set decides: `character` is the country's active character of
    /// that token (the first record of the token in CountryCharacters)
    /// and matches a leader's character; `name` matches the leader's name
    /// (Characters.name); a nonzero `id` matches the role's stored id.
    fn has_country_leader(&self, item: &Item, slot: u32) -> Result<Eval, Failure> {
        let world = self.world();
        let block = item
            .value
            .items()
            .ok_or("has_country_leader needs a block")?;
        let (mut character, mut name, mut id, mut ruling_only) = (None, None, 0_i32, false);
        for entry in block {
            let value = entry.value.scalar_str();
            match field_name(entry).as_deref() {
                Some("character") => character = value,
                Some("name") => name = value,
                Some("id") => {
                    id = value
                        .and_then(|value| value.parse().ok())
                        .ok_or("has_country_leader id is not an integer")?
                }
                Some("ruling_only") => {
                    // Read the ruling-only boolean field.
                    ruling_only = entry
                        .value
                        .scalar()
                        .and_then(installed_content_core::parser::read_bool)
                        .unwrap_or(ruling_only);
                }
                _ => {
                    return Err(format!(
                        "has_country_leader {} is not supported",
                        String::from_utf8_lossy(entry.key.as_deref().unwrap_or_default())
                    )
                    .into());
                }
            }
        }
        let parties = world.party_leaders(slot)?;
        let ruling = parties.iter().find(|party| party.ruling);
        if ruling.is_some_and(|party| party.leaders.is_empty()) {
            return Ok(Eval::False);
        }
        let leaders: Vec<PartyLeader> = if ruling_only {
            let ruling = ruling.ok_or("the country has no ruling party")?;
            ruling.leaders.first().copied().into_iter().collect()
        } else {
            parties
                .iter()
                .flat_map(|party| party.leaders.iter().copied())
                .collect()
        };
        if let Some(character) = character {
            let Some(token) = world.token_of(character.as_bytes())? else {
                return Ok(Eval::False);
            };
            let mut target = None;
            for id in world.country_characters(slot)?.into_iter().flatten() {
                let row = world.character(id)?.ok_or_else(|| -> Failure {
                    format!("character {id} is not recorded").into()
                })?;
                if u32::try_from(row.name_token_id).ok() == Some(token) {
                    target = Some(id);
                    break;
                }
            }
            return Ok(Eval::from_bool(target.is_some_and(|target| {
                leaders
                    .iter()
                    .any(|leader| leader.character == Some(target))
            })));
        }
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            let mut result = Eval::False;
            for leader in &leaders {
                let value = match leader.character {
                    None => Eval::from_bool(false),
                    Some(id) => {
                        let row = world.character(id)?.ok_or_else(|| -> Failure {
                            format!("character {id} is not recorded").into()
                        })?;
                        // An empty name falls back to localising its name token,
                        // but every name-token writer stores that same
                        // localisation into the name, so the fallback
                        // is the empty name again.
                        Eval::from_bool(row.name == name.as_bytes())
                    }
                };
                result = result.or(|| value);
            }
            return Ok(result);
        }
        Ok(Eval::from_bool(
            id != 0 && leaders.iter().any(|leader| leader.id == id),
        ))
    }

    /// A real trait (its definition marker is set on construction; the null object
    /// for an unknown name has none) held by the ruling party's first leader
    /// in the recorded trait roster.
    fn has_country_leader_with_trait(&self, item: &Item, slot: u32) -> Result<Eval, Failure> {
        let world = self.world();
        let name = item
            .value
            .scalar_str()
            .ok_or("has_country_leader_with_trait needs a trait")?;
        Ok(Eval::from_bool(world.leader_has_trait(slot, name)?))
    }

    /// Original-tag iteration: the countries listed for the original
    /// tag of `original_tag_to_check`, using the recorded original-tag buckets
    /// kept in step with each country's independent `original_tag` field,
    /// and then the original country itself. A country whose tag
    /// does not resolve is skipped; each other is
    /// the scope of the children's conjunction. `any` holds
    /// once `count` (default 1) pass, `all` unless one fails.
    pub(super) fn countries_with_original_tag(
        &self,
        name: &str,
        block: &[Item],
        scopes: Scopes,
    ) -> Result<Eval, Failure> {
        let world = self.world();
        let mut check = None;
        let mut count: i64 = 1;
        let mut children = Vec::new();
        for item in block {
            match field_name(item).as_deref() {
                Some("original_tag_to_check") => check = Some(item),
                // `tooltip` is text; `count`
                // is a scoped variable, 1 by default; `original_tag`
                // is reported as the wrong key
                // and adds no child.
                Some("tooltip" | "original_tag") => {}
                // Evaluated in the trigger's scope.
                Some("count") => {
                    count =
                        self.scoped_value(item.value.scalar().unwrap_or_default(), scopes)? / ONE;
                }
                _ => children.push(item.clone()),
            }
        }
        let check = check
            .ok_or_else(|| -> Failure { format!("{name} needs original_tag_to_check").into() })?;
        let Some(target) = self.country_operand(check, scopes)? else {
            return Ok(Eval::from_bool(name.starts_with("all")));
        };
        let selected = world.original_reference(target)?;
        if selected <= 0 {
            return Ok(Eval::from_bool(name.starts_with("all")));
        }
        let recorded = world.recorded();
        let mut countries = Vec::new();
        for raw in recorded
            .original_bucket(selected)?
            .iter()
            .copied()
            .chain(std::iter::once(selected as u32))
        {
            // Test the resolved country's own signed tag.
            if let Some(slot) = recorded.positive_country(raw)? {
                countries.push(slot);
            }
        }
        if name.starts_with("all") {
            let mut result = Eval::True;
            for country in countries {
                let entered = self.with_this(scopes, Scope::Country(country))?;
                result = result.and(|| self.evaluate(&children, entered));
                if result == Eval::False {
                    break;
                }
            }
            return Ok(result);
        }
        // `any`: the count, truncated to whole units, drops
        // by one per passing country and holds once below one, checked after
        // each visited country; an unknown country leaves the count open.
        let (mut passed, mut open) = (0_i64, 0_i64);
        let mut reasons = BTreeSet::new();
        for country in countries {
            match self.evaluate(&children, self.with_this(scopes, Scope::Country(country))?) {
                Eval::True => passed += 1,
                Eval::False => {}
                Eval::Unknown(more) => {
                    open += 1;
                    reasons.extend(more);
                }
            }
            if count - passed < 1 {
                return Ok(Eval::True);
            }
        }
        Ok(if open > 0 && count - passed - open < 1 {
            // Some order of the unknowns reaches the count, another may not.
            Eval::Unknown(reasons)
        } else {
            Eval::False
        })
    }

    /// An army leader of the country's roster
    /// (the leaders whose owner is the country, which capture leaves unchanged)
    /// that is captured and visible in a scope of the country
    /// ([`Self::leader_visible`]).
    fn any_captured_general(&self, slot: u32) -> Result<Eval, Failure> {
        let world = self.world();
        let mut result = Eval::False;
        for row in table::<tables::UnitLeaders>(world.frame())? {
            if row.captured == 0 {
                continue;
            }
            match world.slot(row.owner) {
                Some(owner) if owner == slot => {
                    result = result.or(|| self.leader_visible(row));
                    if result == Eval::True {
                        break;
                    }
                }
                Some(_) => {}
                None => {
                    result = result.or(|| {
                        Eval::unknown(format!(
                            "a UnitLeaders row's country {} has no CountryLinks row",
                            row.owner
                        ))
                    });
                }
            }
        }
        Ok(result)
    }
}
