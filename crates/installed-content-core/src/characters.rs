//! Characters (`common/characters`): authored portraits, advisor roles,
//! country leader roles and unit leader `visible` triggers.
//!
//! Conditional `instance` alternatives are retained without evaluation. An
//! authored value is used only when every alternative agrees; disagreements
//! become [`Authored::Ambiguous`], including repeated tokens across files.
//! Disputed advisor roles use the template their recorded holder was built
//! from ([`Characters::held_advisor`]). [`leader_templates`] lists every
//! character template with its stored file and line for that join.
//!
//! Each file has one reader ([`native`]). A body beginning with `instance`
//! reads as alternatives; other bodies read as one template. Fields apply
//! in order within each body or instance; only alternatives are compared.
//!
//! An unread file suffix leaves every character it could define or redefine
//! unknown ([`crate::coverage`]); lookups name the file and cause.

use crate::{
    ResolvedLayers,
    coverage::{Coverage, Gap},
    key_names::KeyNames,
    modifiers::{self, Modifier, Modifiers},
    parser::{
        self, Item, Unheld, Value,
        native::{self, Field, Shape},
    },
    scope::ScopeKind,
    script_files::ScriptPath,
    triggers::{self, Triggers},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

/// Portrait roles in enum order. Unknown roles are refused, and a missing
/// requested role falls back to the lowest role of the requested size.
/// `airforce` identifies an intelligence lane rather than a portrait role.
pub const PORTRAIT_ROLES: [&str; 6] = ["civilian", "army", "navy", "air", "operative", "scientist"];

/// Portrait sizes in enum order: small, then large.
pub const PORTRAIT_SIZES: [&str; 2] = ["small", "large"];

/// One authored value (a portrait sprite name) as every alternative gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Authored {
    Missing,
    Asset(String),
    /// Alternatives that do not agree, with every value any of them names.
    Ambiguous(BTreeSet<String>),
    /// The character's reading cannot be followed, and why.
    Unknown(String),
}

impl Authored {
    /// Every value this can name.
    pub fn names(&self) -> BTreeSet<String> {
        match self {
            Self::Missing | Self::Unknown(_) => BTreeSet::new(),
            Self::Asset(name) => BTreeSet::from([name.clone()]),
            Self::Ambiguous(names) => names.clone(),
        }
    }

    /// Combines another alternative: equal stays, different becomes
    /// ambiguous, and an unknown one leaves the whole unknown.
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

/// The readers a role template's fields go to, which [`advisor_field`]'s
/// callers embed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    Modifier(modifiers::Node),
    /// The role weight.
    Mtth,
}

/// Advisor field dispatch by keyword identity.
pub(crate) fn advisor_field(field: &Field<'_>) -> Shape<Role> {
    match field.keyword().as_deref() {
        // `available`, `visible`, `allowed`, and `do_effect` read AND triggers.
        Some(b"available" | b"visible" | b"allowed" | b"do_effect") => Shape::Trigger,
        // `on_add` and `on_remove` read effects by key in character scope.
        Some(b"on_add" | b"on_remove") => Shape::EffectIn(ScopeKind::Character.bit()),
        // `name`, `slot`, and `idea_token` read the lexer's current token.
        Some(b"name" | b"slot" | b"idea_token") => Shape::Current,
        // Description and picture fields read strings; cost fields use fixed5
        // scans, and action-tooltip and firing flags use boolean scans.
        Some(
            b"desc"
            | b"picture"
            | b"cost"
            | b"removal_cost"
            | b"command_power"
            | b"always_show_on_actions_tooltip"
            | b"can_be_fired",
        ) => Shape::One,
        // Read the ledger value.
        Some(b"ledger") => crate::ideas::ledger_shape(field),
        // `modifier` reads a modifier object; `ai_will_do` reads a weight.
        Some(b"modifier") => Shape::Object(Role::Modifier(modifiers::Node::Modifier)),
        Some(b"ai_will_do") => Shape::Object(Role::Mtth),
        // `traits` and `research_bonus` read lists only on an opening brace;
        // other values use error recovery. Bonus entries use fixed5 scans.
        Some(b"traits") => Shape::ListOrSkip,
        Some(b"research_bonus") => {
            Shape::ObjectOrSkip(Role::Modifier(modifiers::Node::ResearchBonus))
        }
        // Report and skip an unexpected field.
        _ => Shape::Skip,
    }
}

