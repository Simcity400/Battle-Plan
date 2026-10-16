//! Idea categories and ideas (`common/idea_tags`, `common/ideas`): which slot
//! or category each idea belongs to, and what it shows (name, picture, effects).
//!
//! The recording names active ideas by token only; this is what joins a token to
//! its category, slot and effects. Nothing here assumes vanilla categories:
//! a mod's categories and slots come through the same reader.
//!
//! One idea database (`Database`) takes both directories in load order:
//! `common/idea_tags` files first, then `common/ideas`. Both read
//! `idea_categories` and `ideas`; only the first reads `slot_ledgers`,
//! and only the second rereads a group it already holds. Each file has
//! its own reader replay ([`native`]) with the readers' own dispatch.
//! Conditions (`allowed`, `visible`, `available`) and effects (`on_add`, …)
//! need the game to evaluate and are only read by key. Modifier values
//! are stored as the game stores them ([`crate::modifiers`]).
//!
//! A file whose reading the port cannot follow to its end, or whose stored
//! text the port cannot hold, leaves what the rest of it defines unknown
//! ([`crate::coverage`]): a lookup that part could change is refused, with
//! the file and the cause.

use crate::{
    ResolvedLayers,
    characters::{self, Characters},
    coverage::{Coverage, Gap},
    crt::{Scanned, scan_i32},
    key_names::{KeyNames, Point},
    modifiers::{self, EquipmentBonus, Modifier, Modifiers, TargetedModifier},
    parser::{
        Item, Kind, Unheld,
        native::{self, Field, Shape},
    },
    scope::ScopeKind,
    script_files,
    tokens::Token,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

/// One idea category from `idea_categories`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IdeaCategory {
    pub name: String,
    /// Idea slots, in source order (laws, designers).
    pub slots: Vec<String>,
    /// Character seats, one entry per seat (three political advisors are three entries).
    pub character_slots: Vec<String>,
    /// `type` dispatches by the value keyword: `national_spirit`,
    /// `army_spirit`, `navy_spirit` or `air_spirit`, whatever its case.
    /// None for any other value, a quoted one included (type 0).
    pub kind: Option<String>,
    pub hidden: bool,
    /// False when the category sets `politics_tab = no`.
    pub politics_tab: bool,
    /// The intel ledgers it shows on (`ledger`).
    pub ledger: u32,
    pub cost: Option<f64>,
    pub removal_cost: Option<f64>,
}

/// The categories in the database's array, in load order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IdeaTags {
    /// Every category object read; a repeated name appends beside
    /// the earlier one.
    categories: Vec<IdeaCategory>,
    /// The first file that stops short, and how many categories were read
    /// before it: every category after those is unknown.
    cut: Option<(usize, Gap)>,
}

impl IdeaTags {
    /// Resolve a group or slot name to its category: each category in array
    /// order, its name and then its slots and seats (both kinds), compared
    /// exactly; the first match wins. A gap when a category that could match
    /// comes after a file that stops short.
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

    /// Every category read, in load order, and the gap that may hide more.
    pub fn categories(&self) -> (&[IdeaCategory], Option<&Gap>) {
        (&self.categories, self.cut.as_ref().map(|(_, gap)| gap))
    }
}

