use content_reader_key_names::key_name_products::KeyNamesExt as _;
use content_reader_script::idea_fields::{RuleNode, ledger, ledger_shape, rule_field};

use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_key_names::characters::{self, Characters};
use content_reader_parser::{coverage::{Coverage, Gap}, crt::{Scanned, scan_i32}, parser::{Item, Kind, Unheld}, tokens::Token};
use content_reader_script::{key_names::{KeyNames, Point}, modifiers::{self, EquipmentBonus, Modifier, Modifiers, TargetedModifier}, native::{self, Field, Shape}, scope::ScopeKind};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IdeaCategory {
    pub name: String,
    pub slots: Vec<String>,
    pub character_slots: Vec<String>,
    pub kind: Option<String>,
    pub hidden: bool,
    pub politics_tab: bool,
    pub ledger: u32,
    pub cost: Option<f64>,
    pub removal_cost: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IdeaTags {
    categories: Vec<IdeaCategory>,
    cut: Option<(usize, Gap)>,
}

impl IdeaTags {
    pub fn resolve(&self, name: &str) -> Result<Option<&IdeaCategory>, &Gap> {
        for (index, category) in self.categories.iter().enumerate() {
            if let Some((known, gap)) = &self.cut
                && index >= *known
            {
                return Err(gap);
            }
            if category.name == name
                || category.slots.iter().any(|slot| slot == name)
                || category.character_slots.iter().any(|slot| slot == name)
            {
                return Ok(Some(category));
            }
        }
        match &self.cut {
            Some((_, gap)) => Err(gap),
            None => Ok(None),
        }
    }

    pub fn categories(&self) -> (&[IdeaCategory], Option<&Gap>) {
        (&self.categories, self.cut.as_ref().map(|(_, gap)| gap))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Idea {
    pub token: String,
    pub group: String,
    pub law: bool,
    pub designer: bool,
    pub default: bool,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub modifiers: Modifiers,
    pub targeted: Vec<TargetedModifier>,
    pub equipment_bonuses: Vec<EquipmentBonus>,
    pub research_bonuses: Vec<Modifier>,
    pub traits: Vec<String>,
    pub rules: IdeaRules,
    pub cost: Option<f64>,
    pub removal_cost: Option<f64>,
    pub ledger: u32,
    pub level: i32,
    pub position: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdeaRules {
    pub values: Vec<bool>,
    pub description: String,
    pub overrides: BTreeMap<usize, String>,
}

impl Default for IdeaRules {
    fn default() -> Self {
        Self {
            values: crate::power_balance::rules()
                .iter()
                .map(|rule| rule.2)
                .collect(),
            description: String::new(),
            overrides: BTreeMap::new(),
        }
    }
}

impl IdeaRules {
    fn read_all(&mut self, fields: &[Item]) -> Result<(), String> {
        for field in fields {
            if field.keyword_is("desc") {
                self.description = field
                    .value
                    .read_str()
                    .map_err(|why| why.to_string())?
                    .to_owned();
            } else if field.keyword_is("override") {
                for entry in field.value.items().unwrap_or_default() {
                    let raw = content_reader_parser::fixed5::decimal_i32(entry.key.as_deref().unwrap_or_default());
                    let index = usize::try_from(raw)
                        .ok()
                        .filter(|&index| index < self.values.len())
                        .ok_or_else(|| {
                            format!("rule override index {raw} writes outside rule storage")
                        })?;
                    self.overrides.insert(
                        index,
                        entry
                            .value
                            .c_str()
                            .map_err(|why| why.to_string())?
                            .to_owned(),
                    );
                }
            } else if let Some(index) = crate::power_balance::rules()
                .iter()
                .position(|rule| field.keyword_is(rule.0))
            {
                self.values[index] = field
                    .value
                    .scalar()
                    .and_then(content_reader_parser::parser::read_bool)
                    .unwrap_or(false);
            }
        }
        Ok(())
    }
}

impl Idea {
    pub fn sprite(&self) -> String {
        format!(
            "GFX_idea_{}",
            self.picture.as_deref().unwrap_or(&self.token)
        )
    }
    pub fn sprite_candidates(&self, tag: Option<&str>) -> Vec<String> {
        let own = format!("GFX_idea_{}", self.token);
        let picture = self.sprite();
        let mut candidates = if picture == own {
            vec![own]
        } else {
            vec![own, picture.clone()]
        };
        if let Some(tag) = tag {
            candidates.push(format!("{picture}_{tag}"));
        }
        candidates
    }
}

fn bool_token(item: &Item) -> Option<bool> {
    item.value.scalar().and_then(content_reader_parser::parser::read_bool)
}

fn fixed5_number(item: &Item) -> Option<f64> {
    modifiers::read_fixed5(item).map(|raw| raw as f64 / 100_000.0)
}

fn int32(item: &Item) -> Option<f64> {
    match scan_i32(item.value.scalar()?) {
        Scanned::Stored(value) => Some(f64::from(value)),
        Scanned::End | Scanned::NoMatch => None,
    }
}

fn keyword(item: &Item) -> Option<Vec<u8>> {
    item.keyword().map(|keyword| keyword.folded())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Loader {
    Tags,
    Ideas,
}

fn identity(name: &[u8]) -> Option<Vec<u8>> {
    (!matches!(name.first(), Some(b'0'..=b'9' | b'-'))).then(|| name.to_ascii_lowercase())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Path {
    New { officers: bool },
    Again,
}

#[derive(Clone, Debug)]
struct Groups<'a> {
    loader: Loader,
    officers: &'a [String],
    known: BTreeSet<Vec<u8>>,
    unknown: Option<String>,
}

impl Groups<'_> {
    fn path(&mut self, key: &[u8], quoted: bool) -> Result<Path, String> {
        let identity = identity(key);
        let held = identity.as_ref().is_some_and(|id| self.known.contains(id));
        if !held && let Some(unknown) = &self.unknown {
            return Err(format!(
                "group {} may be one the unknown rest of an earlier file registers ({unknown}), which changes how it is read",
                key.escape_ascii()
            ));
        }
        let path = if self.loader == Loader::Ideas && held && content_reader_parser::parser::looked_up(key, quoted)
        {
            Path::Again
        } else {
            Path::New {
                officers: self.officers.iter().any(|name| name.as_bytes() == key),
            }
        };
        self.known.extend(identity);
        Ok(path)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Top,
    Categories,
    Category,
    Ledgers,
    Groups,
    Group(Path),
    Idea,
    Advisor,
    Rule(RuleNode),
    Mtth,
    Modifier(modifiers::Node),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stop {
    ideas: usize,
    categories: usize,
    cause: String,
}

impl Stop {
    fn first(stop: Option<Self>, other: Self) -> Self {
        match stop {
            Some(stop) if (stop.ideas, stop.categories) <= (other.ideas, other.categories) => stop,
            _ => other,
        }
    }
}

struct Reading<'a> {
    groups: Groups<'a>,
    ideas: usize,
    categories: usize,
    group_start: usize,
    undecided: Vec<modifiers::Undecided>,
    stop: Option<Stop>,
}

impl Reading<'_> {
    fn stop(&mut self, cause: String) {
        self.stop.get_or_insert(Stop {
            ideas: self.group_start,
            categories: self.categories,
            cause,
        });
    }

    fn shape(&mut self, node: Node, field: &Field<'_>) -> Shape<Node> {
        let keyword = field.keyword();
        let keyword = keyword.as_deref();
        let shape = match node {
            Node::Top => match keyword {
                Some(b"idea_categories") => Shape::Object(Node::Categories),
                Some(b"ideas") => Shape::Object(Node::Groups),
                Some(b"slot_ledgers") if self.groups.loader == Loader::Tags => {
                    Shape::Object(Node::Ledgers)
                }
                _ => Shape::Skip,
            },
            Node::Categories => {
                self.categories += 1;
                Shape::Object(Node::Category)
            }
            Node::Category => match keyword {
                Some(
                    b"slot" | b"character_slot" | b"politics_tab" | b"hidden" | b"cost"
                    | b"removal_cost" | b"type",
                ) => Shape::One,
                Some(b"ledger") => ledger_shape(field),
                _ => Shape::Skip,
            },
            Node::Ledgers => ledger_shape(field),
            Node::Groups => {
                self.group_start = self.ideas;
                match self
                    .groups
                    .path(&field.key.text, field.key.kind == Kind::Quoted)
                {
                    Ok(path) => Shape::Object(Node::Group(path)),
                    Err(cause) => {
                        self.stop(cause);
                        Shape::Object(Node::Group(Path::New { officers: false }))
                    }
                }
            }
            Node::Group(Path::New { officers }) => match keyword {
                Some(b"law" | b"designer" | b"use_list_view") => Shape::One,
                _ if field.opens() => {
                    self.ideas += 1;
                    Shape::Object(if officers { Node::Advisor } else { Node::Idea })
                }
                _ => Shape::Skip,
            },
            Node::Group(Path::Again) => match keyword {
                Some(b"law" | b"designer") => Shape::One,
                _ => {
                    self.ideas += 1;
                    Shape::Object(Node::Idea)
                }
            },
            Node::Idea => match keyword {
                Some(
                    b"available" | b"cancel" | b"visible" | b"allowed" | b"allowed_civil_war"
                    | b"allowed_to_remove" | b"do_effect",
                ) => Shape::Trigger,
                Some(b"on_add" | b"on_remove") => Shape::EffectIn(ScopeKind::Country.bit()),
                Some(
                    b"cost" | b"removal_cost" | b"name" | b"picture" | b"level" | b"default"
                    | b"cancel_if_invalid" | b"is_good" | b"category",
                ) => Shape::One,
                Some(b"ledger") => ledger_shape(field),
                Some(b"modifier") => Shape::Object(Node::Modifier(modifiers::Node::Modifier)),
                Some(b"ai_will_do") => Shape::Object(Node::Mtth),
                Some(b"rule") => Shape::Object(Node::Rule(RuleNode::Rule)),
                Some(b"targeted_modifier") => {
                    Shape::Object(Node::Modifier(modifiers::Node::Targeted))
                }
                Some(b"research_bonus") => {
                    Shape::Object(Node::Modifier(modifiers::Node::ResearchBonus))
                }
                Some(b"equipment_bonus") => {
                    Shape::Object(Node::Modifier(modifiers::Node::EquipmentBonuses))
                }
                Some(b"traits") => Shape::List,
                _ => Shape::Skip,
            },
            Node::Advisor => characters::advisor_field(field).map(|role| match role {
                characters::Role::Modifier(modifier) => Node::Modifier(modifier),
                characters::Role::Mtth => Node::Mtth,
            }),
            Node::Rule(rule) => rule_field(rule, field).map(Node::Rule),
            Node::Mtth => native::mean_time_to_happen(field),
            Node::Modifier(modifier) => {
                modifiers::shape(modifier, field, modifiers::UNKNOWN, &mut self.undecided)
                    .map(Node::Modifier)
            }
        };
        if self.stop.is_none()
            && let Some(first) = self.undecided.first()
        {
            let cause = first.to_string();
            self.stop(cause);
        }
        shape
    }
}

fn read_category(name: &Item) -> Result<IdeaCategory, String> {
    let mut unheld = Unheld::default();
    let mut category = IdeaCategory {
        name: unheld
            .keep("a category's name", name.key_str())
            .unwrap_or_default()
            .to_owned(),
        politics_tab: true,
        ..IdeaCategory::default()
    };
    for field in name.value.items().unwrap_or_default() {
        match keyword(field).as_deref() {
            Some(b"slot") => category.slots.extend(
                unheld
                    .keep("a slot", field.value.c_str())
                    .map(str::to_owned),
            ),
            Some(b"character_slot") => category.character_slots.extend(
                unheld
                    .keep("a character slot", field.value.c_str())
                    .map(str::to_owned),
            ),
            Some(b"type") => {
                category.kind = field.value_keyword().filter(|kind| {
                    matches!(
                        kind.as_str(),
                        "national_spirit" | "army_spirit" | "navy_spirit" | "air_spirit"
                    )
                });
            }
            Some(b"hidden") => category.hidden = bool_token(field).unwrap_or(category.hidden),
            Some(b"politics_tab") => {
                category.politics_tab = bool_token(field).unwrap_or(category.politics_tab);
            }
            Some(b"ledger") => category.ledger = ledger(category.ledger, field),
            Some(b"cost") => category.cost = int32(field).or(category.cost),
            Some(b"removal_cost") => {
                category.removal_cost = int32(field).or(category.removal_cost);
            }
            _ => {}
        }
    }
    match unheld.cause() {
        Some(cause) => Err(format!("category {}: {cause}", category.name)),
        None => Ok(category),
    }
}

fn read_idea(token: &Item, group: &str, fields: &[Item]) -> Result<Idea, String> {
    let mut unheld = Unheld::default();
    let mut idea = Idea {
        token: unheld
            .keep("an idea's token", token.key_str())
            .unwrap_or_default()
            .to_owned(),
        group: group.to_owned(),
        level: -1,
        ..Idea::default()
    };
    for field in fields {
        match keyword(field).as_deref() {
            Some(b"default") => {
                idea.default = bool_token(field).unwrap_or(idea.default);
            }
            Some(b"name") => {
                idea.name = unheld
                    .keep("its name", field.value.read_str())
                    .map(str::to_owned);
            }
            Some(b"picture") => {
                idea.picture = unheld
                    .keep("its picture", field.value.read_str())
                    .map(str::to_owned);
            }
            Some(b"cost") => idea.cost = fixed5_number(field).or(idea.cost),
            Some(b"removal_cost") => idea.removal_cost = fixed5_number(field).or(idea.removal_cost),
            Some(b"level") => {
                if let Some(Scanned::Stored(level)) = field.value.scalar().map(scan_i32) {
                    idea.level = level;
                }
            }
            Some(b"ledger") => idea.ledger = ledger(idea.ledger, field),
            Some(b"modifier") => idea
                .modifiers
                .read_all(field.value.items().unwrap_or_default()),
            Some(b"targeted_modifier") => {
                let targeted = modifiers::targeted(field.value.items().unwrap_or_default());
                unheld.absorb(&targeted.modifiers.unheld);
                idea.targeted.push(targeted);
            }
            Some(b"equipment_bonus") => idea.equipment_bonuses.extend(
                modifiers::equipment_bonuses(field.value.items().unwrap_or_default(), &mut unheld),
            ),
            Some(b"research_bonus") => idea.research_bonuses.extend(modifiers::research_bonuses(
                field.value.items().unwrap_or_default(),
                &mut unheld,
            )),
            Some(b"traits") => {
                for entry in field.value.items().unwrap_or_default() {
                    idea.traits.extend(
                        unheld
                            .keep("a trait", entry.value.read_str())
                            .map(str::to_owned),
                    );
                }
            }
            Some(b"rule") => idea
                .rules
                .read_all(field.value.items().unwrap_or_default())
                .map_err(|why| format!("idea {}: {why}", idea.token))?,
            _ => {}
        }
    }
    unheld.absorb(&idea.modifiers.unheld);
    match unheld.cause() {
        Some(cause) => Err(format!("idea {}: {cause}", idea.token)),
        None => Ok(idea),
    }
}

struct Occurrence {
    name: String,
    identity: Option<Vec<u8>>,
    path: Path,
    law: Option<bool>,
    designer: Option<bool>,
    start: usize,
    ideas: Vec<Idea>,
    advisors: Vec<Characters>,
}

enum Declared {
    Category(IdeaCategory),
    Group(Occurrence),
}

struct FileRead {
    declared: Vec<Declared>,
    refused_tokens: Vec<String>,
    stop: Option<Stop>,
    notes: Vec<content_reader_parser::Diagnostic>,
}

fn read_file(input: &[u8], groups: Groups<'_>, names: &KeyNames) -> FileRead {
    let mut reading = Reading {
        groups: groups.clone(),
        ideas: 0,
        categories: 0,
        group_start: 0,
        undecided: Vec::new(),
        stop: None,
    };
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::Top),
        Some(names),
        &mut |node, field| reading.shape(node, field),
    );
    let mut stop = reading.stop.or_else(|| {
        read.unsettled.as_ref().map(|unsettled| Stop {
            ideas: reading.group_start,
            categories: reading.categories,
            cause: format!("{unsettled}; any idea the rest of the file defines is unknown"),
        })
    });
    let mut groups = groups;
    let (mut ideas, mut categories) = (0, 0);
    let mut declared = Vec::new();
    let mut tokens = Vec::new();
    let mut scans = modifiers::ScanFailures::default();
    for root in &read.items {
        match keyword(root).as_deref() {
            Some(b"idea_categories") => {
                for item in root.value.items().unwrap_or_default() {
                    for field in item.value.items().unwrap_or_default() {
                        if field.keyword_is("cost") || field.keyword_is("removal_cost") {
                            scans.int32(item.key_str().unwrap_or("<non-text key>"), field);
                        }
                    }
                    match read_category(item) {
                        Ok(category) => declared.push(Declared::Category(category)),
                        Err(cause) => {
                            stop = Some(Stop::first(
                                stop,
                                Stop {
                                    ideas,
                                    categories,
                                    cause,
                                },
                            ));
                        }
                    }
                    categories += 1;
                }
            }
            Some(b"ideas") => {
                for group in root.value.items().unwrap_or_default() {
                    let key = group.key.as_deref().unwrap_or_default();
                    let path = groups
                        .path(key, group.key_quoted)
                        .unwrap_or(Path::New { officers: false });
                    let mut occurrence = Occurrence {
                        name: String::new(),
                        identity: identity(key),
                        path,
                        law: None,
                        designer: None,
                        start: ideas,
                        ideas: Vec::new(),
                        advisors: Vec::new(),
                    };
                    let mut failed = match group.key_str() {
                        Ok(name) => {
                            occurrence.name = name.to_owned();
                            None
                        }
                        Err(refused) => Some(format!("a group's name is {refused}")),
                    };
                    for entry in group.value.items().unwrap_or_default() {
                        let is = |name: &str| entry.keyword_is(name);
                        match path {
                            _ if is("law") => {
                                occurrence.law = bool_token(entry).or(occurrence.law);
                                continue;
                            }
                            _ if is("designer") => {
                                occurrence.designer = bool_token(entry).or(occurrence.designer);
                                continue;
                            }
                            Path::New { .. } if is("use_list_view") => continue,
                            Path::New { .. } if entry.kind != Kind::Open => continue,
                            _ => {}
                        }
                        tokens.push(match entry.key_str() {
                            Ok(token) => token.to_owned(),
                            Err(_) => entry
                                .key
                                .as_deref()
                                .unwrap_or_default()
                                .escape_ascii()
                                .to_string(),
                        });
                        ideas += 1;
                        let body = entry.value.items().unwrap_or_default();
                        for field in body {
                            if field.keyword_is("cost") || field.keyword_is("removal_cost") {
                                scans.fixed5(entry.key_str().unwrap_or("<non-text key>"), field);
                            }
                        }
                        let read = match path {
                            Path::New { officers: true } => entry
                                .key_str()
                                .map_err(|refused| format!("an advisor's token is {refused}"))
                                .and_then(|token| {
                                    characters::officer_template(token, &occurrence.name, body)
                                })
                                .map(|advisor| occurrence.advisors.push(advisor)),
                            _ => read_idea(entry, &occurrence.name, body).map(|idea| {
                                scans.idea_modifiers(&idea.token, &idea.modifiers, names);
                                for group in &idea.targeted {
                                    scans.idea_modifiers(&idea.token, &group.modifiers, names);
                                }
                                for bonus in &idea.equipment_bonuses {
                                    scans.scan(&idea.token, &bonus.values);
                                }
                                scans.scan(&idea.token, &idea.research_bonuses);
                                occurrence.ideas.push(idea);
                            }),
                        };
                        if let Err(cause) = read {
                            failed.get_or_insert(cause);
                        }
                    }
                    if let Some(cause) = failed {
                        stop = Some(Stop::first(
                            stop,
                            Stop {
                                ideas: occurrence.start,
                                categories,
                                cause,
                            },
                        ));
                    }
                    declared.push(Declared::Group(occurrence));
                }
            }
            _ => {}
        }
    }
    let refused_tokens = match &stop {
        Some(stop) => tokens.split_off(stop.ideas.min(tokens.len())),
        None => Vec::new(),
    };
    if let Some(stop) = &stop {
        let mut categories = 0;
        declared.retain(|declared| match declared {
            Declared::Category(_) => {
                categories += 1;
                categories <= stop.categories
            }
            Declared::Group(occurrence) => {
                occurrence.start < stop.ideas
                    && occurrence.start + occurrence.ideas.len() + occurrence.advisors.len()
                        <= stop.ideas
            }
        });
    }
    let scan_notes = scans.warning();
    FileRead {
        declared,
        refused_tokens,
        stop,
        notes: read
            .misreads
            .iter()
            .map(|note| (note.kind, note.to_string()))
            .chain(scan_notes.map(|note| (content_reader_parser::DiagnosticKind::Misread, note)))
            .collect(),
    }
}

#[derive(Clone, Debug)]
struct Group {
    name: String,
    identity: Option<Vec<u8>>,
    law: bool,
    designer: bool,
    ideas: Vec<usize>,
}

#[derive(Clone, Debug, Default)]
struct Database {
    tags: IdeaTags,
    groups: Vec<Group>,
    ideas: Vec<Idea>,
    unknown: Option<String>,
}

impl Database {
    fn groups<'a>(&self, loader: Loader, officers: &'a [String]) -> Groups<'a> {
        Groups {
            loader,
            officers,
            known: self
                .groups
                .iter()
                .filter_map(|group| group.identity.clone())
                .collect(),
            unknown: self.unknown.clone(),
        }
    }

    fn register(&mut self, mut idea: Idea, group: usize) -> usize {
        idea.position = i32::try_from(self.ideas.len() + 1).unwrap_or(i32::MAX);
        idea.group.clone_from(&self.groups[group].name);
        self.ideas.push(idea);
        let index = self.ideas.len() - 1;
        self.groups[group].ideas.push(index);
        index
    }

    fn apply(&mut self, declared: Vec<Declared>) -> (Vec<usize>, Characters) {
        let mut defined = Vec::new();
        let mut advisors = Characters::default();
        for declared in declared {
            let occurrence = match declared {
                Declared::Category(category) => {
                    self.tags.categories.push(category);
                    continue;
                }
                Declared::Group(occurrence) => occurrence,
            };
            for advisor in occurrence.advisors {
                advisors.merge(advisor);
            }
            let existing = occurrence.identity.as_ref().and_then(|identity| {
                self.groups
                    .iter()
                    .position(|group| group.identity.as_ref() == Some(identity))
            });
            match (occurrence.path, existing) {
                (Path::Again, Some(group)) => {
                    let held = &mut self.groups[group];
                    held.law = occurrence.law.unwrap_or(held.law);
                    held.designer = occurrence.designer.unwrap_or(held.designer);
                    for mut idea in occurrence.ideas {
                        idea.group.clone_from(&self.groups[group].name);
                        let same = self.groups[group].ideas.iter().copied().find(|index| {
                            self.ideas[*index].token.eq_ignore_ascii_case(&idea.token)
                        });
                        match same {
                            Some(index) => {
                                self.ideas[index] = idea;
                                defined.push(index);
                            }
                            None => {
                                self.ideas.push(idea);
                                let index = self.ideas.len() - 1;
                                self.groups[group].ideas.push(index);
                                defined.push(index);
                            }
                        }
                    }
                    for (place, index) in self.groups[group].ideas.iter().enumerate() {
                        self.ideas[*index].position = i32::try_from(place).unwrap_or(i32::MAX);
                    }
                }
                (_, Some(group)) => {
                    for idea in occurrence.ideas {
                        defined.push(self.register(idea, group));
                    }
                }
                (_, None) => {
                    self.groups.push(Group {
                        name: occurrence.name,
                        identity: occurrence.identity,
                        law: occurrence.law.unwrap_or(false),
                        designer: occurrence.designer.unwrap_or(false),
                        ideas: Vec::new(),
                    });
                    let group = self.groups.len() - 1;
                    for idea in occurrence.ideas {
                        defined.push(self.register(idea, group));
                    }
                }
            }
        }
        (defined, advisors)
    }

    fn finish(mut self) -> (IdeaTags, Vec<Idea>) {
        for group in &self.groups {
            for index in &group.ideas {
                self.ideas[*index].law = group.law;
                self.ideas[*index].designer = group.designer;
            }
        }
        (self.tags, self.ideas)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn parse_tags(input: &[u8], names: &KeyNames) -> Vec<IdeaCategory> {
    let mut database = Database::default();
    let read = read_file(
        input,
        database.groups(Loader::Tags, &[]),
        &names.at(Point::IdeaTags),
    );
    database.apply(read.declared);
    database.tags.categories
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IdeaFile {
    pub ideas: Vec<Idea>,
    pub advisors: Characters,
    pub refused: Option<String>,
    pub refused_tokens: Vec<String>,
    pub notes: Vec<String>,
}

#[cfg(any(test, feature = "test-support"))]
pub fn parse_ideas(input: &[u8], officers: &[String], names: &KeyNames) -> IdeaFile {
    let mut database = Database::default();
    let read = read_file(input, database.groups(Loader::Ideas, officers), names);
    let (_, advisors) = database.apply(read.declared);
    IdeaFile {
        ideas: database.finish().1,
        advisors,
        refused: read.stop.map(|stop| stop.cause),
        refused_tokens: read.refused_tokens,
        notes: read.notes.into_iter().map(|(_, message)| message).collect(),
    }
}

#[derive(Clone, Debug, Default)]
pub struct IdeaContent {
    pub tags: IdeaTags,
    ideas: Vec<Idea>,
    by_token: BTreeMap<Token, usize>,
    order: Vec<String>,
    laws_by_slot: BTreeMap<String, Vec<usize>>,
    coverage: Coverage,
    first_files: Vec<usize>,
}

impl IdeaContent {
    pub fn tokens(&self) -> (impl Iterator<Item = &str>, Option<&Gap>) {
        (
            self.order.iter().map(String::as_str),
            self.coverage.any_gap(),
        )
    }
    fn new(tags: IdeaTags, ideas: Vec<Idea>, coverage: Coverage, first_files: Vec<usize>) -> Self {
        let mut by_token = BTreeMap::new();
        let mut order = Vec::new();
        for (index, idea) in ideas.iter().enumerate() {
            if let std::collections::btree_map::Entry::Vacant(entry) =
                by_token.entry(Token::c_str(idea.token.as_bytes()))
            {
                entry.insert(index);
                order.push(idea.token.clone());
            }
        }
        let mut laws_by_slot: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for token in &order {
            let index = by_token[&Token::c_str(token.as_bytes())];
            let idea = &ideas[index];
            if idea.law {
                laws_by_slot
                    .entry(idea.group.clone())
                    .or_default()
                    .push(index);
            }
        }
        Self {
            tags,
            ideas,
            by_token,
            order,
            laws_by_slot,
            coverage,
            first_files,
        }
    }

    #[cfg(feature = "test-support")]
    pub fn from_parsed(categories: Vec<IdeaCategory>, ideas: Vec<Idea>) -> Self {
        Self::new(
            IdeaTags {
                categories,
                cut: None,
            },
            ideas,
            Coverage::default(),
            Vec::new(),
        )
    }

    pub fn idea(&self, token: &str) -> Result<Option<&Idea>, &Gap> {
        let index = self.by_token.get(&Token::c_str(token.as_bytes()));
        if let Some(gap) = index
            .and_then(|index| self.first_files.get(*index))
            .and_then(|file| self.coverage.gap_before(*file))
        {
            return Err(gap);
        }
        if let Some(gap) = self
            .coverage
            .unknown(&index.map_or_else(String::new, ToString::to_string))
        {
            return Err(gap);
        }
        Ok(index.map(|index| &self.ideas[*index]))
    }

    pub fn laws(&self, slot: &str) -> (Vec<&Idea>, Option<&Gap>) {
        let laws = self
            .laws_by_slot
            .get(slot)
            .into_iter()
            .flatten()
            .map(|index| &self.ideas[*index])
            .collect();
        (laws, self.coverage.any_gap())
    }
}

pub struct IdeaCatalogue {
    pub ideas: Arc<IdeaContent>,
    pub advisors: Arc<Characters>,
    pub warnings: Vec<content_reader_parser::Diagnostic>,
}

pub struct CharacterCatalogue {
    pub characters: Arc<Characters>,
    pub warnings: Vec<content_reader_parser::Diagnostic>,
}

fn officer_key(officers: &[String]) -> Vec<u8> {
    let mut key = Vec::new();
    content_reader_layers::kept::put_texts(&mut key, officers);
    key
}

pub fn catalogue(layers: &ResolvedLayers, officers: &[String]) -> io::Result<Arc<IdeaCatalogue>> {
    layers.retained_product("political ideas and advisor templates", 1, &officer_key(officers), |layers| {
        let (ideas, advisors, warnings) = read(layers, officers)?;
        Ok(IdeaCatalogue { ideas: Arc::new(ideas), advisors: Arc::new(advisors), warnings })
    })
}

pub fn characters(layers: &ResolvedLayers, officers: &[String]) -> io::Result<Arc<CharacterCatalogue>> {
    layers.retained_product("characters with political advisor templates", 1, &officer_key(officers), |layers| {
        let ideas = catalogue(layers, officers)?;
        let (characters, warnings) = characters::read(layers, (*ideas.advisors).clone())?;
        Ok(CharacterCatalogue { characters: Arc::new(characters), warnings })
    })
}

pub fn read(
    layers: &ResolvedLayers,
    officers: &[String],
) -> io::Result<(IdeaContent, Characters, Vec<content_reader_parser::Diagnostic>)> {
    let mut database = Database::default();
    let mut coverage = Coverage::default();
    let mut first_files = Vec::new();
    let mut advisors = Characters::default();
    let mut warnings = Vec::new();
    let names = layers.key_names()?;
    for (loader, folder, what, names) in [
        (
            Loader::Tags,
            "common/idea_tags/",
            "Idea categories",
            names.at(Point::IdeaTags),
        ),
        (
            Loader::Ideas,
            "common/ideas/",
            "Ideas",
            names.at(Point::Late),
        ),
    ] {
        for path in script_files::paths(layers, folder, Some(".txt"))? {
            coverage.next_file();
            advisors.next_file();
            let read = read_file(
                &path.read(layers)?,
                database.groups(loader, officers),
                &names,
            );
            warnings.extend(
                read.notes
                    .iter()
                    .map(|(kind, note)| (*kind, format!("{what} in {path}: {note}."))),
            );
            let known = database.tags.categories.len();
            let (defined, file_advisors) = database.apply(read.declared);
            first_files.resize(database.ideas.len(), coverage.file());
            for index in defined {
                coverage.replace(&index.to_string());
            }
            advisors.register(&file_advisors);
            advisors.merge(file_advisors);
            if let Some(stop) = read.stop {
                coverage.gap(&path.text(), stop.cause.clone());
                advisors.gap(&path.text(), &stop.cause);
                if database.tags.cut.is_none() {
                    database.tags.cut = Some((
                        known + stop.categories,
                        Gap {
                            path: path.to_string(),
                            cause: stop.cause.clone(),
                        },
                    ));
                }
                database
                    .unknown
                    .get_or_insert_with(|| format!("{path}: {}", stop.cause));
                warnings.push((content_reader_parser::DiagnosticKind::Unsettled, format!(
                    "{what} unavailable in {path}: {}, so the rest of the file cannot be determined; refused ideas and advisors: {}",
                    stop.cause,
                    if read.refused_tokens.is_empty() {
                        "none".to_owned()
                    } else {
                        read.refused_tokens.join(", ")
                    }
                )));
            }
        }
    }
    let (tags, ideas) = database.finish();
    let content = IdeaContent::new(tags, ideas, coverage, first_files);
    let repeated = content.ideas.len() - content.by_token.len();
    if repeated > 0 {
        warnings.push((
            content_reader_parser::DiagnosticKind::Warning,
            format!("{repeated} ideas share a lexer token; the first database entry is used."),
        ));
    }
    Ok((content, advisors, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_key_names::characters::Authored;

    #[test]
    fn idea_case_aliases_select_the_first_token_and_keep_authored_spelling() {
        content_reader_script::test_game::enter();
        let ideas = vec![
            Idea {
                token: "mod_Spirit".into(),
                picture: Some("first".into()),
                ..Idea::default()
            },
            Idea {
                token: "MOD_SPIRIT".into(),
                picture: Some("second".into()),
                ..Idea::default()
            },
            Idea {
                token: "É_spirit".into(),
                ..Idea::default()
            },
            Idea {
                token: "é_spirit".into(),
                ..Idea::default()
            },
        ];
        let content = IdeaContent::new(IdeaTags::default(), ideas, Coverage::default(), Vec::new());
        let found = content.idea("MOD_spirit").unwrap().unwrap();
        assert_eq!(found.token, "mod_Spirit");
        assert_eq!(found.picture.as_deref(), Some("first"));
        assert_eq!(
            content.tokens().0.collect::<Vec<_>>(),
            ["mod_Spirit", "É_spirit", "é_spirit"]
        );
        assert_eq!(content.idea("É_SPIRIT").unwrap().unwrap().token, "É_spirit");
        assert_eq!(content.idea("é_SPIRIT").unwrap().unwrap().token, "é_spirit");
        assert!(content.idea("missing").unwrap().is_none());
    }

    #[test]
    fn idea_case_aliases_share_in_place_redefinition_coverage() {
        content_reader_script::test_game::enter();
        let mut coverage = Coverage::default();
        coverage.replace("0");
        coverage.next_file();
        coverage.gap("gap.txt", "unread suffix");
        coverage.next_file();
        coverage.replace("1");
        let ideas = vec![
            Idea {
                token: "first".into(),
                ..Idea::default()
            },
            Idea {
                token: "FIRST".into(),
                ..Idea::default()
            },
        ];
        let content = IdeaContent::new(IdeaTags::default(), ideas, coverage.clone(), vec![0, 2]);
        assert_eq!(content.idea("First").unwrap_err().path, "gap.txt");
        coverage.replace("0");
        let content = IdeaContent::new(content.tags, content.ideas, coverage, content.first_files);
        assert_eq!(content.idea("FIRST").unwrap().unwrap().token, "first");
        assert!(content.idea("absent").is_err());
    }

    #[test]
    fn unknown_modifier_key_is_reported_before_numeric_conversion() {
        content_reader_script::test_game::enter();
        let mut scan = modifiers::ScanFailures::default();
        let mut values = Modifiers::default();
        values.read_all(&[Item {
            key: Some(b"tag".to_vec()),
            key_quoted: false,
            operator: content_reader_parser::parser::Operator::Equals,
            value: content_reader_parser::parser::Value::Scalar(b"LEB".to_vec()),
            kind: Kind::Bare,
        }]);
        scan.idea_modifiers("SNS_Lebanese_Resistance", &values, &KeyNames::default());
        let warning = scan.warning().unwrap();
        assert!(warning.contains("unknown modifier keys"), "{warning}");
        assert!(warning.contains("tag = LEB"), "{warning}");
        assert!(!warning.contains("numeric conversion"), "{warning}");
    }

    #[test]
    fn undecided_modifier_key_keeps_the_numeric_note() {
        content_reader_script::test_game::enter();
        let mut scan = modifiers::ScanFailures::default();
        let mut values = Modifiers::default();
        values.read_all(&[Item {
            key: Some(b"tag".to_vec()),
            key_quoted: false,
            operator: content_reader_parser::parser::Operator::Equals,
            value: content_reader_parser::parser::Value::Scalar(b"LEB".to_vec()),
            kind: Kind::Bare,
        }]);
        let mut names = KeyNames::default();
        names.effects.modifiers_gap = Some("unread modifier suffix".into());
        scan.idea_modifiers("SNS_Lebanese_Resistance", &values, &names);
        let warning = scan.warning().unwrap();
        assert!(warning.contains("numeric conversion"), "{warning}");
        assert!(!warning.contains("unknown modifier"), "{warning}");
    }

    fn values(modifiers: &Modifiers) -> Vec<(String, i64)> {
        modifiers
            .values
            .iter()
            .map(|m| (m.name.clone(), m.value))
            .collect()
    }

    fn database(files: &[(Loader, &[u8])]) -> (IdeaTags, Vec<Idea>) {
        let mut database = Database::default();
        for (loader, input) in files {
            let read = read_file(input, database.groups(*loader, &[]), &KeyNames::default());
            assert_eq!(read.stop, None);
            database.apply(read.declared);
        }
        database.finish()
    }

    fn by<'a>(ideas: &'a [Idea], token: &str) -> &'a Idea {
        ideas.iter().find(|idea| idea.token == token).unwrap()
    }

    #[test]
    fn categories_keep_slots_seats_and_the_politics_tab_flag() {
        content_reader_script::test_game::enter();
        let categories = parse_tags(
            b"idea_categories = { government = { slot = economy slot = trade_laws character_slot = political_advisor \
              character_slot = political_advisor ledger = civilian cost = 150 removal_cost = 0 } \
              country = { type = national_spirit cost = 0 removal_cost = -1 } \
              army_spirit = { politics_tab = no type = army_spirit } hidden_ideas = { hidden = yes } \
              quoted = { type = \"national_spirit\" cost = 010 cost = bad } \
              folded = { TYPE = NATIONAL_SPIRIT ledger = ARMY } reset = { type = army_spirit type = default } } \
              slot_ledgers = { army_chief = army }", &KeyNames::default(),
        );
        let tags = IdeaTags {
            categories,
            ..IdeaTags::default()
        };
        let category = |name: &str| tags.resolve(name).unwrap().unwrap();
        let government = category("government");
        assert_eq!(government.slots, ["economy", "trade_laws"]);
        assert_eq!(government.character_slots.len(), 2);
        assert_eq!(
            (government.cost, government.removal_cost, government.ledger),
            (Some(150.0), Some(0.0), 0x2)
        );
        assert_eq!(category("political_advisor").name, "government");
        assert_eq!(category("country").kind.as_deref(), Some("national_spirit"));
        assert!(!category("army_spirit").politics_tab);
        assert!(category("hidden_ideas").hidden);
        let quoted = category("quoted");
        assert_eq!((quoted.kind.as_deref(), quoted.cost), (None, Some(8.0)));
        let folded = category("folded");
        assert_eq!(folded.kind.as_deref(), Some("national_spirit"));
        assert_eq!(folded.ledger, 0x4);
        assert_eq!(category("reset").kind, None);
    }

    #[test]
    fn categories_resolve_in_array_order_and_repeat_as_new_objects() {
        content_reader_script::test_game::enter();
        let categories = parse_tags(
            b"idea_categories = { first = { slot = X } X = { } first = { slot = Y } x = { slot = Z } }", &KeyNames::default(),
        );
        let tags = IdeaTags {
            categories,
            ..IdeaTags::default()
        };
        assert_eq!(tags.categories().0.len(), 4);
        let resolve = |name: &str| tags.resolve(name).unwrap().map(|found| found.slots.clone());
        assert_eq!(resolve("X"), Some(vec!["X".to_owned()]));
        assert_eq!(resolve("first"), Some(vec!["X".to_owned()]));
        assert_eq!(resolve("Y"), Some(vec!["Y".to_owned()]));
        assert_eq!(resolve("z"), None);
    }

    #[test]
    fn ideas_carry_their_group_law_flag_picture_and_effects() {
        content_reader_script::test_game::enter();
        let ideas = parse_ideas(
            b"@bonus = 0.15 ideas = { economy = { law = yes use_list_view = yes \
                civilian_economy = { default = yes allowed = { always = yes } modifier = { consumer_goods_factor = 0.35 } } } \
              tank_manufacturer = { designer = yes porsche = { picture = porsche_logo research_bonus = { armor = @bonus } \
                traits = { heavy_tank_manufacturer } equipment_bonus = { armor = { armor_value = 0.05 } } } } \
              country = { GER_spirit = { name = GER_SPIRIT_NAME modifier = { stability_factor = 0.1 } \
                modifier = { war_support_factor = [?x] custom_modifier_tooltip = GER_TIP } \
                targeted_modifier = { tag = SOV attack_bonus_against = 0.1 } } } }",
            &[], &KeyNames::default(),
        );
        assert_eq!(ideas.refused, None);
        assert_eq!(ideas.notes.len(), 1, "{:?}", ideas.notes);
        assert!(
            ideas.notes[0].contains("1 content values failed native numeric conversion")
                && ideas.notes[0].contains("GER_spirit: war_support_factor = [?x]"),
            "{:?}",
            ideas.notes
        );
        let ideas = ideas.ideas;
        let law = by(&ideas, "civilian_economy");
        assert!(law.law && law.default && !law.designer);
        assert_eq!(law.group, "economy");
        assert_eq!(law.sprite(), "GFX_idea_civilian_economy");
        let designer = by(&ideas, "porsche");
        assert!(designer.designer);
        assert_eq!(designer.sprite(), "GFX_idea_porsche_logo");
        assert_eq!(designer.research_bonuses[0].value, 15_000);
        assert_eq!(designer.traits, ["heavy_tank_manufacturer"]);
        assert_eq!(designer.equipment_bonuses[0].equipment, "armor");
        let spirit = by(&ideas, "GER_spirit");
        assert_eq!(spirit.name.as_deref(), Some("GER_SPIRIT_NAME"));
        assert_eq!(
            values(&spirit.modifiers),
            [
                ("stability_factor".to_owned(), 10_000),
                ("war_support_factor".to_owned(), 0)
            ]
        );
        assert_eq!(spirit.modifiers.tooltips(), ["GER_TIP"]);
        assert_eq!(spirit.targeted[0].tag.as_deref(), Some("SOV"));
        assert_eq!(
            [law.position, designer.position, spirit.position],
            [1, 2, 3]
        );
    }

    #[test]
    fn idea_rules_keep_defaults_repeated_values_and_description_overrides() {
        content_reader_script::test_game::enter();
        let file = parse_ideas(
            b"ideas={country={a={rule={can_create_factions=yes desc=FIRST override={013=DECIMAL 7=VOLUNTEERS}} rule={desc=LAST can_join_factions=maybe can_create_factions=no can_send_volunteers=yes}} b={}}}",
            &[], &KeyNames::default(),
        );
        assert_eq!(file.refused, None);
        let rules = &by(&file.ideas, "a").rules;
        let index = |name| {
            crate::power_balance::rules()
                .iter()
                .position(|rule| rule.0 == name)
                .unwrap()
        };
        assert!(!rules.values[index("can_create_factions")]);
        assert!(!rules.values[index("can_join_factions")]);
        assert!(rules.values[index("can_send_volunteers")]);
        assert_eq!(rules.description, "LAST");
        assert_eq!(
            rules.overrides,
            BTreeMap::from([(13, "DECIMAL".into()), (7, "VOLUNTEERS".into())])
        );
        assert_eq!(by(&file.ideas, "b").rules, IdeaRules::default());
    }

    #[test]
    fn idea_rule_override_outside_rule_storage_is_refused() {
        content_reader_script::test_game::enter();
        for index in [-1, i32::MAX] {
            let input = format!("ideas={{country={{a={{rule={{override={{{index}=TIP}}}}}}}}}}");
            let file = parse_ideas(input.as_bytes(), &[], &KeyNames::default());
            assert!(
                file.refused
                    .as_ref()
                    .is_some_and(|why| why.contains("writes outside rule storage")),
                "{:?}",
                file.refused
            );
        }
    }

    #[test]
    fn a_group_read_again_keeps_its_flags_and_its_ideas() {
        content_reader_script::test_game::enter();
        let (_, ideas) = database(&[
            (
                Loader::Ideas,
                b"ideas = { country = { spirit = { } } economy = { law = yes a = { level = 1 } b = { } } }",
            ),
            (
                Loader::Ideas,
                b"ideas = { economy = { c = { } } Economy = { A = { level = 2 } use_list_view = { } } }",
            ),
        ]);
        let tokens: Vec<_> = ideas.iter().map(|idea| idea.token.as_str()).collect();
        assert_eq!(tokens, ["spirit", "A", "b", "c", "use_list_view"]);
        assert!(
            ideas[1..]
                .iter()
                .all(|idea| idea.law && idea.group == "economy")
        );
        assert_eq!(by(&ideas, "A").level, 2);
        let positions: Vec<_> = ideas.iter().map(|idea| idea.position).collect();
        assert_eq!(positions, [1, 0, 1, 2, 3]);
        let (_, ideas) = database(&[
            (
                Loader::Ideas,
                b"ideas = { economy = { law = yes a = { } } }",
            ),
            (Loader::Ideas, b"ideas = { economy = { law = no b = { } } }"),
        ]);
        assert!(ideas.iter().all(|idea| !idea.law));
    }

    #[test]
    fn a_new_group_of_a_held_name_merges_its_ideas() {
        content_reader_script::test_game::enter();
        let (_, ideas) = database(&[
            (Loader::Tags, b"ideas = { economy = { law = yes a = { } } }"),
            (Loader::Tags, b"ideas = { ECONOMY = { law = no a = { } } }"),
            (
                Loader::Ideas,
                b"ideas = { \"economy\" = { law = no b = 5 c = { } } }",
            ),
        ]);
        let shown: Vec<_> = ideas
            .iter()
            .map(|idea| (idea.token.as_str(), idea.position, idea.law))
            .collect();
        assert_eq!(shown, [("a", 1, true), ("a", 2, true), ("c", 3, true)]);
    }

    #[test]
    fn both_directories_declare_categories_and_ideas() {
        content_reader_script::test_game::enter();
        let (tags, ideas) = database(&[
            (
                Loader::Tags,
                b"idea_categories = { country = { } } ideas = { country = { early = { } } }",
            ),
            (
                Loader::Ideas,
                b"idea_categories = { army = { slot = army_chief } } ideas = { country = { late = { } } }",
            ),
        ]);
        assert_eq!(tags.resolve("army_chief").unwrap().unwrap().name, "army");
        let tokens: Vec<_> = ideas.iter().map(|idea| idea.token.as_str()).collect();
        assert_eq!(tokens, ["early", "late"]);
    }

    #[test]
    fn a_ledger_is_read_as_its_reader_reads_it() {
        content_reader_script::test_game::enter();
        let file = parse_ideas(
            b"ideas = { country = { a = { name = OLD ledger = { name = NEW } ledger = Army ledger = navy } \
              b = { ledger = all ledger = hidden } c = { ledger = military ledger = \"air\" ledger = 5 } \
              d = { ledger = air ledger = invalid } } }",
            &[], &KeyNames::default(),
        );
        assert_eq!(file.refused, None);
        let a = by(&file.ideas, "a");
        assert_eq!((a.name.as_deref(), a.ledger), (Some("OLD"), 0xC));
        assert_eq!(by(&file.ideas, "b").ledger, 0x1);
        assert_eq!(by(&file.ideas, "c").ledger, 0x1C);
        assert_eq!(by(&file.ideas, "d").ledger, 0xFFFF);
    }

    #[test]
    fn ideas_follow_the_file_reader_table_in_document_order() {
        content_reader_script::test_game::enter();
        let file = parse_ideas(
            b"IDEAS = { country = { first = { modifier = { a = @late b = \"@top\" } } \
              second = { @cost = 25 cost = @cost picture = @pic allowed = { @pic = second_pic always = yes } \
              on_add = { @gain = 0.2 add_stability = 0.1 } modifier = { c = @gain } picture = @pic \
              rule = { @ruled = 7 icon = { @icon = 8 } can_decline_call_to_war = no } traits = { @trait } } \
              @trait = trait_x third = { traits = { @trait } research_bonus = { @r = 0.1 armor = @r } \
              equipment_bonus = { armor = { @e = 0.3 armor_value = @e } } targeted_modifier = { @t = SOV tag = @t d = @e } } } } \
              @late = 1 @top = 2 other = { @skipped = 3 x = 1 } idea_categories = { c = { @cat = 0.4 cost = 1 } } \
              ideas = { country = { fourth = { modifier = { e = @skipped f = @late g = @cat h = @ruled i = @icon } } } }",
            &[], &KeyNames::default(),
        );
        assert_eq!(file.refused, None);
        let texts = |idea: &Idea| {
            idea.modifiers
                .values
                .iter()
                .map(|m| (m.key.clone(), m.text.clone(), m.value))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            texts(by(&file.ideas, "first")),
            [
                ("a".to_owned(), "@late".to_owned(), 0),
                ("b".to_owned(), "@top".to_owned(), 0),
            ]
        );
        let second = by(&file.ideas, "second");
        assert_eq!(second.cost, Some(25.0));
        assert_eq!(second.picture.as_deref(), Some("second_pic"));
        assert_eq!(values(&second.modifiers), [("c".to_owned(), 20_000)]);
        assert_eq!(second.traits, ["@trait"]);
        let third = by(&file.ideas, "third");
        assert_eq!(third.traits, ["trait_x"]);
        assert_eq!(third.research_bonuses[0].value, 10_000);
        assert_eq!(third.equipment_bonuses[0].values[0].value, 30_000);
        assert_eq!(third.targeted[0].tag.as_deref(), Some("SOV"));
        assert_eq!(
            values(&third.targeted[0].modifiers),
            [("d".to_owned(), 30_000)]
        );
        assert_eq!(
            texts(by(&file.ideas, "fourth")),
            [
                ("e".to_owned(), "@skipped".to_owned(), 0),
                ("f".to_owned(), "1".to_owned(), 100_000),
                ("g".to_owned(), "0.4".to_owned(), 40_000),
                ("h".to_owned(), "7".to_owned(), 700_000),
                ("i".to_owned(), "@icon".to_owned(), 0),
            ]
        );
    }

    #[test]
    fn unsettled_blocks_refuse_their_group_and_the_rest_of_the_file() {
        content_reader_script::test_game::enter();
        let file = parse_ideas(
            b"@x = 1 ideas = { country = { kept = { modifier = { a = @x } } } \
              economy = { law = yes early = { cost = 1 } broken = { allowed = { @x = 2 always = yes } } late = { } } }",
            &[], &KeyNames::default(),
        );
        assert_eq!(file.refused, None);
        assert_eq!(file.ideas.len(), 4);
        assert!(file.notes[0].contains("variable `@x` is already taken"));
        for (source, expected) in [
            (
                b"ideas = { country = { kept = { } } economy = { early = { } broken = { modifier = { odd = { } } } late = { } } \
                  hidden_ideas = { after = { } } }"
                    .as_slice(),
                "line 1: odd has a block value, which modifier value reader reads",
            ),
            (
                b"ideas = { country = { kept = { } } economy = { early = { } broken = { allowed = { OR = { tag = GER x = { @v = 1 } } } } late = { } } \
                  hidden_ideas = { after = { } } }",
                "line 1: field `x`: trigger dispatch reads it",
            ),
            (
                b"ideas = { country = { kept = { } } economy = { early = { } broken = { name = \"\xfe\" } late = { } } \
                  hidden_ideas = { after = { } } }",
                "idea broken: its name is the stored text `\\xfe` is not UTF-8",
            ),
        ] {
            let file = parse_ideas(source, &[], &KeyNames::unknown());
            let reason = file.refused.unwrap_or_default();
            assert!(reason.starts_with(expected), "{reason}");
            assert_eq!(
                file.ideas
                    .iter()
                    .map(|idea| idea.token.as_str())
                    .collect::<Vec<_>>(),
                ["kept"]
            );
            assert!(file.refused_tokens.starts_with(&["early".to_owned(), "broken".to_owned()]));
        }
    }

    #[test]
    fn keywords_fold_quoted_keys_are_names_and_failed_values_keep_the_field() {
        content_reader_script::test_game::enter();
        let file = parse_ideas(
            b"Ideas = { economy = { LAW = yes USE_LIST_VIEW = yes \"law\" = no \"designer\" = { COST = 5 cost = bad \
              \"cost\" = 9 Picture = pic DEFAULT = yes default = maybe designer = yes } } } \
              \"ideas\" = { country = { skipped = { } } }",
            &[], &KeyNames::default(),
        );
        assert_eq!(file.refused, None);
        assert_eq!(file.ideas.len(), 1);
        let idea = &file.ideas[0];
        assert_eq!(idea.token, "designer");
        assert!(idea.law && !idea.designer && idea.default);
        assert_eq!(idea.cost, Some(5.0));
        assert_eq!(idea.picture.as_deref(), Some("pic"));
        let file = parse_ideas(
            b"ideas = { g = { use_list_view = { } kept = { } } }",
            &[],
            &KeyNames::default(),
        );
        assert_eq!(file.refused, None);
        assert!(
            file.notes[0].contains("field `use_list_view` is a block"),
            "{:?}",
            file.notes
        );
        assert!(file.ideas.is_empty());
    }

    #[test]
    fn officer_corps_groups_are_character_advisor_templates() {
        content_reader_script::test_game::enter();
        let officers = ["theorist".to_owned(), "army_chief".to_owned()];
        let source = b"ideas = { theorist = { GER_x = { picture = p cost = 150 cost = wide traits = { t } \
                       targeted_modifier = { tag = SOV a = 1 } modifier = { b = 2 } TRAITS = { u } idea_token = other \
                       research_bonus = { land_doctrine = 0.1 } MODIFIER = { B = 3 c = 1 } } } \
                       Theorist = { again = { } } \
                       army_chief = { GER_z = { NAME = GER_z_name slot = high_command ledger = ARMY } } \
                       country = { GER_y = { targeted_modifier = { tag = SOV a = 1 } } } }";
        let file = parse_ideas(source, &officers, &KeyNames::default());
        assert_eq!(file.refused, None);
        let shown: Vec<_> = file
            .ideas
            .iter()
            .map(|idea| (idea.token.as_str(), idea.group.as_str()))
            .collect();
        assert_eq!(shown, [("again", "theorist"), ("GER_y", "country")]);
        assert_eq!(file.ideas[1].targeted.len(), 1);
        let advisor = |token: &str| file.advisors.advisor(token).unwrap().unwrap();
        let x = advisor("GER_x");
        assert_eq!(
            (x.character.as_str(), x.slot.as_str(), x.idea_token.as_str()),
            ("GER_x", "theorist", "GER_x")
        );
        assert_eq!(
            (x.cost, x.traits.as_slice()),
            (
                Some(15_000_000),
                ["t".to_owned(), "u".to_owned()].as_slice()
            )
        );
        assert_eq!(
            values(&x.modifiers),
            [("b".to_owned(), 300_000), ("c".to_owned(), 100_000)]
        );
        assert_eq!(x.research_bonuses[0].name, "land_doctrine");
        assert_eq!(
            file.advisors.name("GER_x").unwrap(),
            Some(&Authored::Asset("GER_x".into()))
        );
        let small = ("army".to_owned(), "small".to_owned());
        let portrait =
            |token: &str| file.advisors.portraits(token).unwrap().unwrap()[&small].clone();
        assert_eq!(portrait("GER_x"), Authored::Asset("GFX_idea_GER_x".into()));
        let z = advisor("GER_z");
        assert_eq!((z.slot.as_str(), z.ledger), ("high_command", 0x4));
        assert_eq!(
            file.advisors.name("GER_z").unwrap(),
            Some(&Authored::Asset("GER_z_name".into()))
        );
        assert_eq!(
            portrait("GER_z"),
            Authored::Asset("GFX_idea_GER_z_name".into())
        );
        let plain = parse_ideas(source, &[], &KeyNames::default());
        assert_eq!(plain.ideas.len(), 4);
        assert_eq!(plain.advisors, Characters::default());
    }

    #[test]
    fn a_refused_officer_corps_entry_is_no_advisor() {
        content_reader_script::test_game::enter();
        let officers = ["theorist".to_owned()];
        let file = parse_ideas(
            b"ideas = { theorist = { GER_a = { } GER_b = { modifier = { odd = { } } } GER_c = { } } }",
            &officers, &KeyNames::default(),
        );
        assert!(file.refused.is_some());
        assert_eq!(file.advisors, Characters::default());
        assert_eq!(file.refused_tokens, ["GER_a", "GER_b", "GER_c"]);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn a_file_that_stops_short_refuses_what_it_could_define() {
        content_reader_script::test_game::enter();
        let root = std::env::temp_dir().join(format!("content-reader-ideas-gap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "common/idea_tags/00.txt",
            "idea_categories = { country = { type = national_spirit } }",
        );
        write(
            "common/ideas/a.txt",
            "ideas = { country = { early = { } redone = { } } theorist = { GER_t = { } } }",
        );
        write(
            "common/ideas/b.txt",
            "ideas = { country = { kept = { } odd = { modifier = { x = { } } } } } \
             idea_categories = { later = { } }",
        );
        write(
            "common/ideas/c.txt",
            "ideas = { country = { redone = { } late = { } } }",
        );
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let officers = ["theorist".to_owned(), "another_slot".to_owned()];
        let catalogue = self::catalogue(&layers, &officers).unwrap();
        let same = self::catalogue(&layers, &officers).unwrap();
        assert!(Arc::ptr_eq(&catalogue, &same));
        let reversed = [officers[1].clone(), officers[0].clone()];
        assert!(!Arc::ptr_eq(&catalogue, &self::catalogue(&layers, &reversed).unwrap()));
        let merged = characters(&layers, &officers).unwrap();
        assert!(Arc::ptr_eq(&merged, &characters(&layers, &officers).unwrap()));
        let (content, advisors, warnings) = (&catalogue.ideas, &catalogue.advisors, &catalogue.warnings);
        let gap = content.idea("early").unwrap_err();
        assert_eq!(gap.path, "common/ideas/b.txt");
        assert!(
            gap.cause.starts_with("line 1: x has a block value"),
            "{gap}"
        );
        assert!(content.idea("never").is_err());
        assert!(content.idea("kept").is_err());
        assert!(content.idea("redone").unwrap().is_some());
        assert!(content.idea("late").is_err());
        assert!(content.idea("LATE").is_err());
        assert!(advisors.advisor("GER_t").is_err());
        assert!(content.tags.resolve("country").unwrap().is_some());
        assert!(content.tags.resolve("later").is_err());
        assert!(
            warnings
                .iter()
                .any(|(_, warning)| warning.contains("Ideas unavailable in common/ideas/b.txt"))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_group_an_earlier_gap_may_hold_is_refused() {
        content_reader_script::test_game::enter();
        let mut database = Database {
            unknown: Some("b.txt: line 1: x".into()),
            ..Database::default()
        };
        database.groups.push(Group {
            name: "country".into(),
            identity: Some(b"country".to_vec()),
            law: false,
            designer: false,
            ideas: Vec::new(),
        });
        let read = read_file(
            b"ideas = { country = { kept = { } } fresh = { new = { } } }",
            database.groups(Loader::Ideas, &[]),
            &KeyNames::default(),
        );
        let stop = read.stop.unwrap();
        assert_eq!(stop.ideas, 1);
        assert!(
            stop.cause.starts_with(
                "group fresh may be one the unknown rest of an earlier file registers (b.txt: line 1: x)"
            ),
            "{}",
            stop.cause
        );
        assert_eq!(read.refused_tokens, ["new"]);
    }

    #[test]
    fn the_replay_classes_hold_for_ideas() {
        content_reader_script::test_game::enter();
        let file = parse_ideas(
            b"@v = 3 ideas = { country = { a = { cost = { picture = inner } removal_cost = @v } } }",
            &[], &KeyNames::default(),
        );
        let a = &file.ideas[0];
        assert_eq!((a.cost, a.picture.as_deref()), (None, Some("inner")));
        assert_eq!(a.removal_cost, None);
        assert!(file.notes[0].contains("field `cost` is a block"));
        let file = parse_ideas(
            b"ideas = { country = { open = { removal_cost = @v",
            &[],
            &KeyNames::default(),
        );
        assert_eq!(file.notes.len(), 1, "{:?}", file.notes);
        assert!(
            file.notes[0].contains("1 content values failed native numeric conversion")
                && file.notes[0].contains("open: removal_cost = @v"),
            "{:?}",
            file.notes
        );
        assert_eq!(file.ideas[0].token, "open");
        assert_eq!(file.ideas[0].removal_cost, None);
    }
}