/// Which reader a field goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    /// The character database's file loop.
    Top,
    /// The `characters` loop reads each key as a character body.
    Table,
    /// A character body reads an instance loop when it starts with `instance`,
    /// otherwise one template.
    Body,
    /// An instance reads `allowed` first, then its template fields.
    Instance,
    /// Portrait type dispatch.
    Portraits,
    /// One portrait type reads its size fields.
    Sizes,
    /// Country-leader role fields.
    Leader,
    /// A role template's reader.
    Advisor,
    Role(Role),
    /// Unit-leader role fields for `navy_leader`, `field_marshal`, and
    /// `corps_commander`.
    Military,
    /// Scientist role fields.
    Scientist,
    /// Initial scientist skills.
    ScientistSkills,
}

/// What the dispatch learns while the reader reads.
#[derive(Default)]
struct Reading {
    /// The character being read.
    owner: String,
    /// Portrait keys the game reports as unexpected and skips.
    skipped_portraits: Vec<native::Misread>,
    undecided: Vec<modifiers::Undecided>,
    /// The next field is a character body's, or an instance's, first.
    body_first: bool,
    instance_first: bool,
    /// The body being read started with `instance`.
    instances: bool,
    /// Characters dispatched so far.
    characters: usize,
    /// The first place the reading cannot be followed, and the number of
    /// characters read before the one holding it.
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
            kind: crate::DiagnosticKind::Misread,
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
            // `characters` opens its loop; other roots are reported and skipped.
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
                    // A token other than `instance` ends the instance loop and is consumed
                    // as the body's end.
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
                    // A missing initial `allowed` reports an error and skips the block;
                    // that recovery remains unsettled here.
                    self.stop(
                        field.key.line,
                        "an instance that does not start with `allowed`, which the character-body reader skips through block skipping".to_owned(),
                    );
                    Shape::Skip
                }
            }
            Node::Instance => template_field(field),
            // Each supported portrait role reads size fields on an opening brace;
            // a scalar becomes its large portrait. Unknown roles are reported.
            Node::Portraits => match keyword {
                Some(role) if PORTRAIT_ROLES.iter().any(|known| role == known.as_bytes()) => {
                    Shape::ObjectIfOpen(Node::Sizes)
                }
                _ => {
                    self.skip_portrait_key("role", field);
                    Shape::Skip
                }
            },
            // Recognized sizes read string values; unknown sizes are reported.
            Node::Sizes => match keyword {
                Some(size) if PORTRAIT_SIZES.iter().any(|known| size == known.as_bytes()) => {
                    Shape::One
                }
                _ => {
                    self.skip_portrait_key("size", field);
                    Shape::Skip
                }
            },
            // Read `id` as signed integer; `desc`, `ideology`, and `expire` as
            // strings, with `ideology` using the current token. `traits` reads
            // only an opening list; unknown fields are reported.
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
            // Skill and `legacy_id` fields read signed integers; `desc` reads
            // a string, `visible` an AND trigger, and `traits` an opening list.
            // Unknown fields are reported.
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
            // Read `desc` as a string, `visible` as an AND trigger, `traits` as
            // a dynamic-token list, and `skills` as an initial-skill object.
            // Unknown fields are reported.
            Node::Scientist => match keyword {
                Some(b"desc") => Shape::One,
                Some(b"visible") => Shape::Trigger,
                Some(b"traits") => Shape::List,
                Some(b"skills") => Shape::Object(Node::ScientistSkills),
                _ => Shape::Skip,
            },
            // Each key names a specialization whose value reads as an integer.
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

/// Character-template field dispatch.
fn template_field(field: &Field<'_>) -> Shape<Node> {
    match field.keyword().as_deref() {
        // `available`, `allowed_civil_war`, and `remove` read AND triggers by key.
        Some(b"available" | b"allowed_civil_war" | b"remove") => Shape::Trigger,
        // Read name text, nationality tag, and the capture and generic flags.
        Some(b"name" | b"nationality" | b"can_be_captured" | b"generic") => Shape::One,
        // `gender` reads one value token; invalid values report an error
        // and skip an opening block.
        Some(b"gender") => Shape::Skip,
        // Portrait declarations read into one object.
        Some(b"portraits") => Shape::Object(Node::Portraits),
        // Advisor and country-leader role objects.
        Some(b"advisor") => Shape::Object(Node::Advisor),
        Some(b"country_leader") => Shape::Object(Node::Leader),
        // Unit-leader and scientist roles read objects regardless of
        // whether the value opens a block.
        Some(b"navy_leader" | b"field_marshal" | b"corps_commander") => {
            Shape::Object(Node::Military)
        }
        Some(b"scientist") => Shape::Object(Node::Scientist),
        // Report unexpected fields, including `instance` after the body's
        // first field.
        _ => Shape::Skip,
    }
}

