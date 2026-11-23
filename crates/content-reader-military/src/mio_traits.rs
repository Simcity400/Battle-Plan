use content_reader_key_names::key_name_products::KeyNamesExt as _;

use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_parser::{fields::invalid, parser::Item, tokens::Token};
use content_reader_script::{key_names::KeyNames, modifiers, native::{self, Shape}};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::io;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraitName {
    pub token: String,
    pub name: String,
    pub icon: String,
    pub position: Option<[i32; 2]>,
    pub relative_position: Option<String>,
    pub parents: Vec<ParentGroup>,
    pub mutually_exclusive: Vec<String>,
    pub equipment_types: Vec<Vec<u8>>,
    pub special_background: bool,
    available: Vec<Item>,
    visible: Vec<Item>,
    inherited_available: Vec<Item>,
    inherited_visible: Vec<Item>,
    deleted: Vec<Token>,
    pub bonuses: Vec<Bonus>,
    pub organisation_modifiers: modifiers::Modifiers,
    modifier_parent: Option<Box<ModifierSource>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ModifierSource {
    local: modifiers::Modifiers,
    parent: Option<Box<ModifierSource>>,
}
impl ModifierSource {
    fn resolve(
        &self,
        eligible: &mut impl FnMut(&str) -> Result<bool, String>,
    ) -> Result<modifiers::Modifiers, String> {
        if let Some(parent) = &self.parent
            && self.local.effectively_empty(eligible)?
        {
            return Ok(parent.resolve(eligible)?.copied());
        }
        Ok(self.local.clone())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParentGroup {
    pub traits: Vec<String>,
    pub required: i32,
}

pub struct Tree<'a> {
    pub available: &'a [Item],
    pub visible: &'a [Item],
    pub equipment_types: &'a [Vec<u8>],
    pub research_categories: &'a [Vec<u8>],
    pub initial: Option<&'a TraitName>,
    pub traits: &'a [TraitName],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bonus {
    pub name: String,
    pub value: i64,
}

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
    content_reader_script::equipment_stats::stat_index(bonus.name.as_bytes()).is_some()
}

fn combine(destination: &mut Vec<Bonus>, source: impl IntoIterator<Item = Bonus>) {
    for bonus in source {
        if let Some(old) = destination.iter_mut().find(|old| old.name == bonus.name) {
            old.value = old.value.wrapping_add(bonus.value);
        } else {
            destination.push(bonus);
        }
    }
}

impl TraitName {
    pub fn resolved_organisation_modifiers(
        &self,
        mut eligible: impl FnMut(&str) -> Result<bool, String>,
    ) -> Result<Cow<'_, modifiers::Modifiers>, String> {
        if let Some(parent) = &self.modifier_parent
            && self
                .organisation_modifiers
                .effectively_empty(&mut eligible)?
        {
            return Ok(Cow::Owned(parent.resolve(&mut eligible)?.copied()));
        }
        Ok(Cow::Borrowed(&self.organisation_modifiers))
    }
    fn modifier_source(&self) -> ModifierSource {
        ModifierSource {
            local: self.organisation_modifiers.clone(),
            parent: self.modifier_parent.clone(),
        }
    }
    pub fn available(&self) -> &[Item] {
        if self.available.is_empty() {
            &self.inherited_available
        } else {
            &self.available
        }
    }

    pub fn visible(&self) -> &[Item] {
        if self.visible.is_empty() {
            &self.inherited_visible
        } else {
            &self.visible
        }
    }

    pub fn local_position(&self) -> [i32; 2] {
        self.position.unwrap_or([-1, -1])
    }

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

    fn inherit(&mut self, parent: &Self) {
        if self.name.is_empty() && !self.deleted.contains(&Token::c_str(b"name")) {
            self.name.clone_from(&parent.name);
        }
        let copy = |empty: bool, name: &[u8]| empty && !self.deleted.contains(&Token::c_str(name));
        if copy(self.icon.is_empty(), b"icon") {
            self.icon.clone_from(&parent.icon);
        }
        if copy(!self.special_background, b"special_trait_background") {
            self.special_background = parent.special_background;
        }
        if copy(self.equipment_types.is_empty(), b"limit_to_equipment_type") {
            self.equipment_types.clone_from(&parent.equipment_types);
        }
        if copy(self.available.is_empty(), b"available") {
            self.inherited_available.clone_from(&parent.available);
        }
        if copy(self.visible.is_empty(), b"visible") {
            self.inherited_visible.clone_from(&parent.visible);
        }
        if copy(self.parents.is_empty(), b"parent") {
            self.parents.clone_from(&parent.parents);
        }
        if copy(self.mutually_exclusive.is_empty(), b"mutually_exclusive") {
            self.mutually_exclusive
                .clone_from(&parent.mutually_exclusive);
        }
        if copy(self.local_position() == [-1, -1], b"position") {
            self.position = parent.position;
        }
        if copy(self.relative_position.is_none(), b"relative_position_id") {
            self.relative_position.clone_from(&parent.relative_position);
        }
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
        if !self
            .deleted
            .contains(&Token::c_str(b"organization_modifier"))
        {
            self.modifier_parent = Some(Box::new(parent.modifier_source()));
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Organisation {
    token: String,
    include: Option<String>,
    available: Vec<Item>,
    visible: Vec<Item>,
    inherited_available: Vec<Item>,
    inherited_visible: Vec<Item>,
    equipment_types: Vec<Vec<u8>>,
    research_categories: Vec<Vec<u8>>,
    deleted: Vec<Token>,
    traits: Vec<TraitName>,
    initial: Option<TraitName>,
    delete_initial: bool,
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
    pub fn tree(&self, organisation: &str) -> Result<Option<Tree<'_>>, String> {
        if !self.refusals.is_empty() {
            return Err(self.refusals.join("; "));
        }
        let token = Token::c_str(organisation.as_bytes());
        Ok(self
            .organisations
            .iter()
            .find(|row| Token::c_str(row.token.as_bytes()) == token)
            .map(|row| Tree {
                available: if row.available.is_empty() {
                    &row.inherited_available
                } else {
                    &row.available
                },
                visible: if row.visible.is_empty() {
                    &row.inherited_visible
                } else {
                    &row.visible
                },
                equipment_types: &row.equipment_types,
                research_categories: &row.research_categories,
                initial: row.initial.as_ref(),
                traits: &row.traits,
            }))
    }

    pub fn position(&self, organisation: &str, token: &str) -> Result<Option<[i32; 2]>, String> {
        let Some(tree) = self.tree(organisation)? else {
            return Ok(None);
        };
        let Some(mut index) = tree
            .traits
            .iter()
            .position(|row| Token::c_str(row.token.as_bytes()) == Token::c_str(token.as_bytes()))
        else {
            return Ok(None);
        };
        let mut visited = Vec::new();
        let mut offsets = Vec::new();
        let mut result;
        loop {
            if visited.contains(&index) {
                return Err(format!(
                    "MIO {organisation} trait {token}: relative-position cycle (count: 1)"
                ));
            }
            visited.push(index);
            let row = &tree.traits[index];
            let Some(anchor) = &row.relative_position else {
                result = row.local_position();
                break;
            };
            let Some(next) = tree.traits.iter().position(|row| {
                Token::c_str(row.token.as_bytes()) == Token::c_str(anchor.as_bytes())
            }) else {
                result = [0, 0];
                break;
            };
            offsets.push(row.local_position());
            index = next;
        }
        for offset in offsets.into_iter().rev() {
            result = [
                result[0].wrapping_add(offset[0]),
                result[1].wrapping_add(offset[1]),
            ];
        }
        Ok(Some(result))
    }

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

    fn insert(&mut self, row: Organisation) {
        let token = Token::c_str(row.token.as_bytes());
        if !self
            .organisations
            .iter()
            .any(|old| Token::c_str(old.token.as_bytes()) == token)
        {
            self.organisations.push(row);
        }
    }

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
            let initial = self.organisations[parent].initial.clone();
            let available = self.organisations[parent].available.clone();
            let visible = self.organisations[parent].visible.clone();
            let equipment_types = self.organisations[parent].equipment_types.clone();
            let research_categories = self.organisations[parent].research_categories.clone();
            let child = &mut self.organisations[index];
            if child.available.is_empty() && !child.deleted.contains(&Token::c_str(b"available")) {
                child.inherited_available = available;
            }
            if child.visible.is_empty() && !child.deleted.contains(&Token::c_str(b"visible")) {
                child.inherited_visible = visible;
            }
            if child.equipment_types.is_empty()
                && !child.deleted.contains(&Token::c_str(b"equipment_type"))
            {
                child.equipment_types = equipment_types;
            }
            if child.research_categories.is_empty()
                && !child
                    .deleted
                    .contains(&Token::c_str(b"research_categories"))
            {
                child.research_categories = research_categories;
            }
            if !child.delete_initial
                && let Some(initial) = initial
            {
                if let Some(local) = &mut child.initial {
                    local.inherit(&initial);
                } else {
                    let mut initial = initial;
                    let source = initial.modifier_source();
                    initial.organisation_modifiers = Default::default();
                    initial.modifier_parent = Some(Box::new(source));
                    initial.deleted.clear();
                    child.initial = Some(initial);
                }
            }
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
                    row.deleted.clear();
                    row.inherited_available = std::mem::take(&mut row.available);
                    row.inherited_visible = std::mem::take(&mut row.visible);
                    let source = row.modifier_source();
                    row.organisation_modifiers = Default::default();
                    row.modifier_parent = Some(Box::new(source));
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
        item.value.scalar().map(content_reader_parser::parser::c_string)
    }
    .ok_or_else(|| invalid("MIO name or token is not a scalar (count: 1)"))?;
    String::from_utf8(text.to_vec())
        .map_err(|why| invalid(format!("MIO name or token: {why} (count: 1)")))
}

fn raw_tokens(field: &Item) -> Vec<Vec<u8>> {
    field
        .value
        .items()
        .unwrap_or_default()
        .iter()
        .filter_map(|item| item.value.scalar())
        .map(content_reader_parser::parser::c_string)
        .map(<[u8]>::to_vec)
        .collect()
}

fn token_strings(items: &[Item]) -> io::Result<Vec<String>> {
    items.iter().map(|item| string(item, false)).collect()
}

fn trait_name(items: &[Item]) -> io::Result<TraitName> {
    let mut result = TraitName::default();
    for field in items {
        if field.keyword_is("token") {
            result.token = string(field, false)?;
        } else if field.keyword_is("name") {
            result.name = string(field, true)?;
        } else if field.keyword_is("icon") {
            result.icon = string(field, true)?;
        } else if field.keyword_is("relative_position_id") {
            result.relative_position = Some(string(field, false)?);
        } else if field.keyword_is("special_trait_background") {
            result.special_background =
                content_reader_parser::fields::boolean(&field.value, "MIO trait background")?;
        } else if field.keyword_is("limit_to_equipment_type") {
            result.equipment_types = field
                .value
                .items()
                .unwrap_or_default()
                .iter()
                .map(|item| {
                    item.value
                        .scalar()
                        .map(content_reader_parser::parser::c_string)
                        .map(<[u8]>::to_vec)
                        .ok_or_else(|| invalid("MIO equipment filter is not a scalar (count: 1)"))
                })
                .collect::<io::Result<_>>()?;
        } else if field.keyword_is("mutually_exclusive") {
            result
                .mutually_exclusive
                .extend(token_strings(field.value.items().unwrap_or_default())?);
        } else if field.keyword_is("parent") {
            let mut group = ParentGroup {
                required: 1,
                ..Default::default()
            };
            for value in field.value.items().unwrap_or_default() {
                if value.keyword_is("traits") {
                    group
                        .traits
                        .extend(token_strings(value.value.items().unwrap_or_default())?);
                } else if value.keyword_is("num_parents_needed")
                    && let Some(value) = value
                        .value
                        .scalar()
                        .and_then(|value| content_reader_parser::crt::scan_i32(value).stored())
                {
                    group.required = value;
                }
            }
            result.parents.push(group);
        } else if field.keyword_is("any_parent") || field.keyword_is("all_parents") {
            let traits = token_strings(field.value.items().unwrap_or_default())?;
            let required = if field.keyword_is("all_parents") {
                i32::try_from(traits.len())
                    .map_err(|_| invalid("MIO parent count overflow (count: 1)"))?
            } else {
                1
            };
            result.parents.push(ParentGroup { traits, required });
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
        } else if field.keyword_is("available") {
            result
                .available
                .extend_from_slice(field.value.items().unwrap_or_default());
        } else if field.keyword_is("visible") {
            result
                .visible
                .extend_from_slice(field.value.items().unwrap_or_default());
        } else if field.keyword_is("organization_modifier") {
            result
                .organisation_modifiers
                .read_all(field.value.items().unwrap_or_default());
        } else if field.keyword_is("equipment_bonus") || field.keyword_is("production_bonus") {
            let mut bonuses = Vec::new();
            for item in field.value.items().unwrap_or_default() {
                let Some(name) = item.keyword().map(|name| name.folded()) else {
                    continue;
                };
                let name = String::from_utf8(name).map_err(|why| invalid(why.to_string()))?;
                if content_reader_script::equipment_stats::stat_index(name.as_bytes()).is_none()
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
            let parts = field.value.items().unwrap_or_default();
            let valid =
                parts.len() == 6
                    && parts[0].value.scalar().is_some_and(|value| {
                        content_reader_parser::parser::c_string(value).eq_ignore_ascii_case(b"x")
                    })
                    && parts[1].value.scalar_str() == Some("=")
                    && parts[2].number()
                    && parts[3].value.scalar().is_some_and(|value| {
                        content_reader_parser::parser::c_string(value).eq_ignore_ascii_case(b"y")
                    })
                    && parts[4].value.scalar_str() == Some("=")
                    && parts[5].number();
            if !valid {
                return Err(invalid(
                    "MIO trait position: unsupported integer-point token recovery (count: 1)",
                ));
            }
            result.position = Some([2, 5].map(|index| {
                content_reader_parser::crt::scan_i32(parts[index].value.scalar().unwrap())
                    .stored()
                    .unwrap_or(-1)
            }));
        }
    }
    Ok(result)
}

fn parse(
    input: &[u8],
    names: &KeyNames,
    seen: &mut BTreeSet<Token>,
) -> io::Result<(Vec<Organisation>, Vec<String>)> {
    let registry = |key: &[u8]| modifiers::registered(key, Some(names));
    let mut undecided = Vec::new();
    let earlier = seen.clone();
    let mut duplicates = Vec::new();
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::Persistent(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File => {
                if seen.insert(Token::c_str(field.key.c_str())) {
                    Shape::Object(Node::Organisation)
                } else {
                    duplicates.push(format!(
                        "MIO {} at line {}: duplicate definition skipped (count: 1)",
                        String::from_utf8_lossy(field.key.c_str()),
                        field.key.line
                    ));
                    Shape::Skip
                }
            }
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
            Node::Flavor => Shape::One,
            Node::Mtth => native::mean_time_to_happen(field),
            Node::Bonus => {
                if field.keyword().is_some_and(|name| {
                    content_reader_script::equipment_stats::stat_index(&name).is_some()
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
    let mut kept = BTreeSet::new();
    for item in read.items {
        let token = Token::c_str(item.key.as_deref().unwrap_or_default());
        if earlier.contains(&token) || !kept.insert(token) {
            continue;
        }
        let mut row = Organisation {
            token: String::from_utf8(
                content_reader_parser::parser::c_string(item.key.as_deref().unwrap_or_default()).to_vec(),
            )
            .map_err(|why| invalid(why.to_string()))?,
            ..Default::default()
        };
        for field in item.value.items().unwrap_or_default() {
            if field.keyword_is("available") {
                row.available
                    .extend_from_slice(field.value.items().unwrap_or_default());
            } else if field.keyword_is("visible") {
                row.visible
                    .extend_from_slice(field.value.items().unwrap_or_default());
            } else if field.keyword_is("equipment_type") {
                row.equipment_types = raw_tokens(field);
            } else if field.keyword_is("research_categories") {
                row.research_categories.extend(raw_tokens(field));
            } else if field.keyword_is("include") {
                row.include = Some(string(field, false)?);
            } else if field.keyword_is("trait") || field.keyword_is("add_trait") {
                row.traits
                    .push(trait_name(field.value.items().unwrap_or_default())?);
            } else if field.keyword_is("override_trait") {
                row.overrides
                    .push(trait_name(field.value.items().unwrap_or_default())?);
            } else if field.keyword_is("initial_trait") {
                let mut initial = trait_name(field.value.items().unwrap_or_default())?;
                initial.token = "initial_trait".into();
                row.initial = Some(initial);
            } else if field.keyword_is("delete_included_values") {
                row.deleted.extend(
                    field
                        .value
                        .items()
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|item| item.value.scalar())
                        .map(Token::c_str),
                );
                row.delete_initial |= field.value.items().unwrap_or_default().iter().any(|item| {
                    item.value.scalar().is_some_and(|name| {
                        content_reader_parser::parser::c_string(name).eq_ignore_ascii_case(b"initial_trait")
                    })
                });
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
            .chain(duplicates)
            .collect(),
    ))
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Catalog> {
    let names = layers.key_names()?;
    let mut result = Catalog::default();
    let mut seen = BTreeSet::new();
    for path in script_files::paths(
        layers,
        "common/military_industrial_organization/organizations/",
        Some(".txt"),
    )? {
        match parse(&path.read(layers)?, &names, &mut seen) {
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

pub fn cached(layers: &ResolvedLayers) -> Result<std::sync::Arc<Catalog>, String> {
    #[derive(Default)]
    struct Cached(content_reader_layers::content::diagnostics::Once<Result<std::sync::Arc<Catalog>, String>>);
    layers
        .derived::<Cached>()
        .0
        .get_or_init(|| {
            read(layers)
                .map(std::sync::Arc::new)
                .map_err(|why| why.to_string())
        })
        .clone()
}

#[cfg(test)]
mod tests;