/// One idea from `ideas = { <group> = { <token> = { … } } }`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Idea {
    pub token: String,
    /// The name of the group it belongs to: an idea slot
    /// (`economy`, `tank_manufacturer`) or a category (`country`,
    /// `hidden_ideas`).
    pub group: String,
    /// The group's `law` and `designer` flags as the database holds them
    /// once every file is read: the game reads the current group flags
    /// when it uses the idea.
    pub law: bool,
    pub designer: bool,
    /// The slot's default law.
    pub default: bool,
    /// `name`: the override shown when it is not empty.
    pub name: Option<String>,
    /// The picture's name, from `picture`; the sprite is `GFX_idea_<picture>`.
    pub picture: Option<String>,
    pub modifiers: Modifiers,
    pub targeted: Vec<TargetedModifier>,
    pub equipment_bonuses: Vec<EquipmentBonus>,
    pub research_bonuses: Vec<Modifier>,
    pub traits: Vec<String>,
    /// The idea's rule fields.
    pub rules: IdeaRules,
    pub cost: Option<f64>,
    pub removal_cost: Option<f64>,
    /// The intel ledgers it shows on (`ledger`).
    pub ledger: u32,
    /// `level`: initially -1, then read as a signed integer;
    /// a failed scan keeps it.
    pub level: i32,
    /// Its index in the database's ideas (the null idea at 0, so the first
    /// is 1), or, once a `common/ideas` file rereads its group, its place in
    /// the group from 0. Every idea of that group receives its updated place.
    /// A term of the generated name's seed.
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
            // Construction copies each rule default.
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
                    // Rule override indices use decimal integer conversion.
                    let raw = crate::fixed5::decimal_i32(entry.key.as_deref().unwrap_or_default());
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
                // The temporary boolean starts false before conversion.
                self.values[index] = field
                    .value
                    .scalar()
                    .and_then(crate::parser::read_bool)
                    .unwrap_or(false);
            }
        }
        Ok(())
    }
}

impl Idea {
    /// The picture sprite, prefixed with `GFX_idea_`.
    pub fn sprite(&self) -> String {
        format!(
            "GFX_idea_{}",
            self.picture.as_deref().unwrap_or(&self.token)
        )
    }
    /// The sprites the idea icon resolver tries, in order:
    /// `GFX_idea_<token>` first, then [`Self::sprite`], then that sprite
    /// followed by `_<holder's tag>`; when none exists it draws `GFX_idea_unknown`.
    /// Without a recorded tag, returns only the static candidates; the caller
    /// cannot decide the tag candidate or the final fallback offline.
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

/// Exactly `yes` or `no`, whatever the quoting;
/// anything else fails and leaves the field as it was.
fn bool_token(item: &Item) -> Option<bool> {
    item.value.scalar().and_then(crate::parser::read_bool)
}

/// Fixed5 as a number; a failed conversion leaves the field as it was.
fn fixed5_number(item: &Item) -> Option<f64> {
    modifiers::read_fixed5(item).map(|raw| raw as f64 / 100_000.0)
}

/// Signed integer scanning: a scan that stores nothing (EOF) or fails
/// leaves the field as it was.
fn int32(item: &Item) -> Option<f64> {
    match scan_i32(item.value.scalar()?) {
        Scanned::Stored(value) => Some(f64::from(value)),
        Scanned::End | Scanned::NoMatch => None,
    }
}

/// The keyword a field dispatches on.
fn keyword(item: &Item) -> Option<Vec<u8>> {
    item.keyword().map(|keyword| keyword.folded())
}

/// The intel ledger reader shared by categories, ideas, `slot_ledgers`
/// and advisors: the value keyword sets or ORs the bits; any other value
/// (quoted, a number, the end of text included) is reported and leaves
/// them unchanged.
pub(crate) fn ledger(bits: u32, item: &Item) -> u32 {
    match item.value_keyword().as_deref() {
        Some("hidden") => 1,
        Some("army") => bits | 0x4,
        Some("navy") => bits | 0x8,
        Some("military") => bits | 0x1C,
        Some("all") => bits | 0x1E,
        Some("air") => bits | 0x10,
        Some("civilian") => bits | 0x2,
        Some("invalid") => 0xFFFF,
        _ => bits,
    }
}

/// A recognised `ledger` value consumes one token; any other is reported
/// and skips a block the current token opens.
pub(crate) fn ledger_shape<S>(field: &Field<'_>) -> Shape<S> {
    match field.value_keyword().as_deref() {
        Some(
            b"hidden" | b"army" | b"navy" | b"military" | b"all" | b"air" | b"civilian"
            | b"invalid",
        ) => Shape::One,
        _ => Shape::Skip,
    }
}

/// The two file loops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Loader {
    /// `common/idea_tags`: reads `slot_ledgers`; every group is read
    /// as a new one, then registered.
    Tags,
    /// `common/ideas`: a held group named by a bare key is read again.
    Ideas,
}