/// Read one character file after ideas, resolving triggers against `names`.
/// Return the reader projection and any unsettled suffix, including which
/// characters were read before it.
pub fn read_file(input: &[u8], names: &KeyNames) -> (native::Read, Option<(usize, String)>) {
    let mut reading = Reading::default();
    // Both character-file passes use lexer mode 1.
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

/// A character body's fields, or its instance fields after each `allowed`.
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

    /// The template of each alternative: the body's own fields, or each
    /// instance's.
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

/// Apply advisor fields in order. Strings keep the last value, failed fixed5
/// scans keep the earlier cost, and repeated trait, research-bonus, and modifier
/// blocks accumulate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AdvisorFields {
    pub slot: Option<String>,
    pub idea_token: Option<String>,
    pub name: Option<String>,
    pub traits: Vec<String>,
    pub cost: Option<i64>,
    /// Defaults to zero, then follows `crate::ideas::ledger`.
    pub ledger: u32,
    pub modifiers: Modifiers,
    pub research_bonuses: Vec<Modifier>,
    pub unheld: Unheld,
}

impl AdvisorFields {
    pub(crate) fn apply(&mut self, fields: &[Item]) {
        for field in fields {
            match field.keyword().map(|keyword| keyword.folded()).as_deref() {
                // Current-token text ends at its first NUL.
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
                Some(b"ledger") => self.ledger = crate::ideas::ledger(self.ledger, field),
                // Trait string-list entries append to the existing role.
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

/// Read string-list entries; a value without an opening block adds nothing.
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

/// One character template as its fields leave it, each applied in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Template {
    /// Empty startup allowed conjunction.
    unconditional_startup: bool,
    /// `name` through string read.
    name: Option<String>,
    /// Portraits by type and size. Repeated blocks share this object; later
    /// assignments replace earlier values.
    portraits: BTreeMap<(String, String), String>,
    advisors: Vec<AdvisorFields>,
    /// Country leader roles: ideology and traits.
    leaders: Vec<(Option<String>, Vec<String>)>,
    /// A stored text the port cannot hold, its own or a role's.
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
                // Append each opening block's entries.
                Some(b"traits") => traits.extend(strings(&field.value, &mut self.unheld)),
                _ => {}
            }
        }
        self.leaders.push((ideology, traits));
    }

    /// One field of a `portraits` block: a type's `{` block
    /// sets each size it names; any other value sets the type's large
    /// portrait (size 1).
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

/// Every alternative's value, agreed.
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

/// A file's characters in order, each with its token and alternatives,
/// and the file's stop: the reading's, or the first character whose stored
/// text (its token included) the port cannot hold, from which on the file
/// is unknown.
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

/// Every character's portrait of one category and size in one file, and
/// the place its reading stops short, if it does: a character read from
/// there on is [`Authored::Unknown`] with that cause, and a character the
/// file does not list may be defined past it.
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

/// An advisor role: the idea the game appoints and the character behind it.
#[derive(Clone, Debug, Eq)]
pub struct AdvisorRole {
    pub character: String,
    pub slot: String,
    pub idea_token: String,
    pub traits: Vec<String>,
    /// Advisor cost in fixed5.
    pub cost: Option<i64>,
    /// The intel ledgers it shows on (`crate::ideas::ledger`).
    pub ledger: u32,
    /// The role modifier; repeated blocks read into the same object.
    pub modifiers: Modifiers,
    /// Its `research_bonus` entries.
    pub research_bonuses: Vec<Modifier>,
}

impl PartialEq for AdvisorRole {
    fn eq(&self, other: &Self) -> bool {
        // Role names compare with folded string identity, while character
        // and trait references use lexer tokens. Keep authored spellings for
        // names and artwork.
        self.character.eq_ignore_ascii_case(&other.character)
            && self.slot.eq_ignore_ascii_case(&other.slot)
            && self.idea_token.eq_ignore_ascii_case(&other.idea_token)
            && self.traits.len() == other.traits.len()
            && self.traits.iter().zip(&other.traits).all(|(a, b)| {
                crate::tokens::Token::c_str(a.as_bytes())
                    == crate::tokens::Token::c_str(b.as_bytes())
            })
            && self.cost == other.cost
            && self.ledger == other.ledger
            && self.modifiers == other.modifiers
            && self.research_bonuses == other.research_bonuses
    }
}

/// A country leader role for one ideology.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderRole {
    pub character: String,
    pub ideology: String,
    pub traits: Vec<String>,
}

