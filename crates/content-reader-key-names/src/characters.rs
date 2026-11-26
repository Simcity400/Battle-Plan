use crate::key_name_products::KeyNamesExt as _;

use content_reader_layers::{ResolvedLayers, script_files::ScriptPath};
use content_reader_parser::{coverage::{Coverage, Gap}, parser::{self, Item, Unheld, Value}};
use content_reader_script::{key_names::KeyNames, modifiers::{self, Modifier, Modifiers}, native::{self, Field, Shape}, scope::ScopeKind, triggers::{self, Triggers}};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub const PORTRAIT_ROLES: [&str; 6] = ["civilian", "army", "navy", "air", "operative", "scientist"];

pub const PORTRAIT_SIZES: [&str; 2] = ["small", "large"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Authored {
    Missing,
    Asset(String),
    Ambiguous(BTreeSet<String>),
    Unknown(String),
}

impl Authored {
    pub fn names(&self) -> BTreeSet<String> {
        match self {
            Self::Missing | Self::Unknown(_) => BTreeSet::new(),
            Self::Asset(name) => BTreeSet::from([name.clone()]),
            Self::Ambiguous(names) => names.clone(),
        }
    }

    pub fn agree(&mut self, other: Self) {
        match (&*self, other) {
            (Self::Unknown(_), _) => {}
            (_, other @ Self::Unknown(_)) => *self = other,
            (current, other) if *current == other => {}
            (current, other) => {
                let mut names = current.names();
                names.extend(other.names());
                *self = Self::Ambiguous(names);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Modifier(modifiers::Node),
    Mtth,
}

pub fn advisor_field(field: &Field<'_>) -> Shape<Role> {
    match field.keyword().as_deref() {
        Some(b"available" | b"visible" | b"allowed" | b"do_effect") => Shape::Trigger,
        Some(b"on_add" | b"on_remove") => Shape::EffectIn(ScopeKind::Character.bit()),
        Some(b"name" | b"slot" | b"idea_token") => Shape::Current,
        Some(
            b"desc"
            | b"picture"
            | b"cost"
            | b"removal_cost"
            | b"command_power"
            | b"always_show_on_actions_tooltip"
            | b"can_be_fired",
        ) => Shape::One,
        Some(b"ledger") => content_reader_script::idea_fields::ledger_shape(field),
        Some(b"modifier") => Shape::Object(Role::Modifier(modifiers::Node::Modifier)),
        Some(b"ai_will_do") => Shape::Object(Role::Mtth),
        Some(b"traits") => Shape::ListOrSkip,
        Some(b"research_bonus") => {
            Shape::ObjectOrSkip(Role::Modifier(modifiers::Node::ResearchBonus))
        }
        _ => Shape::Skip,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Top,
    Table,
    Body,
    Instance,
    Portraits,
    Sizes,
    Leader,
    Advisor,
    Role(Role),
    Military,
    Scientist,
    ScientistSkills,
}

#[derive(Default)]
struct Reading {
    owner: String,
    skipped_portraits: Vec<native::Misread>,
    undecided: Vec<modifiers::Undecided>,
    body_first: bool,
    instance_first: bool,
    instances: bool,
    characters: usize,
    stop: Option<(usize, String)>,
}

impl Reading {
    fn stop(&mut self, line: u32, what: String) {
        if self.stop.is_none() {
            self.stop = Some((
                self.characters.saturating_sub(1),
                format!("line {line}: {what}"),
            ));
        }
    }

    fn skip_portrait_key(&mut self, what: &str, field: &Field<'_>) {
        self.skipped_portraits.push(native::Misread {
            kind: content_reader_parser::DiagnosticKind::Misread,
            count: None,
            line: field.key.line,
            what: format!(
                "{}: portrait {what} `{}` is not one the game reads; it reports the key and \
                 skips its value",
                self.owner,
                String::from_utf8_lossy(&field.key.text)
            ),
        });
    }

    fn shape(&mut self, node: Node, field: &Field<'_>) -> Shape<Node> {
        let keyword = field.keyword();
        let keyword = keyword.as_deref();
        let shape = match node {
            Node::Top if keyword == Some(b"characters") => Shape::Object(Node::Table),
            Node::Top => Shape::Skip,
            Node::Table => {
                self.characters += 1;
                self.owner = String::from_utf8_lossy(&field.key.text).into_owned();
                self.body_first = true;
                self.instances = false;
                Shape::Object(Node::Body)
            }
            Node::Body => {
                let first = std::mem::take(&mut self.body_first);
                if keyword == Some(b"instance") && (first || self.instances) {
                    self.instances = true;
                    self.instance_first = true;
                    Shape::Object(Node::Instance)
                } else if self.instances {
                    self.stop(
                        field.key.line,
                        format!(
                            "{} follows a character's instances, where the character-body reader reads the body's end",
                            String::from_utf8_lossy(&field.key.text)
                        ),
                    );
                    Shape::Skip
                } else {
                    template_field(field)
                }
            }
            Node::Instance if std::mem::take(&mut self.instance_first) => {
                if keyword == Some(b"allowed") {
                    Shape::Trigger
                } else {
                    self.stop(
                        field.key.line,
                        "an instance that does not start with `allowed`, which the character-body reader skips through block skipping".to_owned(),
                    );
                    Shape::Skip
                }
            }
            Node::Instance => template_field(field),
            Node::Portraits => match keyword {
                Some(role) if PORTRAIT_ROLES.iter().any(|known| role == known.as_bytes()) => {
                    Shape::ObjectIfOpen(Node::Sizes)
                }
                _ => {
                    self.skip_portrait_key("role", field);
                    Shape::Skip
                }
            },
            Node::Sizes => match keyword {
                Some(size) if PORTRAIT_SIZES.iter().any(|known| size == known.as_bytes()) => {
                    Shape::One
                }
                _ => {
                    self.skip_portrait_key("size", field);
                    Shape::Skip
                }
            },
            Node::Leader => match keyword {
                Some(b"id" | b"desc" | b"expire") => Shape::One,
                Some(b"ideology") => Shape::Current,
                Some(b"traits") => Shape::ListOrSkip,
                _ => Shape::Skip,
            },
            Node::Advisor => advisor_field(field).map(Node::Role),
            Node::Role(Role::Mtth) => native::mean_time_to_happen(field),
            Node::Role(Role::Modifier(modifier)) => {
                modifiers::shape(modifier, field, modifiers::UNKNOWN, &mut self.undecided)
                    .map(|modifier| Node::Role(Role::Modifier(modifier)))
            }
            Node::Military => match keyword {
                Some(
                    b"skill"
                    | b"attack_skill"
                    | b"defense_skill"
                    | b"planning_skill"
                    | b"logistics_skill"
                    | b"maneuvering_skill"
                    | b"coordination_skill"
                    | b"legacy_id"
                    | b"desc",
                ) => Shape::One,
                Some(b"visible") => Shape::Trigger,
                Some(b"traits") => Shape::ListOrSkip,
                _ => Shape::Skip,
            },
            Node::Scientist => match keyword {
                Some(b"desc") => Shape::One,
                Some(b"visible") => Shape::Trigger,
                Some(b"traits") => Shape::List,
                Some(b"skills") => Shape::Object(Node::ScientistSkills),
                _ => Shape::Skip,
            },
            Node::ScientistSkills => Shape::One,
        };
        if self.stop.is_none()
            && let Some(first) = self.undecided.first()
        {
            self.stop = Some((self.characters.saturating_sub(1), first.to_string()));
        }
        shape
    }
}

fn template_field(field: &Field<'_>) -> Shape<Node> {
    match field.keyword().as_deref() {
        Some(b"available" | b"allowed_civil_war" | b"remove") => Shape::Trigger,
        Some(b"name" | b"nationality" | b"can_be_captured" | b"generic") => Shape::One,
        Some(b"gender") => Shape::Skip,
        Some(b"portraits") => Shape::Object(Node::Portraits),
        Some(b"advisor") => Shape::Object(Node::Advisor),
        Some(b"country_leader") => Shape::Object(Node::Leader),
        Some(b"navy_leader" | b"field_marshal" | b"corps_commander") => {
            Shape::Object(Node::Military)
        }
        Some(b"scientist") => Shape::Object(Node::Scientist),
        _ => Shape::Skip,
    }
}

pub fn read_file(input: &[u8], names: &KeyNames) -> (native::Read, Option<(usize, String)>) {
    let mut reading = Reading::default();
    let mut read = native::read(
        input,
        native::Mode::SkipMark,
        native::Top::File(Node::Top),
        Some(names),
        &mut |node, field| reading.shape(node, field),
    );
    let stop = reading.stop.or_else(|| {
        read.unsettled.as_ref().map(|unsettled| {
            (
                reading.characters.saturating_sub(1),
                format!("{unsettled}; any character the rest of the file defines is unknown"),
            )
        })
    });
    read.misreads.append(&mut reading.skipped_portraits);
    read.misreads.sort_by_key(|misread| misread.line);
    (read, stop)
}

struct Body<'a> {
    own: &'a [Item],
    instances: Vec<&'a [Item]>,
}

impl<'a> Body<'a> {
    fn new(items: &'a [Item]) -> Self {
        match items.first() {
            Some(first) if first.keyword_is("instance") => Self {
                own: &[],
                instances: items
                    .iter()
                    .filter(|item| item.keyword_is("instance"))
                    .map(|item| item.value.items().unwrap_or_default())
                    .collect(),
            },
            _ => Self {
                own: items,
                instances: Vec::new(),
            },
        }
    }

    fn alternatives(&self) -> Vec<Template> {
        if self.instances.is_empty() {
            let mut template = Template::read(self.own);
            template.unconditional_startup = true;
            vec![template]
        } else {
            self.instances
                .iter()
                .map(|items| {
                    let mut template = Template::read(items);
                    template.unconditional_startup = items.first().is_some_and(|allowed| {
                        allowed.keyword_is("allowed")
                            && allowed.value.items().is_some_and(<[Item]>::is_empty)
                    });
                    template
                })
                .collect()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AdvisorFields {
    pub slot: Option<String>,
    pub idea_token: Option<String>,
    pub name: Option<String>,
    pub traits: Vec<String>,
    pub cost: Option<i64>,
    pub ledger: u32,
    pub modifiers: Modifiers,
    pub research_bonuses: Vec<Modifier>,
    pub unheld: Unheld,
}

impl AdvisorFields {
    pub(crate) fn apply(&mut self, fields: &[Item]) {
        for field in fields {
            match field.keyword().map(|keyword| keyword.folded()).as_deref() {
                Some(b"slot") => {
                    self.slot = self
                        .unheld
                        .keep("an advisor's slot", field.value.c_str())
                        .map(str::to_owned);
                }
                Some(b"idea_token") => {
                    self.idea_token = self
                        .unheld
                        .keep("an advisor's idea token", field.value.c_str())
                        .map(str::to_owned);
                }
                Some(b"name") => {
                    self.name = self
                        .unheld
                        .keep("an advisor's name", field.value.c_str())
                        .map(str::to_owned);
                }
                Some(b"cost") => self.cost = modifiers::read_fixed5(field).or(self.cost),
                Some(b"ledger") => self.ledger = content_reader_script::idea_fields::ledger(self.ledger, field),
                Some(b"traits") => self.traits.extend(strings(&field.value, &mut self.unheld)),
                Some(b"modifier") => self
                    .modifiers
                    .read_all(field.value.items().unwrap_or_default()),
                Some(b"research_bonus") => {
                    self.research_bonuses.extend(modifiers::research_bonuses(
                        field.value.items().unwrap_or_default(),
                        &mut self.unheld,
                    ));
                }
                _ => {}
            }
        }
        self.unheld.absorb(&self.modifiers.unheld);
    }
}

fn strings(value: &Value, unheld: &mut Unheld) -> Vec<String> {
    value
        .items()
        .unwrap_or_default()
        .iter()
        .filter_map(|item| {
            unheld
                .keep("a trait", item.value.read_str())
                .map(str::to_owned)
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Template {
    unconditional_startup: bool,
    name: Option<String>,
    portraits: BTreeMap<(String, String), String>,
    advisors: Vec<AdvisorFields>,
    leaders: Vec<(Option<String>, Vec<String>)>,
    unheld: Unheld,
}

impl Template {
    fn read(fields: &[Item]) -> Self {
        let mut template = Self::default();
        for field in fields {
            match field.keyword().map(|keyword| keyword.folded()).as_deref() {
                Some(b"name") => {
                    template.name = template
                        .unheld
                        .keep("its name", field.value.read_str())
                        .map(str::to_owned);
                }
                Some(b"portraits") => {
                    for portrait in field.value.items().unwrap_or_default() {
                        template.portrait(portrait);
                    }
                }
                Some(b"advisor") => {
                    let mut advisor = AdvisorFields::default();
                    advisor.apply(field.value.items().unwrap_or_default());
                    template.unheld.absorb(&advisor.unheld);
                    template.advisors.push(advisor);
                }
                Some(b"country_leader") => {
                    template.leader(field.value.items().unwrap_or_default());
                }
                _ => {}
            }
        }
        template
    }

    fn leader(&mut self, fields: &[Item]) {
        let mut ideology = None;
        let mut traits = Vec::new();
        for field in fields {
            match field.keyword().map(|keyword| keyword.folded()).as_deref() {
                Some(b"ideology") => {
                    ideology = self
                        .unheld
                        .keep("a leader's ideology", field.value.c_str())
                        .map(str::to_owned);
                }
                Some(b"traits") => traits.extend(strings(&field.value, &mut self.unheld)),
                _ => {}
            }
        }
        self.leaders.push((ideology, traits));
    }

    fn portrait(&mut self, field: &Item) {
        let Some(kind) = field.keyword().map(|keyword| keyword.folded()) else {
            return;
        };
        let kind = String::from_utf8_lossy(&kind).into_owned();
        match field.value.items() {
            Some(sizes) => {
                for size in sizes {
                    let Some(name) = size.keyword().map(|keyword| keyword.folded()) else {
                        continue;
                    };
                    if let Some(text) = self.unheld.keep("a portrait", size.value.read_str()) {
                        self.portraits.insert(
                            (kind.clone(), String::from_utf8_lossy(&name).into_owned()),
                            text.to_owned(),
                        );
                    }
                }
            }
            None => {
                if let Some(text) = self.unheld.keep("a portrait", field.value.read_str()) {
                    self.portraits
                        .insert((kind, "large".to_owned()), text.to_owned());
                }
            }
        }
    }

    fn portrait_of(&self, category: &str, size: &str) -> Authored {
        self.portraits
            .get(&(category.to_owned(), size.to_owned()))
            .map_or(Authored::Missing, |name| Authored::Asset(name.clone()))
    }
}

fn agreed(alternatives: &[Template], value: impl Fn(&Template) -> Authored) -> Authored {
    let mut values = alternatives.iter().map(value);
    let Some(mut result) = values.next() else {
        return Authored::Missing;
    };
    for alternative in values {
        result.agree(alternative);
    }
    result
}

type Declared = (Vec<(String, Vec<Template>)>, Option<(usize, String)>);

fn characters_of(items: &[Item], stop: Option<(usize, String)>) -> Declared {
    let mut stop = stop;
    let mut result = Vec::new();
    let characters = items
        .iter()
        .filter(|item| item.keyword_is("characters"))
        .flat_map(|root| root.value.items().unwrap_or_default());
    for (index, character) in characters.enumerate() {
        let alternatives = Body::new(character.value.items().unwrap_or_default()).alternatives();
        let (token, failed) = match character.key_str() {
            Ok(token) => (
                token.to_owned(),
                alternatives
                    .iter()
                    .find_map(|template| template.unheld.cause())
                    .map(|cause| format!("character {token}: {cause}")),
            ),
            Err(refused) => (
                character
                    .key
                    .as_deref()
                    .unwrap_or_default()
                    .escape_ascii()
                    .to_string(),
                Some(format!("a character's token is {refused}")),
            ),
        };
        if let Some(cause) = failed
            && stop.as_ref().is_none_or(|(at, _)| index < *at)
        {
            stop = Some((index, cause));
        }
        result.push((token, alternatives));
    }
    (result, stop)
}

pub fn portraits(
    input: &[u8],
    names: &KeyNames,
    category: &str,
    size: &str,
) -> io::Result<(BTreeMap<String, Authored>, Option<String>)> {
    let (read, stop) = read_file(input, names);
    let (declared, stop) = characters_of(&read.items, stop);
    let mut characters = BTreeMap::<String, Authored>::new();
    for (index, (token, alternatives)) in declared.into_iter().enumerate() {
        let portrait = match &stop {
            Some((at, cause)) if index >= *at => Authored::Unknown(cause.clone()),
            _ => agreed(&alternatives, |template| {
                template.portrait_of(category, size)
            }),
        };
        characters
            .entry(token)
            .and_modify(|previous| previous.agree(portrait.clone()))
            .or_insert(portrait);
    }
    Ok((characters, stop.map(|(_, cause)| cause)))
}

#[derive(Clone, Debug, Eq)]
pub struct AdvisorRole {
    pub character: String,
    pub slot: String,
    pub idea_token: String,
    pub traits: Vec<String>,
    pub cost: Option<i64>,
    pub ledger: u32,
    pub modifiers: Modifiers,
    pub research_bonuses: Vec<Modifier>,
}

impl PartialEq for AdvisorRole {
    fn eq(&self, other: &Self) -> bool {
        self.character.eq_ignore_ascii_case(&other.character)
            && self.slot.eq_ignore_ascii_case(&other.slot)
            && self.idea_token.eq_ignore_ascii_case(&other.idea_token)
            && self.traits.len() == other.traits.len()
            && self.traits.iter().zip(&other.traits).all(|(a, b)| {
                content_reader_parser::tokens::Token::c_str(a.as_bytes())
                    == content_reader_parser::tokens::Token::c_str(b.as_bytes())
            })
            && self.cost == other.cost
            && self.ledger == other.ledger
            && self.modifiers == other.modifiers
            && self.research_bonuses == other.research_bonuses
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderRole {
    pub character: String,
    pub ideology: String,
    pub traits: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Characters {
    advisors: BTreeMap<String, AdvisorRole>,
    advisor_spellings: BTreeMap<String, String>,
    ambiguous_advisors: BTreeSet<String>,
    leaders: BTreeMap<(String, String), LeaderRole>,
    ambiguous_leaders: BTreeSet<(String, String)>,
    leader_sets: BTreeMap<String, Vec<LeaderRole>>,
    ambiguous_leader_sets: BTreeSet<String>,
    names: BTreeMap<String, Authored>,
    unconditional_startup: BTreeSet<String>,
    portraits: BTreeMap<String, BTreeMap<(String, String), Authored>>,
    built: BTreeMap<(Vec<u8>, u32), Vec<TaggedRoles>>,
    unplaced: BTreeMap<Vec<u8>, String>,
    pub(crate) advisor_coverage: Coverage,
    pub(crate) character_coverage: Coverage,
    pub notes: Vec<content_reader_parser::Diagnostic>,
}

#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, replay_spool::Spooled,
)]
pub struct BuiltTemplate {
    pub tag: String,
    pub file: Vec<u8>,
    pub line: i32,
    pub generated: bool,
}

pub type Lookup<'a, T> = Result<Option<&'a T>, &'a Gap>;

impl Characters {
    pub fn declared_tokens(&self) -> impl Iterator<Item = &str> {
        self.names.keys().map(String::as_str)
    }

    pub fn advisor_tokens(&self) -> (impl Iterator<Item = &str>, Option<&Gap>) {
        (
            self.advisor_spellings.values().map(String::as_str),
            self.advisor_coverage.any_gap(),
        )
    }
    pub fn advisor(&self, idea_token: &str) -> Lookup<'_, AdvisorRole> {
        let identity = idea_token.to_ascii_lowercase();
        match self
            .advisor_coverage
            .unknown(&identity)
            .or_else(|| self.advisor_coverage.before_first(&identity))
        {
            Some(gap) => Err(gap),
            None => Ok(self.advisors.get(&identity)),
        }
    }

    pub fn ambiguous_advisor(&self, idea_token: &str) -> bool {
        self.ambiguous_advisors
            .contains(&idea_token.to_ascii_lowercase())
    }

    pub fn held_advisor(
        &self,
        idea_token: &str,
        built: Result<&BuiltTemplate, &str>,
    ) -> Result<Option<&AdvisorRole>, String> {
        if let Some(role) = self.advisor(idea_token).map_err(ToString::to_string)? {
            return Ok(Some(role));
        }
        if !self.ambiguous_advisor(idea_token) {
            return Ok(None);
        }
        let disagree = format!(
            "advisor {idea_token}: its character's alternatives define it differently, so its role is that of the template its holder was built from"
        );
        let built = built.map_err(|reason| format!("{disagree}, and {reason}"))?;
        let at = format!(
            "{} at {}:{}",
            built.tag,
            String::from_utf8_lossy(&built.file),
            built.line
        );
        if built.generated {
            return Err(format!(
                "{disagree}, and template {at} was made by generate_character, whose roles content does not define"
            ));
        }
        let file = String::from_utf8_lossy(&built.file).into_owned();
        let candidates: Vec<&Vec<AdvisorRole>> = u32::try_from(built.line)
            .ok()
            .and_then(|line| self.built.get(&(built.file.clone(), line)))
            .into_iter()
            .flatten()
            .filter(|roles| roles.tag.eq_ignore_ascii_case(&built.tag))
            .map(|roles| &roles.advisors)
            .collect();
        let roles = match candidates.as_slice() {
            [roles] => roles,
            [] => {
                return Err(match self.unplaced.get(&built.file) {
                    Some(reason) => format!(
                        "{disagree}, and the templates of {file} cannot be placed by line: {reason}"
                    ),
                    None => format!(
                        "{disagree}, and no content character template with an advisor role is {at}"
                    ),
                });
            }
            _ => {
                return Err(format!(
                    "{disagree}, and several content character templates are {at}"
                ));
            }
        };
        let mut matching = roles
            .iter()
            .filter(|role| role.idea_token.eq_ignore_ascii_case(idea_token));
        let Some(first) = matching.next() else {
            return Err(format!(
                "{disagree}, and template {at} has no advisor role {idea_token}"
            ));
        };
        if matching.any(|role| role != first) {
            return Err(format!(
                "{disagree}, and template {at} defines advisor role {idea_token} more than once, differently"
            ));
        }
        Ok(Some(first))
    }

    pub fn leader(&self, character: &str, ideology: &str) -> Lookup<'_, LeaderRole> {
        match self.character_coverage.unknown(character) {
            Some(gap) => Err(gap),
            None => Ok(self
                .leaders
                .get(&(character.to_owned(), ideology.to_owned()))),
        }
    }

    pub fn leader_roles(&self, character: &str) -> Result<Option<&Vec<LeaderRole>>, String> {
        if let Some(gap) = self.character_coverage.unknown(character) {
            return Err(gap.to_string());
        }
        if self.ambiguous_leader_sets.contains(character) {
            return Err(format!(
                "character {character}: different template roles need the unrecorded template tag-map winner"
            ));
        }
        Ok(self.leader_sets.get(character))
    }

    pub fn template_has_unconditional_startup(&self, character: &str) -> bool {
        self.unconditional_startup.contains(character)
    }

    pub fn name(&self, character: &str) -> Lookup<'_, Authored> {
        match self.character_coverage.unknown(character) {
            Some(gap) => Err(gap),
            None => Ok(self.names.get(character)),
        }
    }

    pub fn portraits(&self, character: &str) -> Lookup<'_, BTreeMap<(String, String), Authored>> {
        match self.character_coverage.unknown(character) {
            Some(gap) => Err(gap),
            None => Ok(self.portraits.get(character)),
        }
    }
}

pub fn parse(input: &[u8], names: &KeyNames) -> io::Result<Characters> {
    parse_file(input, names).map(|(characters, _, _)| characters)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TaggedRoles {
    tag: String,
    advisors: Vec<AdvisorRole>,
}

type TemplateRoles = Vec<(String, Vec<TaggedRoles>)>;

pub fn country_leader_role(character: &str, fields: &[Item]) -> Result<LeaderRole, String> {
    let mut template = Template::default();
    template.leader(fields);
    if let Some(why) = template.unheld.cause() {
        return Err(format!("country-leader role: {why}"));
    }
    Ok(leader_roles(character, &template).remove(0))
}

pub fn advisor_role(character: &str, fields: &[Item]) -> Result<AdvisorRole, String> {
    let mut advisor = AdvisorFields::default();
    advisor.apply(fields);
    if let Some(why) = advisor.unheld.cause() {
        return Err(format!("advisor role: {why}"));
    }
    Ok(advisor_roles(
        character,
        &Template {
            advisors: vec![advisor],
            ..Template::default()
        },
    )
    .remove(0))
}

fn leader_roles(character: &str, template: &Template) -> Vec<LeaderRole> {
    let mut roles = BTreeMap::new();
    for (ideology, traits) in &template.leaders {
        let ideology = ideology.clone().unwrap_or_default();
        roles.insert(
            ideology.to_ascii_lowercase(),
            LeaderRole {
                character: character.to_owned(),
                ideology,
                traits: traits.clone(),
            },
        );
    }
    roles.into_values().collect()
}

fn advisor_roles(token: &str, template: &Template) -> Vec<AdvisorRole> {
    template
        .advisors
        .iter()
        .map(|advisor| AdvisorRole {
            character: token.to_owned(),
            slot: advisor.slot.clone().unwrap_or_default(),
            idea_token: advisor.idea_token.clone().unwrap_or_default(),
            traits: advisor.traits.clone(),
            cost: advisor.cost,
            ledger: advisor.ledger,
            modifiers: advisor.modifiers.clone(),
            research_bonuses: advisor.research_bonuses.clone(),
        })
        .collect()
}

fn parse_file(
    input: &[u8],
    names: &KeyNames,
) -> io::Result<(Characters, Option<String>, TemplateRoles)> {
    let (read, stop) = read_file(input, names);
    let mut result = Characters {
        notes: read
            .misreads
            .iter()
            .map(|note| (note.kind, note.to_string()))
            .collect(),
        ..Characters::default()
    };
    let (characters, stop) = characters_of(&read.items, stop);
    if let Some((at, reason)) = &stop {
        let refused: Vec<&str> = characters[(*at).min(characters.len())..]
            .iter()
            .map(|(token, _)| token.as_str())
            .collect();
        result.notes.push((
            content_reader_parser::DiagnosticKind::Unsettled,
            format!(
                "{reason}; the rest of the file cannot be determined; refused characters: {}",
                if refused.is_empty() {
                    "none".to_owned()
                } else {
                    refused.join(", ")
                }
            ),
        ));
    }
    let known = stop.as_ref().map_or(characters.len(), |(at, _)| *at);
    let mut roles = TemplateRoles::new();
    let mut defaulted = BTreeSet::new();
    let mut scans = modifiers::ScanFailures::distinct();
    for (token, alternatives) in characters.into_iter().take(known) {
        let token = token.as_str();
        if alternatives
            .iter()
            .any(|template| template.unconditional_startup)
        {
            result.unconditional_startup.insert(token.to_owned());
        }
        let mut advisors = BTreeMap::new();
        let mut ambiguous = BTreeSet::new();
        let mut leaders = BTreeMap::new();
        let mut ambiguous_leaders = BTreeSet::new();
        let mut alternative_roles = Vec::with_capacity(alternatives.len());
        for template in &alternatives {
            for advisor in &template.advisors {
                if advisor.idea_token.is_none() || advisor.slot.is_none() {
                    defaulted.insert(format!("{token}: advisor idea_token or slot missing"));
                }
                scans.modifiers(token, &advisor.modifiers);
                scans.scan(token, &advisor.research_bonuses);
            }
            let template_roles = advisor_roles(token, template);
            for role in &template_roles {
                if role.idea_token.is_empty() || role.slot.is_empty() {
                    continue;
                }
                let identity = role.idea_token.to_ascii_lowercase();
                result
                    .advisor_spellings
                    .entry(identity.clone())
                    .or_insert_with(|| role.idea_token.clone());
                agree_role(&mut advisors, &mut ambiguous, identity, role.clone());
            }
            let template_leaders = leader_roles(token, template);
            agree_role(
                &mut result.leader_sets,
                &mut result.ambiguous_leader_sets,
                token.to_owned(),
                template_leaders.clone(),
            );
            alternative_roles.push(TaggedRoles {
                tag: token.to_owned(),
                advisors: template_roles,
            });
            for role in template_leaders {
                if role.ideology.is_empty() {
                    defaulted.insert(format!(
                        "{token}: country_leader ideology missing (native logs an invalid ideology)"
                    ));
                    agree_role(
                        &mut leaders,
                        &mut ambiguous_leaders,
                        (token.to_owned(), String::new()),
                        role,
                    );
                    continue;
                }
                let key = (token.to_owned(), role.ideology.clone());
                agree_role(&mut leaders, &mut ambiguous_leaders, key, role);
            }
        }
        merge(
            &mut result.advisors,
            &mut result.ambiguous_advisors,
            advisors,
            ambiguous,
        );
        merge(
            &mut result.leaders,
            &mut result.ambiguous_leaders,
            leaders,
            ambiguous_leaders,
        );
        let name = agreed(&alternatives, |template| {
            template
                .name
                .clone()
                .map_or(Authored::Missing, Authored::Asset)
        });
        result
            .names
            .entry(token.to_owned())
            .and_modify(|previous| previous.agree(name.clone()))
            .or_insert(name);
        let portraits = result.portraits.entry(token.to_owned()).or_default();
        for category in PORTRAIT_ROLES {
            for size in PORTRAIT_SIZES {
                let portrait = agreed(&alternatives, |template| {
                    template.portrait_of(category, size)
                });
                portraits
                    .entry((category.to_owned(), size.to_owned()))
                    .and_modify(|previous| previous.agree(portrait.clone()))
                    .or_insert(portrait);
            }
        }
        roles.push((token.to_owned(), alternative_roles));
    }
    if !defaulted.is_empty() {
        result.notes.push((
            content_reader_parser::DiagnosticKind::Warning,
            format!(
                "{} character roles kept with native role defaults: {}",
                defaulted.len(),
                defaulted.into_iter().collect::<Vec<_>>().join("; ")
            ),
        ));
    }
    if let Some(warning) = scans.warning() {
        result.notes.push((content_reader_parser::DiagnosticKind::Misread, warning));
    }
    Ok((result, stop.map(|(_, cause)| cause), roles))
}

fn agree_role<K: Ord, R: PartialEq>(
    found: &mut BTreeMap<K, R>,
    ambiguous: &mut BTreeSet<K>,
    key: K,
    role: R,
) {
    if ambiguous.contains(&key) {
        return;
    }
    match found.get(&key) {
        Some(existing) if *existing != role => {
            found.remove(&key);
            ambiguous.insert(key);
        }
        Some(_) => {}
        None => {
            found.insert(key, role);
        }
    }
}

fn merge<K: Ord + Clone, R: PartialEq>(
    found: &mut BTreeMap<K, R>,
    ambiguous: &mut BTreeSet<K>,
    more: BTreeMap<K, R>,
    more_ambiguous: BTreeSet<K>,
) {
    for (key, value) in more {
        if ambiguous.contains(&key) {
            continue;
        }
        match found.get(&key) {
            Some(existing) if *existing != value => {
                found.remove(&key);
                ambiguous.insert(key);
            }
            Some(_) => {}
            None => {
                found.insert(key, value);
            }
        }
    }
    for key in more_ambiguous {
        found.remove(&key);
        ambiguous.insert(key);
    }
}

pub fn tokens(layers: &ResolvedLayers) -> io::Result<BTreeSet<Vec<u8>>> {
    let mut tokens = BTreeSet::new();
    for path in paths(layers)? {
        let read = native::read(
            &path.read(layers)?,
            native::Mode::SkipMark,
            native::Top::File(false),
            None,
            &mut |table, field| match table {
                true => Shape::SkipIfOpen,
                false if field.keyword_is("characters") => Shape::Object(true),
                false => Shape::Skip,
            },
        );
        for table in read
            .items
            .iter()
            .filter(|item| item.keyword_is("characters"))
        {
            tokens.extend(table.value.items().unwrap_or_default().iter().map(|item| {
                parser::c_string(item.key.as_deref().unwrap_or_default()).to_ascii_lowercase()
            }));
        }
    }
    Ok(tokens)
}

pub fn paths(layers: &ResolvedLayers) -> io::Result<Vec<content_reader_layers::script_files::ScriptPath>> {
    content_reader_layers::script_files::paths(layers, "common/characters/", Some(".txt"))
}

impl Characters {
    pub fn register(&mut self, other: &Characters) {
        for token in other.names.keys() {
            self.character_coverage.add(token);
        }
        for idea in other.advisors.keys().chain(&other.ambiguous_advisors) {
            self.advisor_coverage.add(idea);
        }
    }

    pub fn next_file(&mut self) {
        self.advisor_coverage.next_file();
        self.character_coverage.next_file();
    }

    pub fn gap(&mut self, path: &str, cause: &str) {
        self.advisor_coverage.gap(path, cause);
        self.character_coverage.gap(path, cause);
    }

    pub fn merge(&mut self, other: Characters) {
        for (identity, spelling) in other.advisor_spellings {
            self.advisor_spellings.entry(identity).or_insert(spelling);
        }
        self.unconditional_startup
            .extend(other.unconditional_startup);
        merge(
            &mut self.advisors,
            &mut self.ambiguous_advisors,
            other.advisors,
            other.ambiguous_advisors,
        );
        merge(
            &mut self.leaders,
            &mut self.ambiguous_leaders,
            other.leaders,
            other.ambiguous_leaders,
        );
        merge(
            &mut self.leader_sets,
            &mut self.ambiguous_leader_sets,
            other.leader_sets,
            other.ambiguous_leader_sets,
        );
        for (token, name) in other.names {
            self.names
                .entry(token)
                .and_modify(|previous| previous.agree(name.clone()))
                .or_insert(name);
        }
        for (token, portraits) in other.portraits {
            let entry = self.portraits.entry(token).or_default();
            for (key, portrait) in portraits {
                entry
                    .entry(key)
                    .and_modify(|previous| previous.agree(portrait.clone()))
                    .or_insert(portrait);
            }
        }
        for (site, templates) in other.built {
            self.built.entry(site).or_default().extend(templates);
        }
        self.unplaced.extend(other.unplaced);
    }

    fn place(&mut self, path: &ScriptPath, input: &[u8], roles: TemplateRoles) {
        let templates = match parse_leader_templates(path, input) {
            Ok(templates) => templates,
            Err(error) => {
                self.unplaced.insert(
                    path.as_bytes().to_vec(),
                    format!("its lines cannot be read: {error}"),
                );
                return;
            }
        };
        let mut templates = templates.into_iter();
        for (token, alternatives) in roles {
            for template_roles in alternatives {
                let site = templates.next();
                let line = match &site {
                    Some(site) if site.tag == token => site.line,
                    other => {
                        self.unplaced.insert(
                            path.as_bytes().to_vec(),
                            format!(
                                "character {token}'s template is read as {} in the database's order",
                                other.as_ref().map_or("none".to_owned(), |site| format!(
                                    "character {}'s",
                                    site.tag
                                ))
                            ),
                        );
                        return;
                    }
                };
                match line {
                    Some(line) => self
                        .built
                        .entry((path.as_bytes().to_vec(), line))
                        .or_default()
                        .push(template_roles),
                    None => {
                        self.unplaced.insert(
                            path.as_bytes().to_vec(),
                            format!("character {token} has a template the database never reads"),
                        );
                        return;
                    }
                }
            }
        }
    }
}

pub fn officer_template(
    token: &str,
    group: &str,
    fields: &[Item],
) -> Result<Characters, String> {
    let mut advisor = AdvisorFields {
        slot: Some(group.to_owned()),
        ..AdvisorFields::default()
    };
    advisor.apply(fields);
    if let Some(cause) = advisor.unheld.cause() {
        return Err(format!("advisor {token}: {cause}"));
    }
    let name = advisor
        .name
        .take()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| token.to_owned());
    let portrait = format!("GFX_idea_{name}");
    let mut template = Characters::default();
    template.advisors.insert(
        token.to_ascii_lowercase(),
        AdvisorRole {
            character: token.to_owned(),
            slot: advisor.slot.unwrap_or_default(),
            idea_token: token.to_owned(),
            traits: advisor.traits,
            cost: advisor.cost,
            ledger: advisor.ledger,
            modifiers: advisor.modifiers,
            research_bonuses: advisor.research_bonuses,
        },
    );
    template
        .advisor_spellings
        .insert(token.to_ascii_lowercase(), token.to_owned());
    template
        .names
        .insert(token.to_owned(), Authored::Asset(name));
    template.portraits.insert(
        token.to_owned(),
        BTreeMap::from([(
            ("army".to_owned(), "small".to_owned()),
            Authored::Asset(portrait),
        )]),
    );
    Ok(template)
}

pub fn read(
    layers: &ResolvedLayers,
    templates: Characters,
) -> io::Result<(Characters, Vec<content_reader_parser::Diagnostic>)> {
    let mut result = templates;
    let mut warnings = Vec::new();
    let names = layers.key_names()?;
    for listed in paths(layers)? {
        let path = listed.to_string();
        result.next_file();
        let parsed =
            layers.recorded_product("character file templates", 1, listed.as_bytes(), |layers| {
                let input = listed.read(layers)?;
                Ok(parse_file(&input, &names)
                    .map(|(mut file, stop, roles)| {
                        file.place(&listed, &input, roles);
                        (file, stop)
                    })
                    .map_err(|error| error.to_string()))
            })?;
        match parsed.as_ref() {
            Ok((file, stop)) => {
                let mut file = file.clone();
                warnings.extend(
                    std::mem::take(&mut file.notes)
                        .into_iter()
                        .map(|(kind, note)| (kind, format!("Characters in {path}: {note}."))),
                );
                result.register(&file);
                if let Some(cause) = stop {
                    result.gap(&path, cause);
                }
                result.merge(file);
            }
            Err(error) => {
                warnings.push((
                    content_reader_parser::DiagnosticKind::Refused,
                    format!("Characters unavailable in {path}: {error}"),
                ));
                result.gap(&path, &error.to_string());
            }
        }
    }
    Ok((result, warnings))
}

const UNIT_LEADER_ROLES: [&str; 3] = ["corps_commander", "field_marshal", "navy_leader"];

fn is_key(item: &Item, key: &str) -> bool {
    item.keyword_is(key)
}

#[derive(Clone, Debug, PartialEq)]
pub struct LeaderTemplate {
    pub path: ScriptPath,
    pub tag: String,
    pub line: Option<u32>,
    pub role: Result<Option<Triggers>, String>,
}

fn character_templates(
    path: &ScriptPath,
    tag: &str,
    value: &Value,
    lines: &BTreeMap<*const Value, parser::BlockLines>,
    roles: &[&str],
) -> Vec<LeaderTemplate> {
    let template = |line: Option<u32>, role| LeaderTemplate {
        path: path.clone(),
        tag: tag.to_owned(),
        line,
        role,
    };
    let Some(items) = value.items() else {
        return Vec::new();
    };
    let leading = items
        .iter()
        .take_while(|item| is_key(item, "instance"))
        .count();
    if leading == 0 {
        let body: Vec<Item> = items
            .iter()
            .filter(|item| !is_key(item, "instance"))
            .cloned()
            .collect();
        let line = lines
            .get(&std::ptr::from_ref::<Value>(value))
            .map(|lines| lines.first_token);
        return vec![template(line, template_role_for(&body, roles))];
    }
    let trailing = leading < items.len();
    items[..leading]
        .iter()
        .map(|item| {
            if trailing {
                return template(None, Err("keys follow its instance blocks".into()));
            }
            let Some(body) = item.value.items() else {
                return template(None, Err("an instance is not a block".into()));
            };
            match body.split_first() {
                Some((allowed, rest)) if is_key(allowed, "allowed") => {
                    let line = lines
                        .get(&(&raw const allowed.value))
                        .map(|lines| lines.close);
                    match line {
                        Some(line) => template(Some(line), template_role_for(rest, roles)),
                        None => template(None, Err("its allowed is not a block".into())),
                    }
                }
                _ => template(None, Ok(None)),
            }
        })
        .collect()
}

fn template_role_for(items: &[Item], roles: &[&str]) -> Result<Option<Triggers>, String> {
    let Some(role) = items
        .iter()
        .rfind(|item| roles.iter().any(|key| is_key(item, key)))
    else {
        return Ok(None);
    };
    let role = role.value.items().ok_or_else(|| {
        if roles == UNIT_LEADER_ROLES {
            "its unit leader role is not a block".to_owned()
        } else {
            "its scientist role is not a block".to_owned()
        }
    })?;
    let mut visible = Vec::new();
    for item in role.iter().filter(|item| is_key(item, "visible")) {
        let block = triggers::body(&item.value).ok_or_else(|| {
            if roles == UNIT_LEADER_ROLES {
                "its unit leader role's visible is not a block".to_owned()
            } else {
                "its scientist role's visible is not a block".to_owned()
            }
        })?;
        visible.extend(block);
    }
    Ok(Some(visible))
}

fn index_lines(
    items: &[Item],
    spans: &[parser::BlockLines],
    next: &mut usize,
    out: &mut BTreeMap<*const Value, parser::BlockLines>,
) {
    for item in items {
        if let Some(children) = item.value.items() {
            if let Some(&span) = spans.get(*next) {
                out.insert(&raw const item.value, span);
            }
            *next += 1;
            index_lines(children, spans, next, out);
        }
    }
}

pub fn parse_leader_templates(path: &ScriptPath, input: &[u8]) -> io::Result<Vec<LeaderTemplate>> {
    parse_visibility_templates(path, input, &UNIT_LEADER_ROLES)
}
fn parse_visibility_templates(
    path: &ScriptPath,
    input: &[u8],
    roles: &[&str],
) -> io::Result<Vec<LeaderTemplate>> {
    let (document, spans, _) = parser::parse_unclosed_lines(input)?;
    let mut lines = BTreeMap::new();
    let mut next = 0;
    index_lines(&document.items, &spans, &mut next, &mut lines);
    if next != spans.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{path}: {next} blocks but {} spans", spans.len()),
        ));
    }
    let mut templates = Vec::new();
    for root in document
        .items
        .iter()
        .filter(|item| is_key(item, "characters"))
    {
        let Some(characters) = root.value.items() else {
            continue;
        };
        for character in characters {
            let Some(tag) = character.key.as_deref() else {
                continue;
            };
            let tag = String::from_utf8_lossy(tag);
            templates.extend(character_templates(
                path,
                &tag,
                &character.value,
                &lines,
                roles,
            ));
        }
    }
    Ok(templates)
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScientistVisibilityTemplate {
    pub line: u32,
    pub triggers: Result<Triggers, String>,
}

pub fn parse_scientist_visibility_templates(
    input: &[u8],
) -> io::Result<Vec<ScientistVisibilityTemplate>> {
    parse_role_visibility_templates(input, &["scientist"])
}

pub fn parse_leader_visibility_templates(
    input: &[u8],
) -> io::Result<Vec<ScientistVisibilityTemplate>> {
    parse_role_visibility_templates(input, &UNIT_LEADER_ROLES)
}

fn parse_role_visibility_templates(
    input: &[u8],
    roles: &[&str],
) -> io::Result<Vec<ScientistVisibilityTemplate>> {
    let (document, spans, _) = parser::parse_unclosed_lines(input)?;
    let mut lines = BTreeMap::new();
    let mut next = 0;
    index_lines(&document.items, &spans, &mut next, &mut lines);
    if next != spans.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "role visibility block lines differ",
        ));
    }
    fn visit(
        items: &[Item],
        roles: &[&str],
        lines: &BTreeMap<*const Value, parser::BlockLines>,
        out: &mut Vec<ScientistVisibilityTemplate>,
    ) {
        for item in items {
            let Some(body) = item.value.items() else {
                continue;
            };
            if roles.iter().any(|role| is_key(item, role))
                && let Some(last) = body.iter().rfind(|item| is_key(item, "visible"))
                && let Some(line) = lines.get(&(&raw const last.value))
            {
                out.push(ScientistVisibilityTemplate {
                    line: line.open,
                    triggers: template_role_for(std::slice::from_ref(item), roles)
                        .and_then(|role| role.ok_or_else(|| "role missing".into())),
                });
            }
            visit(body, roles, lines, out);
        }
    }
    let mut out = Vec::new();
    visit(&document.items, roles, &lines, &mut out);
    Ok(out)
}

pub fn leader_templates(layers: &ResolvedLayers) -> Result<Vec<LeaderTemplate>, String> {
    visibility_templates(layers, &UNIT_LEADER_ROLES)
}
fn visibility_templates(
    layers: &ResolvedLayers,
    roles: &[&str],
) -> Result<Vec<LeaderTemplate>, String> {
    let paths = layers
        .directory_files("common/characters", ".txt")
        .map_err(|error| format!("common/characters cannot be listed: {error}"))?;
    let mut templates = Vec::new();
    for path in paths {
        let bytes = path
            .read_uncovered(layers)
            .map_err(|error| format!("{path} cannot be read: {error}"))?;
        templates.extend(
            parse_visibility_templates(&path, &bytes, roles)
                .map_err(|error| format!("{path} cannot be parsed: {error}"))?,
        );
    }
    Ok(templates)
}

#[cfg(test)]
mod tests {
    #[test]
    fn hard_character_reads_refuse_the_catalogue_with_or_without_cached_names() {
        content_reader_script::test_game::enter();
        for cached_names in [false, true] {
            let root = std::env::temp_dir().join(format!(
                "characters-hard-read-{}-{cached_names}", std::process::id()
            ));
            std::fs::create_dir_all(root.join("common/characters")).unwrap();
            let path = root.join("common/characters/a.txt");
            std::fs::write(&path, b"characters = { original = { name = original } }").unwrap();
            let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
            if cached_names {
                layers.key_names().unwrap();
            }
            std::fs::remove_file(&path).unwrap();
            std::fs::create_dir(&path).unwrap();
            assert!(super::read(&layers, super::Characters::default()).is_err());
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn scientist_visible_sources_include_effect_roles_and_append_repeated_visible_blocks() {
        let templates = super::parse_scientist_visibility_templates(
            br#"
characters={ original={scientist={visible={always=yes}}} }
add_scientist_role={scientist={
 visible={always=no}
 visible={is_ai=yes}
}}
"#,
        )
        .unwrap();
        assert_eq!(templates.len(), 2);
        assert_eq!((templates[0].line, templates[1].line), (2, 5));
        let triggers = templates[1].triggers.as_ref().unwrap();
        assert_eq!(triggers.len(), 2);
        assert_eq!(triggers[0].value.read_string(), Some(&b"no"[..]));
        let leaders = super::parse_leader_visibility_templates(
            br#"
effect={generate_character={
 field_marshal={visible={has_war=yes}}
 scientist={visible={always=no}}
 navy_leader={
  visible=yes
 }
}}
"#,
        )
        .unwrap();
        assert_eq!(leaders.iter().map(|template| template.line).collect::<Vec<_>>(), [3]);
        assert_eq!(leaders[0].triggers.as_ref().unwrap().len(), 1);
        let refused = super::parse_leader_visibility_templates(b"corps_commander={\nvisible=yes\nvisible={ always=no }\n}").unwrap();
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].line, 3);
        assert_eq!(refused[0].triggers, Err("its unit leader role's visible is not a block".into()));
    }

    use super::*;

    #[test]
    fn scientist_visibility_uses_its_own_role_and_retains_instance_origin() {
        let input=b"characters={c={field_marshal={visible={always=no}} scientist={visible={always=yes} VISIBLE={has_country_flag=mod_science}}} d={instance={allowed={} scientist={}}}}";
        let templates =
            parse_visibility_templates(&"common/characters/mod.txt".into(), input, &["scientist"])
                .unwrap();
        assert_eq!(templates.len(), 2);
        assert_eq!(
            templates[0].role.as_ref().unwrap().as_ref().unwrap().len(),
            2
        );
        assert!(
            templates[1]
                .role
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .is_empty()
        );
        assert_eq!(templates[0].path, "common/characters/mod.txt");
        assert!(templates.iter().all(|template| template.line == Some(1)));
    }

    #[test]
    fn advisor_case_aliases_preserve_first_spelling_and_role_identity() {
        content_reader_script::test_game::enter();
        let mut characters = parse(br#"characters = {
            mod_holder = { advisor = { slot = mod_slot idea_token = Joseph_Votel traits = { mod_Trait } } }
        }"#, &KeyNames::default()).unwrap();
        let alias = parse(br#"characters = {
            mod_holder = { advisor = { slot = MOD_SLOT idea_token = joseph_votel traits = { MOD_TRAIT } } }
        }"#, &KeyNames::default()).unwrap();
        characters.merge(alias);
        let found = characters.advisor("JOSEPH_VOTEL").unwrap().unwrap();
        assert_eq!(found.idea_token, "Joseph_Votel");
        assert_eq!(found.slot, "mod_slot");
        assert_eq!(found.traits, ["mod_Trait"]);
        assert!(!characters.ambiguous_advisor("jOsEpH_vOtEl"));
        assert_eq!(
            characters.advisor_tokens().0.collect::<Vec<_>>(),
            ["Joseph_Votel"]
        );
        let characters = parse("characters = { a = { advisor = { slot = seat idea_token = É_role } } b = { advisor = { slot = seat idea_token = é_role } } }".as_bytes(), &KeyNames::default()).unwrap();
        assert_eq!(
            characters.advisor("É_ROLE").unwrap().unwrap().idea_token,
            "É_role"
        );
        assert_eq!(
            characters.advisor("é_ROLE").unwrap().unwrap().idea_token,
            "é_role"
        );
        let mut text = String::from("characters = {");
        for index in 0..5_000 {
            text.push_str(&format!(
                "holder_{index} = {{ advisor = {{ slot = seat idea_token = mod_Role_{index} }} }}"
            ));
        }
        text.push('}');
        let characters = parse(text.as_bytes(), &KeyNames::default()).unwrap();
        assert_eq!(characters.advisor_tokens().0.count(), 5_000);
        for index in 0..5_000 {
            let token = format!("MOD_ROLE_{index}");
            assert_eq!(
                characters.advisor(&token).unwrap().unwrap().idea_token,
                format!("mod_Role_{index}")
            );
        }
    }

    #[test]
    fn advisor_case_aliases_share_ambiguity_and_coverage() {
        content_reader_script::test_game::enter();
        let mut characters = parse(br#"characters = {
            mod_holder = {
                instance = { allowed = { always = yes } advisor = { slot = seat idea_token = mod_Role traits = { first } } }
                instance = { allowed = { always = no } advisor = { slot = seat idea_token = MOD_ROLE traits = { second } } }
            }
        }"#, &KeyNames::default()).unwrap();
        assert!(characters.ambiguous_advisor("MOD_role"));
        assert_eq!(characters.advisor("mod_role"), Ok(None));
        assert!(
            characters
                .held_advisor("mOd_RoLe", Err("missing template"))
                .unwrap_err()
                .contains("missing template")
        );
        let mut coverage = Characters::default();
        coverage.register(&characters);
        coverage.next_file();
        coverage.gap("gap.txt", "unread suffix");
        coverage.merge(characters.clone());
        assert_eq!(coverage.advisor("mOd_RoLe").unwrap_err().path, "gap.txt");
        characters.advisor_coverage.next_file();
        characters
            .advisor_coverage
            .gap("early.txt", "unread suffix");
        characters.next_file();
        let known = parse(
            br#"characters = { x = { advisor = { slot = seat idea_token = later_Role } } }"#,
            &KeyNames::default(),
        )
        .unwrap();
        characters.register(&known);
        characters.merge(known);
        assert_eq!(
            characters.advisor("LATER_ROLE").unwrap_err().path,
            "early.txt"
        );
        assert!(characters.advisor("absent").is_err());
    }

    #[test]
    fn country_leader_sets_keep_overrides_and_alternative_identity() {
        content_reader_script::test_game::enter();
        let characters = parse(
            br#"characters = {
            single = { country_leader = { ideology = left traits = { old } }
                       country_leader = { ideology = left traits = { final } } }
            split = { instance = { allowed = { always = yes }
                                  country_leader = { ideology = left traits = { a } } }
                      instance = { allowed = { always = yes }
                                  country_leader = { ideology = right traits = { b } } } }
            absent = { name = absent }
        }"#,
            &KeyNames::default(),
        )
        .unwrap();
        let single = characters.leader_roles("single").unwrap().unwrap();
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].traits, ["final"]);
        assert_eq!(
            characters.leader("single", "left").unwrap().unwrap().traits,
            ["final"]
        );
        assert!(
            characters
                .leader_roles("split")
                .unwrap_err()
                .contains("unrecorded template tag-map winner")
        );
        assert!(
            characters
                .leader_roles("absent")
                .unwrap()
                .unwrap()
                .is_empty()
        );
        assert_eq!(characters.leader_roles("missing").unwrap(), None);
    }

    #[test]
    fn agreeing_template_projection_does_not_prove_startup_presence() {
        content_reader_script::test_game::enter();
        let mut characters = parse(
            br#"characters = {
                conditional = { instance = { allowed = { always = no }
                    name = same country_leader = { ideology = left } }
                    instance = { allowed = { always = no }
                    name = same country_leader = { ideology = left } } }
                empty_allowed = { instance = { allowed = { } name = present } }
                plain = { name = present }
            }"#,
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(
            characters
                .leader_roles("conditional")
                .unwrap()
                .unwrap()
                .len(),
            1
        );
        assert!(!characters.template_has_unconditional_startup("conditional"));
        assert!(!characters.template_has_unconditional_startup("missing"));
        assert!(characters.template_has_unconditional_startup("empty_allowed"));
        assert!(characters.template_has_unconditional_startup("plain"));
        characters.merge(parse(
            b"characters = { conditional = { name = same country_leader = { ideology = left } } }",
            &KeyNames::default(),
        ).unwrap());
        assert!(characters.template_has_unconditional_startup("conditional"));
    }

    #[test]
    fn effect_role_projections_share_character_role_field_application() {
        content_reader_script::test_game::enter();
        let fields = parser::parse(b"ideology = left traits = { a } traits = { b }")
            .unwrap()
            .items;
        assert_eq!(
            country_leader_role("c", &fields).unwrap(),
            LeaderRole {
                character: "c".into(),
                ideology: "left".into(),
                traits: vec!["a".into(), "b".into()],
            }
        );
        let fields = parser::parse(b"slot = theorist idea_token = c_idea traits = { a } traits = { b } cost = 2 cost = invalid modifier = { research_speed_factor = 0.1 } modifier = { research_speed_factor = 0.2 }").unwrap().items;
        let role = advisor_role("c", &fields).unwrap();
        assert_eq!(role.cost, Some(200_000));
        assert_eq!(role.traits, ["a", "b"]);
        assert_eq!(role.modifiers.values[0].value, 20_000);
    }

    #[test]
    fn repeated_character_alternatives_count_each_problem_once() {
        content_reader_script::test_game::enter();
        let (characters, _, _) = parse_file(
            b"characters = { mod_character = { \
            advisor = { modifier = { mod_bonus = invalid } } \
            country_leader = { } \
            instance = { allowed = { always = yes } } \
            instance = { allowed = { always = no } } } }",
            &KeyNames::default(),
        )
        .unwrap();
        let roles = characters
            .notes
            .iter()
            .find(|note| note.1.contains("native role defaults"))
            .unwrap();
        assert_eq!(roles.0, content_reader_parser::DiagnosticKind::Warning);
        assert!(
            roles
                .1
                .starts_with("2 character roles kept with native role defaults"),
            "{roles:?}"
        );
        let scans = characters
            .notes
            .iter()
            .find(|note| note.1.contains("numeric conversion"))
            .unwrap();
        assert!(scans.1.starts_with("1 content values failed"), "{scans:?}");
        assert_eq!(
            scans
                .1
                .matches("mod_character: mod_bonus = invalid")
                .count(),
            1
        );
    }

    #[test]
    fn missing_role_identity_stays_in_template() {
        content_reader_script::test_game::enter();
        let (mut characters, _, roles) = parse_file(
            b"characters={c={advisor={slot=army_chief} country_leader={desc=only_desc}}}",
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(roles[0].1[0].advisors[0].idea_token, "");
        assert_eq!(roles[0].1[0].advisors[0].slot, "army_chief");
        assert!(characters.advisor("").unwrap().is_none());
        assert_eq!(characters.leader("c", "").unwrap().unwrap().ideology, "");
        assert!(characters.notes[0].1.contains("native role defaults"));
        let file = b"common/characters/\x80test.txt";
        characters.place(
            &ScriptPath::new(file.to_vec()),
            b"characters={c={advisor={slot=army_chief} country_leader={desc=only_desc}}}",
            roles,
        );
        assert!(!characters.built.is_empty());
        assert!(characters.built.keys().all(|(path, _)| path == file));
    }

    const FILE: &[u8] = br#"characters = {
        GER_schacht = { name = GER_schacht portraits = { civilian = { large = GFX_schacht_large small = GFX_idea_schacht } }
            advisor = { slot = political_advisor idea_token = GER_schacht_idea cost = 150 traits = { captain_of_industry } ledger = civilian } }
        GER_hitler = { portraits = { civilian = { large = GFX_hitler } }
            country_leader = { ideology = nazism traits = { dictator } } }
        GER_split = {
            instance = { allowed = { has_dlc = "A" } portraits = { army = { small = GFX_a } }
                advisor = { slot = army_chief idea_token = GER_split_idea traits = { a_trait } } }
            instance = { allowed = { NOT = { has_dlc = "A" } } portraits = { army = { small = GFX_b } }
                advisor = { slot = army_chief idea_token = GER_split_idea traits = { b_trait } } } }
        GER_same = {
            instance = { allowed = { always = yes } portraits = { army = { large = GFX_same } } }
            instance = { allowed = { always = yes } portraits = { army = { large = GFX_same } } } } }"#;

    fn portrait(characters: &Characters, token: &str, category: &str, size: &str) -> Authored {
        characters.portraits(token).unwrap().unwrap()[&(category.to_owned(), size.to_owned())]
            .clone()
    }

    fn name(characters: &Characters, token: &str) -> Option<Authored> {
        characters.name(token).unwrap().cloned()
    }

    #[test]
    fn advisors_and_leaders_keep_their_roles_and_traits() {
        content_reader_script::test_game::enter();
        let characters = parse(FILE, &KeyNames::default()).unwrap();
        assert!(characters.notes.is_empty(), "{:?}", characters.notes);
        assert_eq!(
            characters.declared_tokens().collect::<Vec<_>>(),
            ["GER_hitler", "GER_same", "GER_schacht", "GER_split"]
        );
        let schacht = characters.advisor("GER_schacht_idea").unwrap().unwrap();
        assert_eq!(
            (
                schacht.character.as_str(),
                schacht.slot.as_str(),
                schacht.cost
            ),
            ("GER_schacht", "political_advisor", Some(15_000_000))
        );
        assert_eq!(schacht.traits, ["captain_of_industry"]);
        let leader = characters.leader("GER_hitler", "nazism").unwrap().unwrap();
        assert_eq!(leader.traits, ["dictator"]);
        assert_eq!(
            name(&characters, "GER_schacht"),
            Some(Authored::Asset("GER_schacht".into()))
        );
    }

    #[test]
    fn a_disagreeing_advisor_role_is_its_built_templates() {
        content_reader_script::test_game::enter();
        let root =
            std::env::temp_dir().join(format!("content-reader-built-advisor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "common/characters/a.txt",
            "characters = {\n\
             \tGER_split = {\n\
             \t\tINSTANCE = {\n\
             \t\t\tAllowed = { has_dlc = \"A\" }\n\
             \t\t\tAdvisor = { slot = army_chief idea_token = GER_split_idea traits = { a_trait } }\n\
             \t\t}\n\
             \t\tinstance = {\n\
             \t\t\tallowed = {\n\
             \t\t\t\tNOT = { has_dlc = \"A\" }\n\
             \t\t\t}\n\
             \t\t\tadvisor = { SLOT = army_chief IDEA_TOKEN = GER_split_idea Traits = { b_trait } cost = 2 }\n\
             \t\t}\n\
             \t}\n\
             \tGER_plain = { advisor = { slot = army_chief idea_token = GER_plain_idea } }\n\
             }\n",
        );
        write(
            "common/characters/b.txt",
            "characters = { GER_plain = { advisor = { slot = army_chief idea_token = GER_plain_idea traits = { other } } }              GER_fixed = { advisor = { slot = army_chief idea_token = GER_fixed_idea } } }",
        );
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let (characters, warnings) = read(&layers, Characters::default()).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let built = |file: &str, line, tag: &str| BuiltTemplate {
            tag: tag.to_owned(),
            file: file.as_bytes().to_vec(),
            line,
            generated: false,
        };
        let traits = |found: Result<Option<&AdvisorRole>, String>| {
            found.map(|role| role.map(|role| role.traits.clone()))
        };
        let a = "common/characters/a.txt";
        assert_eq!(characters.advisor("GER_split_idea"), Ok(None));
        assert_eq!(
            traits(characters.held_advisor("GER_split_idea", Ok(&built(a, 4, "GER_split")))),
            Ok(Some(vec!["a_trait".to_owned()]))
        );
        let second = characters
            .held_advisor("ger_SPLIT_idea", Ok(&built(a, 10, "ger_split")))
            .unwrap()
            .unwrap();
        assert_eq!(
            (second.traits.as_slice(), second.cost),
            (["b_trait".to_owned()].as_slice(), Some(200_000))
        );
        assert_eq!(
            traits(characters.held_advisor(
                "GER_plain_idea",
                Ok(&built("common/characters/b.txt", 1, "GER_plain"))
            )),
            Ok(Some(vec!["other".to_owned()]))
        );
        assert_eq!(
            traits(characters.held_advisor("GER_plain_idea", Ok(&built(a, 14, "GER_plain")))),
            Ok(Some(Vec::new()))
        );
        assert!(matches!(
            characters.held_advisor("GER_fixed_idea", Err("no holder")),
            Ok(Some(_))
        ));
        let refusal = |found: Result<Option<&AdvisorRole>, String>| found.unwrap_err();
        for (template, cause) in [
            (
                Ok(built(a, 5, "GER_split")),
                "no content character template",
            ),
            (
                Ok(built(a, 4, "GER_plain")),
                "no content character template",
            ),
            (
                Ok(BuiltTemplate {
                    generated: true,
                    ..built("", -1, "GER_split")
                }),
                "generate_character",
            ),
            (Err("no holder"), "no holder"),
        ] {
            let reason = refusal(
                characters.held_advisor("GER_split_idea", template.as_ref().map_err(|r| *r)),
            );
            assert!(
                reason.starts_with("advisor GER_split_idea:") && reason.contains(cause),
                "{reason}"
            );
        }
    }

    #[test]
    fn alternatives_that_disagree_are_ambiguous_and_agreeing_ones_are_kept() {
        content_reader_script::test_game::enter();
        let characters = parse(FILE, &KeyNames::default()).unwrap();
        assert_eq!(characters.advisor("GER_split_idea"), Ok(None));
        assert!(characters.ambiguous_advisor("GER_split_idea"));
        assert_eq!(
            portrait(&characters, "GER_split", "army", "small"),
            Authored::Ambiguous(BTreeSet::from(["GFX_a".into(), "GFX_b".into()]))
        );
        assert_eq!(
            portrait(&characters, "GER_same", "army", "large"),
            Authored::Asset("GFX_same".into())
        );
        assert_eq!(
            portrait(&characters, "GER_schacht", "civilian", "small"),
            Authored::Asset("GFX_idea_schacht".into())
        );
    }

    #[test]
    fn portraits_of_one_kind_match_the_full_parse() {
        content_reader_script::test_game::enter();
        let (army, stop) = portraits(FILE, &KeyNames::default(), "army", "large").unwrap();
        assert_eq!(stop, None);
        assert_eq!(army["GER_same"], Authored::Asset("GFX_same".into()));
        assert_eq!(army["GER_schacht"], Authored::Missing);
    }

    #[test]
    fn only_agreeing_army_large_alternatives_are_usable() {
        content_reader_script::test_game::enter();
        let (characters, stop) = portraits(
            br#"characters = {
                plain = { portraits = { army = { large = GFX_army small = GFX_small }
                                         civilian = { large = GFX_civilian } } }
                same = { instance = { allowed = { has_dlc = A }
                                      portraits = { army = { large = GFX_army } } }
                         instance = { allowed = { always = yes } portraits = { army = { large = GFX_army } } } }
                different = { instance = { allowed = { always = yes } portraits = { army = { large = GFX_first } } }
                              instance = { allowed = { always = yes } portraits = { army = { large = GFX_second } } } }
                incomplete = { instance = { allowed = { always = yes } portraits = { army = { large = GFX_army } } }
                               instance = { allowed = { always = yes } portraits = { army = { small = GFX_small } } } }
                repeated = { portraits = { ARMY = { LARGE = GFX_first large = GFX_second } } }
                blocks = { portraits = { army = { large = GFX_first } } portraits = { army = { large = GFX_second } } }
                shorthand = { portraits = { army = GFX_short } }
                direct = { portraits = { army = { large = "gfx/leaders/portrait.dds" } } }
                no_army = { portraits = { civilian = { large = GFX_civilian } } }
            }"#,
            &KeyNames::default(),
            "army",
            "large",
        )
        .unwrap();
        assert_eq!(stop, None);
        for token in ["plain", "same"] {
            assert_eq!(characters[token], Authored::Asset("GFX_army".to_owned()));
        }
        assert_eq!(
            characters["different"],
            Authored::Ambiguous(["GFX_first".to_owned(), "GFX_second".to_owned()].into())
        );
        assert_eq!(
            characters["incomplete"],
            Authored::Ambiguous(["GFX_army".to_owned()].into())
        );
        for token in ["repeated", "blocks"] {
            assert_eq!(characters[token], Authored::Asset("GFX_second".to_owned()));
        }
        assert_eq!(
            characters["shorthand"],
            Authored::Asset("GFX_short".to_owned())
        );
        assert_eq!(
            characters["direct"],
            Authored::Asset("gfx/leaders/portrait.dds".to_owned())
        );
        assert_eq!(characters["no_army"], Authored::Missing);
    }

    #[test]
    fn instances_require_leading_allowed_and_refuse_unsettled_suffixes() {
        content_reader_script::test_game::enter();
        let characters = parse(
            br#"characters = { late = { name = own instance = { allowed = { always = yes } name = inner } } }"#, &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(
            name(&characters, "late"),
            Some(Authored::Asset("own".into()))
        );
        for source in [
            br#"characters = { kept = { name = k } a = { instance = { name = x } } b = { name = b } }"#
                .as_slice(),
            br#"characters = { kept = { name = k } a = { instance = { allowed = { always = yes } } name = x } b = { name = b } }"#,
        ] {
            let characters = parse(source, &KeyNames::default()).unwrap();
            assert_eq!(
                characters.declared_tokens().collect::<Vec<_>>(),
                ["kept"],
                "{}",
                String::from_utf8_lossy(source)
            );
            let note = characters.notes.last().unwrap();
            assert!(note.1.contains("refused characters: a, b"), "{note:?}");
            let (found, stop) = portraits(source, &KeyNames::default(), "army", "large").unwrap();
            let cause = stop.unwrap();
            assert_eq!(found["kept"], Authored::Missing);
            assert_eq!(found["a"], Authored::Unknown(cause.clone()));
            assert_eq!(found["b"], Authored::Unknown(cause));
        }
    }

    #[test]
    fn role_fields_apply_in_order() {
        content_reader_script::test_game::enter();
        let characters = parse(
            br#"@slot = other characters = { c = {
                advisor = { slot = theorist idea_token = c_idea cost = 10 cost = wide traits = { a } traits = { b }
                    modifier = { x = 1 } modifier = { X = 2 y = 3 } research_bonus = { land_doctrine = 0.2 } }
                advisor = { slot = @slot idea_token = c_second }
                country_leader = { ideology = nazism traits = { one } traits = { two } } } }"#, &KeyNames::default(),
        )
        .unwrap();
        let advisor = characters.advisor("c_idea").unwrap().unwrap();
        assert_eq!(advisor.cost, Some(1_000_000));
        assert_eq!(advisor.traits, ["a", "b"]);
        let modifiers: Vec<_> = advisor
            .modifiers
            .values
            .iter()
            .map(|m| (m.name.as_str(), m.value))
            .collect();
        assert_eq!(modifiers, [("x", 200_000), ("y", 300_000)]);
        assert_eq!(advisor.research_bonuses[0].value, 20_000);
        assert_eq!(
            characters.advisor("c_second").unwrap().unwrap().slot,
            "@slot"
        );
        assert_eq!(
            characters.leader("c", "nazism").unwrap().unwrap().traits,
            ["one", "two"]
        );
    }

    #[test]
    fn military_roles_read_as_their_templates_read() {
        content_reader_script::test_game::enter();
        let characters = parse(
            br#"characters = {
                b = { name = B field_marshal = { skill = 3 traits = { t } visible = { always = yes } } name = B2 }
                c = { scientist = { skills = { land = 2 } traits = { t } desc = d } name = C }
                a = { corps_commander = bad name = X portraits = { army = { large = GFX_x } } }
                d = { name = D } }"#, &KeyNames::default(),
        )
        .unwrap();
        assert!(
            characters.notes[0]
                .1
                .contains("field `corps_commander` is `bad`"),
            "{:?}",
            characters.notes
        );
        assert_eq!(name(&characters, "a"), Some(Authored::Missing));
        assert_eq!(
            portrait(&characters, "a", "army", "large"),
            Authored::Missing
        );
        assert_eq!(name(&characters, "d"), None);
        assert_eq!(name(&characters, "b"), Some(Authored::Asset("B2".into())));
        assert_eq!(name(&characters, "c"), Some(Authored::Asset("C".into())));
    }

    #[test]
    fn the_replay_classes_hold_for_characters() {
        content_reader_script::test_game::enter();
        let characters = parse(
            br#"@p = GFX_p characters = {
                quoted = { "name" = quoted NAME = bare PORTRAITS = { ARMY = { LARGE = @p } } }
                block = { name = { portraits = { army = { large = GFX_inner } } } nationality = GER }
                open = { name = open"#, &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(
            name(&characters, "quoted"),
            Some(Authored::Asset("bare".into()))
        );
        assert_eq!(
            portrait(&characters, "quoted", "army", "large"),
            Authored::Asset("GFX_p".into())
        );
        assert_eq!(
            name(&characters, "block"),
            Some(Authored::Asset("{".into()))
        );
        assert_eq!(
            portrait(&characters, "block", "army", "large"),
            Authored::Asset("GFX_inner".into())
        );
        assert!(characters.names.contains_key("nationality"));
        assert!(
            characters.notes[0].1.contains("field `name` is a block"),
            "{:?}",
            characters.notes
        );
        assert_eq!(
            name(&characters, "open"),
            Some(Authored::Asset("open".into()))
        );
    }

    fn conjunction(source: &str) -> Triggers {
        parser::parse(source.as_bytes()).unwrap().items
    }

    #[test]
    fn leader_template_lines_follow_the_first_token_and_instance_allowed_close() {
        let templates = parse_leader_templates(
            &"common/characters/mod.txt".into(),
            br#"characters = {
    next = {
        corps_commander = {}
    }
    blank = {

        navy_leader = {}
    }
    instanced = {
        instance = {
            allowed = {
                always = yes
            }
            field_marshal = {}
        }
    }
}"#,
        )
        .unwrap();
        let places: Vec<_> = templates
            .iter()
            .map(|template| (template.tag.as_str(), template.line))
            .collect();
        assert_eq!(
            places,
            [
                ("next", Some(3)),
                ("blank", Some(7)),
                ("instanced", Some(13))
            ]
        );
        assert!(
            templates
                .iter()
                .all(|template| template.role == Ok(Some(Vec::new())))
        );
    }

    #[test]
    fn leader_templates_follow_the_database_and_its_lines() {
        content_reader_script::test_game::enter();
        let templates = parse_leader_templates(
            &"common/characters/X.txt".into(),
            br#"characters = {
    plain = { field_marshal = { skill = 2 visible = { has_government = neutrality } } }
    joined = {
        Corps_Commander = { visible = { has_dlc = A } VISIBLE = { tag = GER } }
    }
    replaced = { corps_commander = { visible = { tag = GER } } navy_leader = { skill = 1 } }
    advisor_only = { advisor = { slot = army_chief visible = { tag = GER } } }
    split = {
        instance = {
            allowed = {
                has_dlc = A
            }
            field_marshal = { visible = { tag = GER } }
        }
        instance = { allowed = { } advisor = { slot = army_chief } }
        instance = { name = x field_marshal = { } }
    }
    late = { name = late instance = { allowed = { } navy_leader = { } } navy_leader = { visible = { tag = ITA } } }
    trailing = { instance = { allowed = { } navy_leader = { } } name = trailing }
    scalar_role = { corps_commander = yes }
    scalar_visible = { corps_commander = { visible = yes } }
}
other = { ignored = { navy_leader = { } } }
"#,
        )
        .unwrap();
        let found: Vec<_> = templates
            .iter()
            .map(|template| (template.tag.as_str(), template.line))
            .collect();
        assert_eq!(
            found,
            [
                ("plain", Some(2)),
                ("joined", Some(4)),
                ("replaced", Some(6)),
                ("advisor_only", Some(7)),
                ("split", Some(12)),
                ("split", Some(15)),
                ("split", None),
                ("late", Some(18)),
                ("trailing", None),
                ("scalar_role", Some(20)),
                ("scalar_visible", Some(21)),
            ]
        );
        assert!(
            templates
                .iter()
                .all(|t| t.path == "common/characters/X.txt")
        );
        let role = |index: usize| templates[index].role.clone();
        assert_eq!(
            role(0),
            Ok(Some(conjunction("has_government = neutrality")))
        );
        assert_eq!(role(1), Ok(Some(conjunction("has_dlc = A tag = GER"))));
        assert_eq!(role(2), Ok(Some(Triggers::default())));
        assert_eq!(role(3), Ok(None));
        assert_eq!(role(4), Ok(Some(conjunction("tag = GER"))));
        assert_eq!(role(5), Ok(None));
        assert_eq!(role(6), Ok(None));
        assert_eq!(role(7), Ok(Some(conjunction("tag = ITA"))));
        assert!(role(8).is_err());
        assert!(role(9).is_err());
        assert_eq!(
            role(10),
            Err("its unit leader role's visible is not a block".into())
        );
    }
}