/// A group's name as the token registry keys it: A-Z folded, compared
/// without case. `None` for a name starting with a digit or `-`,
/// which is refused (id 0), so the group never merges.
fn identity(name: &[u8]) -> Option<Vec<u8>> {
    (!matches!(name.first(), Some(b'0'..=b'9' | b'-'))).then(|| name.to_ascii_lowercase())
}

/// How a group occurrence is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Path {
    /// A new group; `officers` when its name is in
    /// NDefines.NCharacter.OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU.
    New { officers: bool },
    /// Read again into a group the database holds by the key's token id.
    Again,
}

/// What decides a group occurrence's path: the database's group names so
/// far, and the cause when an earlier file stops short, whose unknown rest
/// may have registered any name.
#[derive(Clone, Debug)]
struct Groups<'a> {
    loader: Loader,
    officers: &'a [String],
    known: BTreeSet<Vec<u8>>,
    unknown: Option<String>,
}

impl Groups<'_> {
    /// The path of the group `key` names, registering its name; an error
    /// when an earlier gap leaves it unknown whether the database holds it.
    fn path(&mut self, key: &[u8], quoted: bool) -> Result<Path, String> {
        let identity = identity(key);
        let held = identity.as_ref().is_some_and(|id| self.known.contains(id));
        if !held && let Some(unknown) = &self.unknown {
            return Err(format!(
                "group {} may be one the unknown rest of an earlier file registers ({unknown}), which changes how it is read",
                key.escape_ascii()
            ));
        }
        let path = if self.loader == Loader::Ideas && held && crate::parser::looked_up(key, quoted)
        {
            Path::Again
        } else {
            // Compare the officer-corps group name exactly.
            Path::New {
                officers: self.officers.iter().any(|name| name.as_bytes() == key),
            }
        };
        self.known.extend(identity);
        Ok(path)
    }
}

/// Which reader a field goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Top,
    /// The `idea_categories` loop: each key reads a category object.
    Categories,
    /// One category and its fields.
    Category,
    /// The `slot_ledgers` loop: each entry reads a ledger value.
    Ledgers,
    /// The `ideas` loop: each key a group.
    Groups,
    Group(Path),
    /// One idea and its fields.
    Idea,
    /// An advisor template and its fields.
    Advisor,
    /// Rule fields ([`rule_field`]).
    Rule(RuleNode),
    /// Mean-time-to-happen fields.
    Mtth,
    Modifier(modifiers::Node),
}

/// Rule readers (an idea's `rule`, an ideology group's `rules`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuleNode {
    /// One rule field.
    Rule,
    /// Its `override` loop.
    Override,
}

/// `icon` skips a block when the current token is `{`; `desc` reads
/// a string; `override` opens its own loop, converting each key and
/// value text as decimal integers. Any other key reads a rule boolean
/// or is only logged.
pub(crate) fn rule_field(node: RuleNode, field: &Field<'_>) -> Shape<RuleNode> {
    match node {
        RuleNode::Rule => match field.keyword().as_deref() {
            Some(b"icon") => Shape::SkipIfOpen,
            Some(b"override") => Shape::Object(RuleNode::Override),
            _ => Shape::One,
        },
        RuleNode::Override => Shape::One,
    }
}

/// Where a file's reading, or what it stores, cannot be followed: the
/// count of ideas and categories before it, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stop {
    ideas: usize,
    categories: usize,
    cause: String,
}

impl Stop {
    /// The earlier of two stops.
    fn first(stop: Option<Self>, other: Self) -> Self {
        match stop {
            Some(stop) if (stop.ideas, stop.categories) <= (other.ideas, other.categories) => stop,
            _ => other,
        }
    }
}