/// What the characters files say, as far as every alternative agrees.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Characters {
    /// Advisor roles by the idea token the game appoints.
    advisors: BTreeMap<String, AdvisorRole>,
    /// Folded identity to first authored spelling (keeps the
    /// existing string on a case alias; uses the same lookup).
    advisor_spellings: BTreeMap<String, String>,
    /// Idea tokens whose alternatives or definitions disagree; left out of `advisors`.
    ambiguous_advisors: BTreeSet<String>,
    /// Country leader roles by (character, ideology).
    leaders: BTreeMap<(String, String), LeaderRole>,
    ambiguous_leaders: BTreeSet<(String, String)>,
    /// Complete country-leader role sets, needed by the native singleton
    /// selection. Differing alternatives use the held template.
    leader_sets: BTreeMap<String, Vec<LeaderRole>>,
    ambiguous_leader_sets: BTreeSet<String>,
    /// The name localisation key of each character, when every alternative agrees.
    names: BTreeMap<String, Authored>,
    /// At least one authored template always registers after the startup
    /// tag-map clear; conditional agreement alone is insufficient.
    unconditional_startup: BTreeSet<String>,
    /// Portraits by character, then (category, size): `civilian`/`army` and `large`/`small`.
    portraits: BTreeMap<String, BTreeMap<(String, String), Authored>>,
    /// Each read template's roles by the file and line the game
    /// stores on it (see [`LeaderTemplate`]), with its tag and byte-exact file path.
    built: BTreeMap<(Vec<u8>, u32), Vec<TaggedRoles>>,
    /// Files whose templates cannot all be placed by line, and why, by their paths' bytes.
    unplaced: BTreeMap<Vec<u8>, String>,
    /// Which file decides each advisor's idea token, and the files that
    /// stop short.
    pub(crate) advisor_coverage: Coverage,
    /// The same for character tokens (names, portraits, leader roles).
    pub(crate) character_coverage: Coverage,
    /// The places the game's reading leaves the text's braces, each read as
    /// the game reads it, and where a file's reading cannot be followed.
    pub notes: Vec<crate::Diagnostic>,
}

/// The template a recorded character holds, identified by its Characters-row
/// template columns. Startup builds characters from nongenerated templates
/// whose `allowed` holds, and builds their advisors from those same templates.
/// When alternatives disagree, a held role therefore belongs to the recorded
/// character's template.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[derive(replay_spool::Spooled)]
pub struct BuiltTemplate {
    /// The template tag as lexer-token text.
    pub tag: String,
    /// The starting file and line of the template read.
    pub file: Vec<u8>,
    pub line: i32,
    /// Whether `generate_character` created this template.
    pub generated: bool,
}

/// A lookup's answer: found, proven absent (`Ok(None)`), or unknown because
/// a file that could define it stops short.
pub type Lookup<'a, T> = Result<Option<&'a T>, &'a Gap>;

impl Characters {
    /// Retained character tokens in lexical order. Reader notes name refused tails.
    pub fn declared_tokens(&self) -> impl Iterator<Item = &str> {
        self.names.keys().map(String::as_str)
    }

