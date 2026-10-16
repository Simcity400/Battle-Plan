//! MIO trait names and bonuses held by the template reader and include pass.

use crate::{
    ResolvedLayers,
    fields::invalid,
    key_names::KeyNames,
    modifiers,
    parser::{
        Item,
        native::{self, Shape},
    },
    script_files,
    tokens::Token,
};
use std::io;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraitName {
    pub token: String,
    /// Optional replacement for `<organisation>_<trait>`.
    pub name: String,
    deleted: Vec<Token>,
    /// Equipment and production bonuses in stored order.
    pub bonuses: Vec<Bonus>,
    /// Organisation modifiers read through the shared modifier reader.
    pub organisation_modifiers: modifiers::Modifiers,
    /// Shared modifier-copy/effective-emptiness API needed only by inheritance.
    pub modifier_inheritance_refusal: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bonus {
    pub name: String,
    pub value: i64,
}

// Seven production tokens. Equipment uses its shared registry.
pub fn is_production_stat(name: &str) -> bool {
    matches!(
        name,
        "production_cost_factor"
            | "production_capacity_factor"
            | "production_efficiency_cap_factor"
            | "production_efficiency_gain_factor"
            | "production_resource_need_factor"
            | "production_resource_penalty_factor"
            | "production_conversion_speed_factor"
    )
}

fn equipment_bonus(bonus: &Bonus) -> bool {
    crate::equipment::stat_index(bonus.name.as_bytes()).is_some()
}

fn combine(destination: &mut Vec<Bonus>, source: impl IntoIterator<Item = Bonus>) {
    // Combine in first-insertion order with wrapping fixed5 addition.
    for bonus in source {
        if let Some(old) = destination.iter_mut().find(|old| old.name == bonus.name) {
            old.value = old.value.wrapping_add(bonus.value);
        } else {
            destination.push(bonus);
        }
    }
}

impl TraitName {
    // The caller supplies recorded lexer spellings and picks the first key
    // that its localisation catalogue defines; this is not scope expansion.
    pub fn name_candidates(
        &self,
        organisation: &str,
        trait_token: &str,
        owner: Option<&str>,
    ) -> Vec<String> {
        let base = if self.name.is_empty() {
            format!("{organisation}_{trait_token}")
        } else {
            self.name.clone()
        };
        let mut result = Vec::new();
        if let Some(owner) = owner {
            result.push(format!("{owner}_{base}"));
        }
        result.push(base);
        result
    }

    // Nonempty local values win; deletion suppresses only copying an unset
    // name. The inherited record's parent is the child.
    fn inherit(&mut self, parent: &Self) {
        if self.name.is_empty() && !self.deleted.contains(&Token::c_str(b"name")) {
            self.name.clone_from(&parent.name);
        }
        // A local bonus in a category replaces that whole inherited category;
        // deletion removes it too. The remaining parent values add.
        let equipment = self.bonuses.iter().any(equipment_bonus)
            || self.deleted.contains(&Token::c_str(b"equipment_bonus"));
        let production = self.bonuses.iter().any(|bonus| !equipment_bonus(bonus))
            || self.deleted.contains(&Token::c_str(b"production_bonus"));
        combine(
            &mut self.bonuses,
            parent
                .bonuses
                .iter()
                .filter(|bonus| {
                    if equipment_bonus(bonus) {
                        !equipment
                    } else {
                        !production
                    }
                })
                .cloned(),
        );
        if self.organisation_modifiers.values.is_empty()
            && self.organisation_modifiers.tooltip_table().is_empty()
            && self.organisation_modifiers.children.is_empty()
            && !self
                .deleted
                .contains(&Token::c_str(b"organization_modifier"))
        {
            self.organisation_modifiers
                .clone_from(&parent.organisation_modifiers);
            if parent.organisation_modifiers != modifiers::Modifiers::default() {
                self.modifier_inheritance_refusal = Some(
                    "MIO organization_modifier inheritance needs shared modifier tooltip-table copying".into(),
                );
            }
        } else if !self
            .deleted
            .contains(&Token::c_str(b"organization_modifier"))
            && self.organisation_modifiers.tooltip_table().is_empty()
            && self.organisation_modifiers.children.is_empty()
            && parent.organisation_modifiers != modifiers::Modifiers::default()
        {
            self.modifier_inheritance_refusal = Some(
                "MIO organization_modifier inheritance needs shared modifier effective-emptiness eligibility gate".into(),
            );
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Organisation {
    token: String,
    include: Option<String>,
    traits: Vec<TraitName>,
    overrides: Vec<TraitName>,
    removed: Vec<Token>,
}

#[derive(Default)]
pub struct Catalog {
    organisations: Vec<Organisation>,
    pub refusals: Vec<String>,
    pub warnings: Vec<String>,
}

impl Catalog {
    /// Scan the prepared ordinary-trait array from its first row.
    /// The separately stored initial trait is not in that lookup.
    pub fn trait_definition(
        &self,
        organisation: &str,
        token: &str,
    ) -> Result<Option<&TraitName>, String> {
        if !self.refusals.is_empty() {
            return Err(format!(
                "MIO trait {organisation}/{token}: {}",
                self.refusals.join("; ")
            ));
        }
        let organisation = Token::c_str(organisation.as_bytes());
        let token = Token::c_str(token.as_bytes());
        Ok(self
            .organisations
            .iter()
            .find(|row| Token::c_str(row.token.as_bytes()) == organisation)
            .and_then(|row| {
                row.traits
                    .iter()
                    .find(|row| Token::c_str(row.token.as_bytes()) == token)
            }))
    }

    // A repeated template resets in place without appending its pointer
    // twice: retain first creation order.
    fn insert(&mut self, row: Organisation) {
        let token = Token::c_str(row.token.as_bytes());
        if let Some(old) = self
            .organisations
            .iter_mut()
            .find(|old| Token::c_str(old.token.as_bytes()) == token)
        {
            *old = row;
        } else {
            self.organisations.push(row);
        }
    }

    // Database finalization walks creation order once, without a recursive
    // include walk, merging each parent's ordinary traits.
    fn finalize(&mut self) {
        for index in 0..self.organisations.len() {
            let Some(include) = self.organisations[index].include.as_deref() else {
                continue;
            };
            let token = Token::c_str(include.as_bytes());
            let Some(parent) = self
                .organisations
                .iter()
                .position(|row| Token::c_str(row.token.as_bytes()) == token)
            else {
                self.warnings.push(format!(
                    "MIO {} includes missing {include}",
                    self.organisations[index].token
                ));
                continue;
            };
            if parent == index {
                self.refusals
                    .push(format!("MIO {include} includes itself (count: 1)"));
                continue;
            }
            let traits = self.organisations[parent].traits.clone();
            let child = &mut self.organisations[index];
            for inherited in traits {
                let token = Token::c_str(inherited.token.as_bytes());
                if child.removed.contains(&token) {
                    continue;
                }
                if let Some(index) = child
                    .overrides
                    .iter()
                    .position(|row| Token::c_str(row.token.as_bytes()) == token)
                {
                    // A matched override appends even when a local trait has
                    // the same ID; the first-match lookup retains the local.
                    let mut row = child.overrides.remove(index);
                    row.inherit(&inherited);
                    child.traits.push(row);
                } else if let Some(row) = child
                    .traits
                    .iter_mut()
                    .find(|row| Token::c_str(row.token.as_bytes()) == token)
                {
                    row.inherit(&inherited);
                } else {
                    let mut row = inherited;
                    // Copying into a fresh child never copies the parent's
                    // `delete_included_values` list.
                    row.deleted.clear();
                    if row.organisation_modifiers != modifiers::Modifiers::default() {
                        row.modifier_inheritance_refusal = Some(
                            "MIO organization_modifier inheritance needs shared modifier tooltip-table copying".into(),
                        );
                    }
                    child.traits.push(row);
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Node {
    File,
    Organisation,
    Trait,
    Parent,
    Flavor,
    Mtth,
    Bonus,
    Modifier(modifiers::Node),
}

fn string(item: &Item, read_string: bool) -> io::Result<String> {
    let text = if read_string {
        item.value.read_string()
    } else {
        item.value.scalar().map(crate::parser::c_string)
    }
    .ok_or_else(|| invalid("MIO name or token is not a scalar (count: 1)"))?;
    String::from_utf8(text.to_vec())
        .map_err(|why| invalid(format!("MIO name or token: {why} (count: 1)")))
}

fn trait_name(items: &[Item]) -> io::Result<TraitName> {
    let mut result = TraitName::default();
    for field in items {
        if field.keyword_is("token") {
            result.token = string(field, false)?;
        } else if field.keyword_is("name") {
            result.name = string(field, true)?;
        } else if field.keyword_is("delete_included_values") {
            result.deleted.extend(
                field
                    .value
                    .items()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|item| item.value.scalar())
                    .map(Token::c_str),
            );
        } else if field.keyword_is("organization_modifier") {
            result
                .organisation_modifiers
                .read_all(field.value.items().unwrap_or_default());
        } else if field.keyword_is("equipment_bonus") || field.keyword_is("production_bonus") {
            // Reject a repeated stat in one block, then reject the entire
            // block if any stored stat has the wrong category.
            let mut bonuses = Vec::new();
            for item in field.value.items().unwrap_or_default() {
                let Some(name) = item.keyword().map(|name| name.folded()) else {
                    continue;
                };
                let name = String::from_utf8(name).map_err(|why| invalid(why.to_string()))?;
                if crate::equipment::stat_index(name.as_bytes()).is_none()
                    && !is_production_stat(&name)
                {
                    continue;
                }
                if bonuses.iter().any(|old: &Bonus| old.name == name) {
                    continue;
                }
                bonuses.push(Bonus {
                    name,
                    value: modifiers::stored(item),
                });
            }
            let equipment = field.keyword_is("equipment_bonus");
            if bonuses
                .iter()
                .all(|bonus| equipment_bonus(bonus) == equipment)
            {
                combine(&mut result.bonuses, bonuses);
            }
        } else if field.keyword_is("position") {
            // A position consumes seven raw tokens: x = n y = n }. As in
            // focus::parse, the shared raw-list reader is exact only for that
            // reached form; refuse malformed token recovery.
            let parts = field.value.items().unwrap_or_default();
            let valid = parts.len() == 6
                && parts[0]
                    .value
                    .scalar()
                    .is_some_and(|value| value.eq_ignore_ascii_case(b"x"))
                && parts[1].value.scalar_str() == Some("=")
                && parts[2].number()
                && parts[3]
                    .value
                    .scalar()
                    .is_some_and(|value| value.eq_ignore_ascii_case(b"y"))
                && parts[4].value.scalar_str() == Some("=")
                && parts[5].number();
            if !valid {
                return Err(invalid(
                    "MIO trait position: unsupported integer-point token recovery (count: 1)",
                ));
            }
        }
    }
    Ok(result)
}

fn parse(input: &[u8], names: &KeyNames) -> io::Result<(Vec<Organisation>, Vec<String>)> {
    let registry = |key: &[u8]| modifiers::registered(key, Some(names));
    let mut undecided = Vec::new();
    let read = native::read(
        input,
        native::Mode::KeepMark,
        // The file loop stops on either a closing brace or the end of text.
        native::Top::Persistent(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File => Shape::Object(Node::Organisation),
            // Consumed organisation and trait fields that do not affect names
            // still use their field readers, including variable references.
            Node::Organisation => match field.keyword().as_deref() {
                Some(
                    b"name" | b"background" | b"icon" | b"include" | b"research_bonus"
                    | b"task_capacity" | b"xp_research_type",
                ) => Shape::One,
                Some(b"equipment_type" | b"research_categories") => Shape::RawList,
                Some(b"remove_trait" | b"delete_included_values") => Shape::List,
                Some(b"trait" | b"add_trait" | b"override_trait" | b"initial_trait") => {
                    Shape::Object(Node::Trait)
                }
                Some(b"tree_header_text") => Shape::Object(Node::Flavor),
                Some(b"allowed" | b"available" | b"visible") => Shape::Trigger,
                Some(
                    b"on_design_team_assigned_to_tech"
                    | b"on_design_team_assigned_to_variant"
                    | b"on_industrial_manufacturer_assigned"
                    | b"on_tech_research_cancelled"
                    | b"on_tech_research_completed"
                    | b"on_industrial_manufacturer_unassigned",
                ) => Shape::EffectIn(0x200),
                Some(b"ai_will_do") => Shape::Object(Node::Mtth),
                _ => Shape::Skip,
            },
            Node::Trait => match field.keyword().as_deref() {
                Some(
                    b"name"
                    | b"icon"
                    | b"relative_position_id"
                    | b"special_trait_background"
                    | b"token",
                ) => Shape::One,
                Some(b"position") => Shape::RawList,
                Some(b"limit_to_equipment_type") => Shape::RawList,
                Some(b"parent") => Shape::ObjectOrSkip(Node::Parent),
                Some(
                    b"any_parent"
                    | b"all_parents"
                    | b"mutually_exclusive"
                    | b"delete_included_values",
                ) => Shape::List,
                Some(b"equipment_bonus" | b"production_bonus") => Shape::Object(Node::Bonus),
                Some(b"organization_modifier") => {
                    Shape::Object(Node::Modifier(modifiers::Node::Modifier))
                }
                Some(b"available" | b"visible") => Shape::Trigger,
                Some(b"on_complete") => Shape::EffectIn(0x200),
                Some(b"ai_will_do") => Shape::Object(Node::Mtth),
                _ => Shape::Skip,
            },
            Node::Parent => match field.keyword().as_deref() {
                Some(b"traits") => Shape::List,
                Some(b"num_parents_needed") => Shape::One,
                _ => Shape::Skip,
            },
            // Tree flavour text ignores unknown keys without an error skip.
            Node::Flavor => Shape::One,
            Node::Mtth => native::mean_time_to_happen(field),
            // Bonus fields consume known stats as fixed5;
            // unexpected fields have their blocks skipped.
            Node::Bonus => {
                if field.keyword().is_some_and(|name| {
                    crate::equipment::stat_index(&name).is_some()
                        || std::str::from_utf8(&name)
                            .ok()
                            .is_some_and(is_production_stat)
                }) {
                    Shape::One
                } else {
                    Shape::Skip
                }
            }
            Node::Modifier(node) => {
                modifiers::shape(node, field, &registry, &mut undecided).map(Node::Modifier)
            }
        },
    );
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!("MIO definitions: {cause} (count: 1)")));
    }
    if let Some(cause) = undecided.first() {
        return Err(invalid(format!("MIO definitions: {cause} (count: 1)")));
    }
    let mut rows = Vec::new();
    for item in read.items {
        let mut row = Organisation {
            token: String::from_utf8(
                crate::parser::c_string(item.key.as_deref().unwrap_or_default()).to_vec(),
            )
            .map_err(|why| invalid(why.to_string()))?,
            ..Default::default()
        };
        for field in item.value.items().unwrap_or_default() {
            if field.keyword_is("include") {
                row.include = Some(string(field, false)?);
            } else if field.keyword_is("trait") || field.keyword_is("add_trait") {
                row.traits
                    .push(trait_name(field.value.items().unwrap_or_default())?);
            } else if field.keyword_is("override_trait") {
                row.overrides
                    .push(trait_name(field.value.items().unwrap_or_default())?);
            } else if field.keyword_is("initial_trait") {
                // Assign the fixed `initial_trait` token and store it separately.
                // Validate its consumed fields too.
                let _ = trait_name(field.value.items().unwrap_or_default())?;
            } else if field.keyword_is("remove_trait") {
                row.removed.extend(
                    field
                        .value
                        .items()
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|item| item.value.scalar())
                        .map(Token::c_str),
                );
            }
        }
        rows.push(row);
    }
    Ok((
        rows,
        read.misreads
            .into_iter()
            .map(|why| why.to_string())
            .collect(),
    ))
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Catalog> {
    let names = layers.key_names()?;
    let mut result = Catalog::default();
    for path in script_files::paths(
        layers,
        "common/military_industrial_organization/organizations/",
        Some(".txt"),
    ) {
        match parse(&layers.read(&path)?, &names) {
            Ok((rows, warnings)) => {
                result
                    .warnings
                    .extend(warnings.into_iter().map(|why| format!("{path}: {why}")));
                for row in rows {
                    result.insert(row);
                }
            }
            Err(why) => result.refusals.push(format!("{path}: {why}")),
        }
    }
    result.finalize();
    result.warnings.extend(result.refusals.iter().cloned());
    Ok(result)
}

#[cfg(test)]
mod tests;