/// What the dispatch learns while the reader reads.
struct Reading<'a> {
    groups: Groups<'a>,
    /// Ideas and advisors, and categories, dispatched so far, and the idea
    /// count when the group being read began.
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
            // `idea_categories` and `ideas` open their own loops; `slot_ledgers`
            // opens its own loop only for idea tags. Any other key, a quoted one
            // included, is reported and skipped.
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
            // `slot` and `character_slot` read text; `politics_tab` and `hidden`
            // read booleans; `cost` and `removal_cost` read signed integers;
            // `type` uses the value keyword, and `ledger` the shared ledger reader.
            // Any other key is reported and skipped.
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
            // `law`, `designer` and `use_list_view` read booleans. Any other key
            // with a `{` value reads an idea, or for an officer-corps group an
            // advisor template; any other value is reported and skipped.
            Node::Group(Path::New { officers }) => match keyword {
                Some(b"law" | b"designer" | b"use_list_view") => Shape::One,
                _ if field.opens() => {
                    self.ideas += 1;
                    Shape::Object(if officers { Node::Advisor } else { Node::Idea })
                }
                _ => Shape::Skip,
            },
            // `law` and `designer` read booleans; any other key, `use_list_view`
            // included, reads an idea whatever its value, without checking for
            // `{` or testing whether the group belongs to the officer corps.
            Node::Group(Path::Again) => match keyword {
                Some(b"law" | b"designer") => Shape::One,
                _ => {
                    self.ideas += 1;
                    Shape::Object(Node::Idea)
                }
            },
            Node::Idea => match keyword {
                // Conjunction triggers: `available`, `cancel`, `visible`, `allowed`,
                // `allowed_civil_war`, `allowed_to_remove`, `do_effect`.
                Some(
                    b"available" | b"cancel" | b"visible" | b"allowed" | b"allowed_civil_war"
                    | b"allowed_to_remove" | b"do_effect",
                ) => Shape::Trigger,
                // `on_add` and `on_remove` read effects by key in country scope.
                Some(b"on_add" | b"on_remove") => Shape::EffectIn(ScopeKind::Country.bit()),
                // `cost` and `removal_cost` read fixed5; `name` and `picture` read
                // strings; `level` reads a signed integer; `default`,
                // `cancel_if_invalid` and `is_good` read booleans; `category`
                // is only logged.
                Some(
                    b"cost" | b"removal_cost" | b"name" | b"picture" | b"level" | b"default"
                    | b"cancel_if_invalid" | b"is_good" | b"category",
                ) => Shape::One,
                // `ledger` uses the shared ledger reader.
                Some(b"ledger") => ledger_shape(field),
                // Object fields: `modifier`, `ai_will_do`, `rule` and
                // `targeted_modifier`.
                Some(b"modifier") => Shape::Object(Node::Modifier(modifiers::Node::Modifier)),
                Some(b"ai_will_do") => Shape::Object(Node::Mtth),
                Some(b"rule") => Shape::Object(Node::Rule(RuleNode::Rule)),
                Some(b"targeted_modifier") => {
                    Shape::Object(Node::Modifier(modifiers::Node::Targeted))
                }
                // Own loops: `research_bonus` and `equipment_bonus`; `traits`
                // reads each token through the closing brace.
                Some(b"research_bonus") => {
                    Shape::Object(Node::Modifier(modifiers::Node::ResearchBonus))
                }
                Some(b"equipment_bonus") => {
                    Shape::Object(Node::Modifier(modifiers::Node::EquipmentBonuses))
                }
                Some(b"traits") => Shape::List,
                // Any other key, `designer` and a quoted one included,
                // is reported and skipped.
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

/// One category as read.
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
            // Hash the value's text for its name identity.
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
            // The value keyword selects `national_spirit`, `army_spirit`,
            // `navy_spirit` or `air_spirit`; any other value, including `default`,
            // a quoted value or the end of text, selects type 0. Each assignment
            // replaces the last.
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

/// One idea body as read, or what it stores that the port cannot hold.
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
            // Read the value as string text.
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
            // Every `modifier` block reads into the idea's one modifier object.
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
            // Each entry reads string text.
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

/// One group occurrence as the file read it.
struct Occurrence {
    name: String,
    identity: Option<Vec<u8>>,
    path: Path,
    law: Option<bool>,
    designer: Option<bool>,
    /// The count of ideas and advisors before it in the file.
    start: usize,
    ideas: Vec<Idea>,
    advisors: Vec<Characters>,
}

/// What one file declares, in document order, as far as it can be followed.
enum Declared {
    Category(IdeaCategory),
    Group(Occurrence),
}

/// One file as its loader reads it: its declarations up to the stop, the
/// ideas and advisors the stop leaves known by name only, the stop, and
/// the misreads.
struct FileRead {
    declared: Vec<Declared>,
    refused_tokens: Vec<String>,
    stop: Option<Stop>,
    notes: Vec<crate::Diagnostic>,
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
    // Both idea tags and ideas construct a mode-0 lexer per file.
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
    // The same walk as the dispatch, over what it read.
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
                            // Only an entry the group read as one is an idea.
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
        // Only what comes before the stop is known.
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
            .chain(scan_notes.map(|note| (crate::DiagnosticKind::Misread, note)))
            .collect(),
    }
}