    /// Every advisor idea token, including ambiguous roles, and any gap that
    /// may hide further declarations.
    pub fn advisor_tokens(&self) -> (impl Iterator<Item = &str>, Option<&Gap>) {
        (
            self.advisor_spellings.values().map(String::as_str),
            self.advisor_coverage.any_gap(),
        )
    }
    /// The advisor role appointed by an idea token. Alternatives that
    /// disagree are `Ok(None)` here; [`Self::held_advisor`] takes the role
    /// of the template a holder was built from.
    pub fn advisor(&self, idea_token: &str) -> Lookup<'_, AdvisorRole> {
        let identity = idea_token.to_ascii_lowercase();
        // Every declaration participates in role agreement: an earlier
        // unread suffix can disagree too, or supply the first spelling
        // retained by the native hashed-string set.
        match self
            .advisor_coverage
            .unknown(&identity)
            .or_else(|| self.advisor_coverage.before_first(&identity))
        {
            Some(gap) => Err(gap),
            None => Ok(self.advisors.get(&identity)),
        }
    }

    /// Whether an idea token's alternatives or definitions disagree.
    pub fn ambiguous_advisor(&self, idea_token: &str) -> bool {
        self.ambiguous_advisors
            .contains(&idea_token.to_ascii_lowercase())
    }

    /// An appointed advisor role: use agreement across alternatives, otherwise
    /// the recorded holder's [`BuiltTemplate`], matched by file, line, and folded
    /// tag. `built` supplies that identity or its unavailable-input reason;
    /// `Err` names what cannot be decided.
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

    /// A character's country leader role for one ideology.
    pub fn leader(&self, character: &str, ideology: &str) -> Lookup<'_, LeaderRole> {
        match self.character_coverage.unknown(character) {
            Some(gap) => Err(gap),
            None => Ok(self
                .leaders
                .get(&(character.to_owned(), ideology.to_owned()))),
        }
    }

    /// The complete set of country-leader roles, only when alternatives
    /// agree. The order is a projection; native selection is by ideology
    /// token or exactly one role.
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

    /// Proves tag presence without reevaluating startup conditions. Plain
    /// templates and empty instance conjunctions always register; other
    /// conditions require the stored map.
    pub fn template_has_unconditional_startup(&self, character: &str) -> bool {
        self.unconditional_startup.contains(character)
    }

    /// A character's name key, as far as every alternative agrees.
    pub fn name(&self, character: &str) -> Lookup<'_, Authored> {
        match self.character_coverage.unknown(character) {
            Some(gap) => Err(gap),
            None => Ok(self.names.get(character)),
        }
    }

    /// A character's portraits by (category, size).
    pub fn portraits(&self, character: &str) -> Lookup<'_, BTreeMap<(String, String), Authored>> {
        match self.character_coverage.unknown(character) {
            Some(gap) => Err(gap),
            None => Ok(self.portraits.get(character)),
        }
    }
}

/// Parses one characters file, its triggers against `names`.
pub fn parse(input: &[u8], names: &KeyNames) -> io::Result<Characters> {
    parse_file(input, names).map(|(characters, _, _)| characters)
}

/// A template's tag and its complete native role projections.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TaggedRoles {
    tag: String,
    advisors: Vec<AdvisorRole>,
}

/// Each character the file reads before any stop, with each alternative's
/// roles, in file order.
type TemplateRoles = Vec<(String, Vec<TaggedRoles>)>;

/// Project country-leader fields from an already-read effect body.
/// Trait blocks append.
pub fn country_leader_role(character: &str, fields: &[Item]) -> Result<LeaderRole, String> {
    let mut template = Template::default();
    template.leader(fields);
    if let Some(why) = template.unheld.cause() {
        return Err(format!("country-leader role: {why}"));
    }
    Ok(leader_roles(character, &template).remove(0))
}

/// Project advisor fields from an already-read effect body, sharing the
/// character catalogue's field application.
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

/// An ideology-keyed map: repeated ideologies replace earlier roles.
/// Text keys project registered ideology identity.
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

/// One alternative's advisor templates, inserted by slot identity. Missing
/// tokens default to invalid identities, projected here as empty strings.
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

/// [`parse`], the place the file's reading stops short, if it does, and
/// each character's alternatives' roles.
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
            crate::DiagnosticKind::Unsettled,
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
    // A character from the stop on is known by name only: the file's gap
    // covers it.
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
                // An absent identity remains on the native character
                // template, but no named idea can appoint that role.
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
                    // An invalid ideology reports an error but still inserts the role
                    // under the null ideology identity.
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
            crate::DiagnosticKind::Warning,
            format!(
                "{} character roles kept with native role defaults: {}",
                defaulted.len(),
                defaulted.into_iter().collect::<Vec<_>>().join("; ")
            ),
        ));
    }
    if let Some(warning) = scans.warning() {
        result.notes.push((crate::DiagnosticKind::Misread, warning));
    }
    Ok((result, stop.map(|(_, cause)| cause), roles))
}

/// One alternative's role: a key alternatives define differently is
/// ambiguous.
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

/// Folded character tokens registered before resources. Bodies are read
/// after ideas, so trigger readers see all registered character names.
/// The mode-1 file loop interns each first-NUL key under `characters`
/// and skips its template bodies, including instance blocks. Other roots
/// report an error and skip their blocks.
pub fn tokens(layers: &ResolvedLayers) -> io::Result<BTreeSet<Vec<u8>>> {
    let mut tokens = BTreeSet::new();
    for path in paths(layers) {
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

/// Direct `.txt` files under `common/characters` in load order;
/// see [`crate::script_files::paths`].
pub fn paths(layers: &ResolvedLayers) -> Vec<crate::script_files::ScriptPath> {
    crate::script_files::paths(layers, "common/characters/", Some(".txt"))
}

impl Characters {
    /// The current file defines what `other` holds: its character tokens
    /// (names, portraits, leader roles) and its advisors' idea tokens.
    pub(crate) fn register(&mut self, other: &Characters) {
        for token in other.names.keys() {
            self.character_coverage.add(token);
        }
        for idea in other.advisors.keys().chain(&other.ambiguous_advisors) {
            self.advisor_coverage.add(idea);
        }
    }

    /// The next file in load order, for both coverages.
    pub(crate) fn next_file(&mut self) {
        self.advisor_coverage.next_file();
        self.character_coverage.next_file();
    }

    /// The current file stops short, for both coverages.
    pub(crate) fn gap(&mut self, path: &str, cause: &str) {
        self.advisor_coverage.gap(path, cause);
        self.character_coverage.gap(path, cause);
    }

    /// Adds another source's characters; a token the two define differently
    /// is ambiguous. Notes stay with their source; coverage is the caller's.
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

    /// Places each template of a file's characters (`roles`, in file order)
    /// at the file and line the game stores on it: the templates the
    /// database makes of the same file, in the same order, with their lines
    /// ([`parse_leader_templates`]). A template whose tag differs from its
    /// character's leaves that one and the rest unplaced, named.
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

/// A character template projected from an idea entry in a group named by
/// NDefines.NCharacter.OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU. Its token and name
/// use the entry key; it has one advisor whose fields share
/// [`AdvisorFields::apply`].
pub(crate) fn officer_template(
    token: &str,
    group: &str,
    fields: &[Item],
) -> Result<Characters, String> {
    // The slot starts as the group name; an authored `slot` replaces it.
    let mut advisor = AdvisorFields {
        slot: Some(group.to_owned()),
        ..AdvisorFields::default()
    };
    advisor.apply(fields);
    if let Some(cause) = advisor.unheld.cause() {
        return Err(format!("advisor {token}: {cause}"));
    }
    // An empty name defaults to the entry key; any other name names the template.
    let name = advisor
        .name
        .take()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| token.to_owned());
    // The small army portrait uses `GFX_idea_` followed by the name.
    let portrait = format!("GFX_idea_{name}");
    let mut template = Characters::default();
    // After reading, the idea token is reset to the key regardless of `idea_token`.
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

/// Reads every characters file, with the templates other content declares
/// (officer-corps idea groups, [`crate::ideas::read`], whose coverage comes
/// with them). A file that cannot be read is left out with a warning; a file
/// whose reading stops short leaves what it could define unknown;
/// definitions of one token in several files must agree.
pub fn read(
    layers: &ResolvedLayers,
    templates: Characters,
) -> io::Result<(Characters, Vec<crate::Diagnostic>)> {
    let mut result = templates;
    let mut warnings = Vec::new();
    let names = layers.key_names()?;
    for listed in paths(layers) {
        let path = listed.to_string();
        result.next_file();
        let input = listed.read(layers)?;
        match parse_file(&input, &names) {
            Ok((mut file, stop, roles)) => {
                warnings.extend(
                    std::mem::take(&mut file.notes)
                        .into_iter()
                        .map(|(kind, note)| (kind, format!("Characters in {path}: {note}."))),
                );
                result.register(&file);
                if let Some(cause) = stop {
                    result.gap(&path, &cause);
                }
                file.place(&listed, &input, roles);
                result.merge(file);
            }
            Err(error) => {
                warnings.push((
                    crate::DiagnosticKind::Refused,
                    format!("Characters unavailable in {path}: {error}"),
                ));
                result.gap(&path, &error.to_string());
            }
        }
    }
    Ok((result, warnings))
}

/// Unit-leader role keys: `corps_commander`, `field_marshal`, and `navy_leader`.
const UNIT_LEADER_ROLES: [&str; 3] = ["corps_commander", "field_marshal", "navy_leader"];

/// A script key as the lexer finds its token ([`Item::keyword_is`]): a bare
/// key looked up folded; a quoted key is never a
/// keyword.
fn is_key(item: &Item, key: &str) -> bool {
    item.keyword_is(key)
}

/// One character template with the unit-leader visibility role. The first
/// file pass registers templates under `characters`; the post-load pass
/// reads each body. A body creates one template or one per leading `instance`.
/// Each instance must begin with `allowed`; otherwise it reports an error
/// and skips unread. Template reads store the starting file and line.
#[derive(Clone, Debug, PartialEq)]
pub struct LeaderTemplate {
    /// Byte-exact file path, matching what the game stores on the template.
    pub path: ScriptPath,
    /// The authored character token used as its template tag.
    pub tag: String,
    /// The game's reading began at the first token after the character block's `{`,
    /// or an instance's `}` closing `allowed`. `None` means the template was never read.
    pub line: Option<u32>,
    /// `None` without a role, otherwise its `visible` conjunction;
    /// `Err` for a form refused by name.
    pub role: Result<Option<Triggers>, String>,
}

/// The templates one character block makes, in database order.
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
        // An `instance` after another key goes to the template's
        // unexpected-token report
        // and is not read.
        let body: Vec<Item> = items
            .iter()
            .filter(|item| !is_key(item, "instance"))
            .cloned()
            .collect();
        let line = lines
            .get(&(value as *const Value))
            .map(|lines| lines.first_token);
        return vec![template(line, template_role_for(&body, roles))];
    }
    let trailing = leading < items.len();
    items[..leading]
        .iter()
        .map(|item| {
            if trailing {
                // The loader reports the keys after the instances and the
                // second pass then reads them as characters of their own.
                return template(None, Err("keys follow its instance blocks".into()));
            }
            let Some(body) = item.value.items() else {
                return template(None, Err("an instance is not a block".into()));
            };
            match body.split_first() {
                Some((allowed, rest)) if is_key(allowed, "allowed") => {
                    let line = lines
                        .get(&(&allowed.value as *const Value))
                        .map(|lines| lines.close);
                    match line {
                        Some(line) => template(Some(line), template_role_for(rest, roles)),
                        None => template(None, Err("its allowed is not a block".into())),
                    }
                }
                // "Expected allowed": the instance is skipped unread.
                _ => template(None, Ok(None)),
            }
        })
        .collect()
}

/// One template's unit-leader role: `None` when absent, otherwise its
/// `visible` conjunction. Each role field replaces the preceding role;
/// repeated `visible` fields append to one AND trigger in order. An empty
/// conjunction holds.
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
        // The tree reader cannot follow a trigger-body value that
        // opens no block, so the template is refused by name.
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

/// Every value's block lines, by address, in the parser's pre-order.
fn index_lines(
    items: &[Item],
    spans: &[parser::BlockLines],
    next: &mut usize,
    out: &mut BTreeMap<*const Value, parser::BlockLines>,
) {
    for item in items {
        if let Some(children) = item.value.items() {
            if let Some(&span) = spans.get(*next) {
                out.insert(&item.value as *const Value, span);
            }
            *next += 1;
            index_lines(children, spans, next, out);
        }
    }
}

/// The templates of one characters file, in database order. Only `characters`
/// roots are read; others are reported.
pub fn parse_leader_templates(
    path: &ScriptPath,
    input: &[u8],
) -> io::Result<Vec<LeaderTemplate>> {
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

/// A scientist role's retained visible conjunction, identified by the last
/// visible block the native reader loaded. Role effects have their own origin.
#[derive(Clone, Debug, PartialEq)]
pub struct ScientistVisibilityTemplate {
    pub line: u32,
    pub triggers: Result<Triggers, String>,
}

pub fn parse_scientist_visibility_templates(
    input: &[u8],
) -> io::Result<Vec<ScientistVisibilityTemplate>> {
    let (document, spans, _) = parser::parse_unclosed_lines(input)?;
    let mut lines = BTreeMap::new();
    let mut next = 0;
    index_lines(&document.items, &spans, &mut next, &mut lines);
    if next != spans.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "scientist visibility block lines differ",
        ));
    }
    fn visit(
        items: &[Item],
        lines: &BTreeMap<*const Value, parser::BlockLines>,
        out: &mut Vec<ScientistVisibilityTemplate>,
    ) {
        for item in items {
            let Some(body) = item.value.items() else {
                continue;
            };
            if is_key(item, "scientist")
                && let Some(last) = body.iter().rfind(|item| is_key(item, "visible"))
                && let Some(line) = lines.get(&(&last.value as *const Value))
            {
                out.push(ScientistVisibilityTemplate {
                    line: line.open,
                    triggers: template_role_for(std::slice::from_ref(item), &["scientist"])
                        .and_then(|role| role.ok_or_else(|| "scientist role missing".into())),
                });
            }
            visit(body, lines, out);
        }
    }
    let mut out = Vec::new();
    visit(&document.items, &lines, &mut out);
    Ok(out)
}