/// A group the database holds: its flags and its ideas' places in
/// the database's ideas.
#[derive(Clone, Debug)]
struct Group {
    name: String,
    identity: Option<Vec<u8>>,
    law: bool,
    designer: bool,
    ideas: Vec<usize>,
}

/// The idea database both loaders build.
#[derive(Clone, Debug, Default)]
struct Database {
    tags: IdeaTags,
    groups: Vec<Group>,
    /// The idea array without its null idea: an idea's index is its
    /// place here plus one.
    ideas: Vec<Idea>,
    /// The first file that stops short, whose unknown rest may register
    /// any group name.
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

    /// The idea takes the database's count (the null idea included)
    /// as its index and joins the database and `group`.
    fn register(&mut self, mut idea: Idea, group: usize) -> usize {
        idea.position = i32::try_from(self.ideas.len() + 1).unwrap_or(i32::MAX);
        idea.group.clone_from(&self.groups[group].name);
        self.ideas.push(idea);
        let index = self.ideas.len() - 1;
        self.groups[group].ideas.push(index);
        index
    }

    /// One file's declarations, in order; the indices of the ideas it
    /// defines, and the advisors it declares.
    fn apply(&mut self, declared: Vec<Declared>) -> (Vec<usize>, Characters) {
        let mut defined = Vec::new();
        let mut advisors = Characters::default();
        for declared in declared {
            let occurrence = match declared {
                // A name declared again adds another object.
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
                // Reread the held group: its flags change for every idea it holds;
                // an idea whose name matches without case is rebuilt in place under
                // the new spelling; any other is appended. Every idea then takes
                // its place in the group as its index.
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
                // A new group whose name the database already holds gives it its
                // ideas, appended whatever their names, and is discarded with
                // its flags.
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

    /// The ideas as the game uses them once every file is read: each takes
    /// its group's flags.
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

/// Parses one `common/idea_tags` file, its triggers against `names`.
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

/// One file's ideas in read order. `refused` is set when the reading cannot
/// be followed: it names where and why, and `refused_tokens` the ideas and
/// advisors from the group holding that place on, whose contents are not
/// known. `notes` are the places the game's reading leaves the text's
/// braces, each read as the game reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IdeaFile {
    pub ideas: Vec<Idea>,
    /// The character templates the officer-corps groups declare
    /// (`characters::officer_template`): advisors, not ideas.
    pub advisors: Characters,
    pub refused: Option<String>,
    pub refused_tokens: Vec<String>,
    pub notes: Vec<String>,
}

/// Parses one `common/ideas` file on its own, its triggers against `names`.
/// `officers` is NDefines.NCharacter.OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU.
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

/// Every idea category and idea in the content stack.
#[derive(Clone, Debug, Default)]
pub struct IdeaContent {
    pub tags: IdeaTags,
    /// The database's ideas, in its order.
    ideas: Vec<Idea>,
    /// The first matching lexer token wins; ASCII case folds without
    /// changing the definition's authored spelling.
    by_token: BTreeMap<Token, usize>,
    /// Tokens in load order (a token defined again keeps its first place): a law
    /// slot lists its laws from least to most escalated this way.
    order: Vec<String>,
    /// Effective laws by slot, in database order.
    laws_by_slot: BTreeMap<String, Vec<usize>>,
    /// Definition indices: a group reread replaces an idea in place,
    /// while an appended duplicate cannot replace that match.
    coverage: Coverage,
    /// A gap before an entry was created could hold an earlier matching
    /// token, even after this entry is rebuilt in place.
    first_files: Vec<usize>,
}

impl IdeaContent {
    /// Every declared token in database order, and a gap that may hide more.
    /// Resolve each with `idea`: a later unread file may have replaced it.
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

    /// Content from parsed files alone, every file read to its end: other
    /// crates' test fixtures.
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

    /// An idea by token: None when no file defines it, the gap when a file
    /// that could define or redefine it stops short.
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

    /// The laws of a slot in load order, and the gap that may hide more.
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

/// Reads `common/idea_tags` and then `common/ideas` into one database.
/// `officers` is NDefines.NCharacter.OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU,
/// whose groups the game reads as character advisor templates. Those
/// come back apart from the ideas, for [`characters::read`].
pub fn read(
    layers: &ResolvedLayers,
    officers: &[String],
) -> io::Result<(IdeaContent, Characters, Vec<crate::Diagnostic>)> {
    let mut database = Database::default();
    let mut coverage = Coverage::default();
    let mut first_files = Vec::new();
    let mut advisors = Characters::default();
    let mut warnings = Vec::new();
    let names = layers.key_names()?;
    // The content loader reads idea tags fourth, and ideas after buildings.
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
        for path in script_files::paths(layers, folder, Some(".txt")) {
            coverage.next_file();
            advisors.next_file();
            let read = read_file(
                &layers.read(&path)?,
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
                // The ideas, categories and advisors from the stop on are
                // known by name only, and the rest of the file may define
                // any other: every lookup this file could answer is refused.
                coverage.gap(&path, stop.cause.clone());
                advisors.gap(&path, &stop.cause);
                if database.tags.cut.is_none() {
                    database.tags.cut = Some((
                        known + stop.categories,
                        Gap {
                            path: path.clone(),
                            cause: stop.cause.clone(),
                        },
                    ));
                }
                database
                    .unknown
                    .get_or_insert_with(|| format!("{path}: {}", stop.cause));
                warnings.push((crate::DiagnosticKind::Unsettled, format!(
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
            crate::DiagnosticKind::Warning,
            format!("{repeated} ideas share a lexer token; the first database entry is used."),
        ));
    }
    Ok((content, advisors, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::characters::Authored;

    #[test]
    fn idea_case_aliases_select_the_first_token_and_keep_authored_spelling() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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
        // Rebuilding the matched object in its held group clears the gap;
        // appending a shadowed alias in another group above did not.
        coverage.replace("0");
        let content = IdeaContent::new(content.tags, content.ideas, coverage, content.first_files);
        assert_eq!(content.idea("FIRST").unwrap().unwrap().token, "first");
        assert!(content.idea("absent").is_err());
    }

    #[test]
    fn unknown_modifier_key_is_reported_before_numeric_conversion() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let mut scan = modifiers::ScanFailures::default();
        let mut values = Modifiers::default();
        values.read_all(&[Item {
            key: Some(b"tag".to_vec()),
            key_quoted: false,
            operator: crate::parser::Operator::Equals,
            value: crate::parser::Value::Scalar(b"LEB".to_vec()),
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
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let mut scan = modifiers::ScanFailures::default();
        let mut values = Modifiers::default();
        values.read_all(&[Item {
            key: Some(b"tag".to_vec()),
            key_quoted: false,
            operator: crate::parser::Operator::Equals,
            value: crate::parser::Value::Scalar(b"LEB".to_vec()),
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

    /// Files through the database in order, each by its loader.
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
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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
        // A quoted type is no keyword; `cost` uses integer scanning,
        // so 010 is 8, and a failed scan keeps it.
        let quoted = category("quoted");
        assert_eq!((quoted.kind.as_deref(), quoted.cost), (None, Some(8.0)));
        // The type dispatch uses the value keyword, folded by the lexer;
        // a value that names no type resets it.
        let folded = category("folded");
        assert_eq!(folded.kind.as_deref(), Some("national_spirit"));
        assert_eq!(folded.ledger, 0x4);
        assert_eq!(category("reset").kind, None);
    }

    /// Visit each category in array order, its name and then its slots,
    /// compared exactly: an earlier category holding slot `X` wins over a
    /// later category named `X`. A repeated name adds another object,
    /// which the scan reaches after the first.
    #[test]
    fn categories_resolve_in_array_order_and_repeat_as_new_objects() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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
        // Both blocks read into the one modifier object; `[?x]` does not
        // convert and leaves the 0.
        assert_eq!(
            values(&spirit.modifiers),
            [
                ("stability_factor".to_owned(), 10_000),
                ("war_support_factor".to_owned(), 0)
            ]
        );
        assert_eq!(spirit.modifiers.tooltips(), ["GER_TIP"]);
        assert_eq!(spirit.targeted[0].tag.as_deref(), Some("SOV"));
        // Each idea takes its database index, with the null idea at 0.
        assert_eq!(
            [law.position, designer.position, spirit.position],
            [1, 2, 3]
        );
    }

    #[test]
    fn idea_rules_keep_defaults_repeated_values_and_description_overrides() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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

    /// Rereading a held group keeps its flags: a later occurrence without
    /// `law` keeps it, and a later `law = no` changes every idea of the group.
    /// An idea whose name matches without case is rebuilt in place under
    /// the new spelling; any other key, `use_list_view` included, is an idea.
    /// Every idea then takes its place in the group as its index.
    #[test]
    fn a_group_read_again_keeps_its_flags_and_its_ideas() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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

    /// Idea tags read every group as a new one; registration appends its
    /// ideas to a held group of the same name, whatever their names, with
    /// their database indices, and discards the new group's flags. A quoted
    /// key in `common/ideas` misses the held-group scan and merges the
    /// same way.
    #[test]
    fn a_new_group_of_a_held_name_merges_its_ideas() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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

    /// Both loaders read `idea_categories` and `ideas`: an idea declared
    /// under `common/idea_tags` and a category declared under `common/ideas`
    /// join the database like any other.
    #[test]
    fn both_directories_declare_categories_and_ideas() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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

    /// The value keyword selects ledger bits: `hidden` sets 1, `invalid`
    /// sets 0xFFFF, and the others OR their bits. Any other value is reported
    /// and its block skipped, so a block's fields never become the enclosing
    /// idea's fields.
    #[test]
    fn a_ledger_is_read_as_its_reader_reads_it() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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

    /// One reader per file, in document order: definitions inside ideas,
    /// modifier blocks and conditions register; quoting keeps text; a
    /// reference before its definition keeps its text.
    #[test]
    fn ideas_follow_the_file_reader_table_in_document_order() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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
        // The later `picture` reads the variable its condition defined.
        assert_eq!(second.picture.as_deref(), Some("second_pic"));
        assert_eq!(values(&second.modifiers), [("c".to_owned(), 20_000)]);
        // A string list resolves; before its definition `@trait` stays text.
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
        // An unknown top-level key is skipped, defining nothing.
        assert_eq!(
            texts(by(&file.ideas, "fourth")),
            [
                ("e".to_owned(), "@skipped".to_owned(), 0),
                ("f".to_owned(), "1".to_owned(), 100_000),
                // `idea_categories` is read by key.
                ("g".to_owned(), "0.4".to_owned(), 40_000),
                // Rule fields read by key but skip `icon`.
                ("h".to_owned(), "7".to_owned(), 700_000),
                ("i".to_owned(), "@icon".to_owned(), 0),
            ]
        );
    }

    /// A taken name reads nothing more and the reading goes on, noted. A
    /// block whose reading the port cannot settle refuses the ideas of the
    /// group holding it and every later one: the group's flags may follow
    /// anywhere in it.
    #[test]
    fn unsettled_blocks_refuse_their_group_and_the_rest_of_the_file() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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

    /// A bare key folds; a quoted one has no keyword. In a group a quoted
    /// `"law"` with a scalar is an error the reader records, and with a block
    /// an idea named `law`; in an idea a quoted field is skipped. A failed
    /// conversion leaves the field as it was.
    #[test]
    fn keywords_fold_quoted_keys_are_names_and_failed_values_keep_the_field() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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
        // `designer` is the group's flag, so an idea reports it as unexpected.
        assert!(idea.law && !idea.designer && idea.default);
        assert_eq!(idea.cost, Some(5.0));
        assert_eq!(idea.picture.as_deref(), Some("pic"));
        // `use_list_view` is a boolean flag of the group: a block's `{` is
        // its value and the block's `}` ends the group.
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

    /// A new group named in
    /// NDefines.NCharacter.OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU makes each
    /// entry a character template holding one advisor. Its fields read in
    /// order as an advisor. The slot is the group's unless a `slot` field
    /// names another; the idea token is always the key, an empty name is the
    /// key, and the army/small portrait is `GFX_idea_` followed by the name.
    /// `traits` blocks add up, a failed cost keeps the earlier one, and the
    /// role keeps its `modifier` and `research_bonus`. A held group read
    /// again makes no officer test: its entries are ideas.
    #[test]
    fn officer_corps_groups_are_character_advisor_templates() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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
        // Without the define every group is an idea group.
        let plain = parse_ideas(source, &[], &KeyNames::default());
        assert_eq!(plain.ideas.len(), 4);
        assert_eq!(plain.advisors, Characters::default());
    }

    /// An officer-corps entry past the place the reading cannot follow is
    /// refused like an idea.
    #[test]
    fn a_refused_officer_corps_entry_is_no_advisor() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let officers = ["theorist".to_owned()];
        let file = parse_ideas(
            b"ideas = { theorist = { GER_a = { } GER_b = { modifier = { odd = { } } } GER_c = { } } }",
            &officers, &KeyNames::default(),
        );
        assert!(file.refused.is_some());
        assert_eq!(file.advisors, Characters::default());
        assert_eq!(file.refused_tokens, ["GER_a", "GER_b", "GER_c"]);
    }

    /// A file whose reading stops short leaves every idea, category and
    /// advisor it could define unknown: a lookup names the file and the
    /// cause instead of answering from an earlier layer or from absence. A
    /// later file's definition is known again; a category before the stop
    /// resolves, and a group a later file names whose earlier registration
    /// the stop may hide is refused.
    #[cfg(feature = "test-support")]
    #[test]
    fn a_file_that_stops_short_refuses_what_it_could_define() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let root = std::env::temp_dir().join(format!("installed-content-ideas-gap-{}", std::process::id()));
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
        let (content, advisors, warnings) = read(&layers, &["theorist".to_owned()]).unwrap();
        let gap = content.idea("early").unwrap_err();
        assert_eq!(gap.path, "common/ideas/b.txt");
        assert!(
            gap.cause.starts_with("line 1: x has a block value"),
            "{gap}"
        );
        // Absent is unknown too: b.txt may define it past the stop.
        assert!(content.idea("never").is_err());
        assert!(content.idea("kept").is_err());
        assert!(content.idea("redone").unwrap().is_some());
        // A later group's new entry cannot prove that an unread earlier
        // suffix did not append the token to another group first.
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

    /// A group a later file names that the database does not hold may be
    /// one the unknown rest of an earlier file registers. That decides
    /// whether it is read again or as new, so the file stops there, by name.
    #[test]
    fn a_group_an_earlier_gap_may_hold_is_refused() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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

    /// The replay's classes through this reader: a quoted key is no keyword
    /// and a bare one folds (above); a variable holds within its file only;
    /// a block where fixed5 takes one value is read as the idea's own fields;
    /// the end of text closes every loop.
    #[test]
    fn the_replay_classes_hold_for_ideas() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let file = parse_ideas(
            b"@v = 3 ideas = { country = { a = { cost = { picture = inner } removal_cost = @v } } }",
            &[], &KeyNames::default(),
        );
        let a = &file.ideas[0];
        assert_eq!((a.cost, a.picture.as_deref()), (None, Some("inner")));
        // The block's `}` ended the idea, so `removal_cost` was read by the
        // group and is no field of `a`.
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
        // `@v` is another file's variable: its text, which does not convert.
        assert_eq!(file.ideas[0].removal_cost, None);
    }
}