/// Every character template of the recorded content, in database order. A
/// file that cannot be read or parsed refuses the whole reader, since the
/// templates it holds could be any character's.
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        // ASCII aliases do not fold non-ASCII bytes.
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    /// Repeated ideologies replace earlier roles; singleton selection requires
    /// one remaining role. Alternatives need a recorded template identity even
    /// when their ideology names agree.
    #[test]
    fn country_leader_sets_keep_overrides_and_alternative_identity() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        assert_eq!(roles.0, crate::DiagnosticKind::Warning);
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    /// Disputed advisor roles use the recorded holder's template, identified
    /// by file, starting line, and folded tag. An instance starts at the `}`
    /// closing `allowed`; a plain body starts at its `{`.
    #[test]
    fn a_disagreeing_advisor_role_is_its_built_templates() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root =
            std::env::temp_dir().join(format!("installed-content-built-advisor-{}", std::process::id()));
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
        // Definitions in two files disagree too; each is its own template.
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
        // A role every alternative agrees on needs no template.
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (army, stop) = portraits(FILE, &KeyNames::default(), "army", "large").unwrap();
        assert_eq!(stop, None);
        assert_eq!(army["GER_same"], Authored::Asset("GFX_same".into()));
        assert_eq!(army["GER_schacht"], Authored::Missing);
    }

    /// Portrait assignments apply in order: later sizes replace earlier ones,
    /// scalar roles set their large portrait, and repeated blocks share one
    /// object. Only conditional instances are compared.
    #[test]
    fn only_agreeing_army_large_alternatives_are_usable() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    /// Leading `instance` bodies read only instances, each beginning with
    /// `allowed`. Later `instance` fields are reported. Missing `allowed` or
    /// fields after instances leave recovery unsettled, refusing the current
    /// and later characters by name; [`portraits`] names each cause.
    #[test]
    fn instances_require_leading_allowed_and_refuse_unsettled_suffixes() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    /// Advisor fields apply in order: traits accumulate, failed costs retain
    /// the earlier value, repeated modifiers share one object, and research
    /// bonuses retain entries. `slot`, `idea_token`, and `ideology` use current
    /// lexer text, so `@name` keeps that text. Country-leader traits accumulate too.
    #[test]
    fn role_fields_apply_in_order() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

    /// Unit-leader and scientist roles read object fields even from a scalar
    /// value. In that case following fields belong to the role rather than
    /// the character.
    #[test]
    fn military_roles_read_as_their_templates_read() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        // The role's loop took a's `}` too, so `d` was read as a field of
        // `a` (reported and skipped): no character.
        assert_eq!(name(&characters, "d"), None);
        assert_eq!(name(&characters, "b"), Some(Authored::Asset("B2".into())));
        assert_eq!(name(&characters, "c"), Some(Authored::Asset("C".into())));
    }

    /// Quoted keys are not keywords and bare keys fold; variables remain
    /// in file scope. A block on a string field becomes template fields,
    /// and end-of-text closes every loop.
    #[test]
    fn the_replay_classes_hold_for_characters() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        // `name` read `{`; the block's portraits are the template's and its
        // `}` ended the character, so `nationality` is a character too.
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
        // A last bare token at the end still reads as the value.
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
                // The game's reader has lexed the first body token when reading begins.
                ("joined", Some(4)),
                ("replaced", Some(6)),
                ("advisor_only", Some(7)),
                // Each instance's line is its allowed's closing brace; the
                // one without `allowed` first is never read.
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
        // Every visible of the role joins the one conjunction, keys folded.
        assert_eq!(role(1), Ok(Some(conjunction("has_dlc = A tag = GER"))));
        // A later role replaces the earlier one.
        assert_eq!(role(2), Ok(Some(Triggers::default())));
        assert_eq!(role(3), Ok(None));
        assert_eq!(role(4), Ok(Some(conjunction("tag = GER"))));
        assert_eq!(role(5), Ok(None));
        assert_eq!(role(6), Ok(None));
        // An instance after other keys is not read; the block's own role is.
        assert_eq!(role(7), Ok(Some(conjunction("tag = ITA"))));
        assert!(role(8).is_err());
        assert!(role(9).is_err());
        // A scalar visible is refused by name, never read as true.
        assert_eq!(
            role(10),
            Err("its unit leader role's visible is not a block".into())
        );
    }
}
