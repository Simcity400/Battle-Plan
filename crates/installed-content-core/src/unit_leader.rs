//! Commander traits, skill thresholds and component modifiers from `common/unit_leader`.
//!
//! Every file gets its own reader. The name pass first registers every
//! `leader_traits` entry, skipping its body; the body pass reads the file
//! again. Skill tables fill each category's level-indexed array and replace
//! duplicate levels in source order, with signed `cost` and a category
//! `type`. Each `leader_traits` entry whose key token is registered reads
//! into its trait. Missing, negative or out-of-range levels return the null
//! definition. Component hovers use the recorded retained definition and
//! values; content supplies hidden sets and custom tooltip keys.
//!
//! Both passes read through `parser::native`, which replays the readers'
//! token consumption: every field's reader below is a [`Shape`], and where
//! the text leaves the tree's shape the reading follows the game and names
//! the misread. Keys dispatch on the token as the lexer gives it: a quoted
//! key is never a keyword, a bare one folds. Trait and sub-unit names
//! compare without case; modifier names are found through the lexer's
//! token table.

use std::{collections::BTreeMap, io, sync::Arc};

use crate::{
    ResolvedLayers,
    crt::{self, Scanned},
    key_names::KeyNames,
    modifiers::{self, Modifiers},
    parser::{
        Item, Value,
        native::{self, Field, Kind, Shape, Token},
    },
    scope::ScopeKind,
    script_files,
};

mod division_experience;
mod skills;
mod sub_units;

pub use division_experience::{DivisionExperience, DivisionExperienceProgress};
pub use skills::{Component, Skills};
pub use sub_units::{SubUnit, SubUnits, sub_unit_files, sub_units};

#[cfg(test)]
use sub_units::SUB_UNIT_DIRECTORIES;

const DIRECTORY: &str = "common/unit_leader/";

/// What `common/unit_leader` holds for the commander hovers.
#[derive(Clone, Debug)]
pub struct Leaders<N = String> {
    pub traits: Traits,
    pub skills: Skills,
    /// Where the game's reading of a file leaves the text, by file and line:
    /// the reading goes on as the game's does.
    pub misreads: Vec<N>,
    /// The files as read, for [`Self::reread`].
    files: Vec<(String, crate::content::Bytes)>,
    names: Arc<KeyNames>,
}

impl<N> Default for Leaders<N> {
    fn default() -> Self {
        Self {
            traits: Traits::default(),
            skills: Skills::default(),
            misreads: Vec::new(),
            files: Vec::new(),
            names: Arc::default(),
        }
    }
}

pub use crate::modifiers::Registry;

impl<N: crate::DiagnosticMessage> Leaders<N> {
    /// Reads the content stack's files in the engine's directory order.
    /// `terrain` finds a content terrain type without ASCII case and
    /// gives its registered spelling; each becomes an adjuster.
    pub fn load(
        layers: &ResolvedLayers,
        terrain: &dyn Fn(&str) -> Option<String>,
    ) -> io::Result<Self> {
        // These traits load after buildings and raids.
        let names = layers.key_names()?;
        Ok(Self::read_files(files(layers)?, terrain, &|_| None, names))
    }

    /// The same files read again with the modifier registry the recording
    /// holds, which settles every [`Conditional`] whose name it knows.
    pub fn reread(&self, terrain: &dyn Fn(&str) -> Option<String>, registry: Registry<'_>) -> Self {
        Self::read_files(self.files.clone(), terrain, registry, self.names.clone())
    }

    /// Whether any trait or the skill levels depend on the registry.
    pub fn conditional(&self) -> bool {
        !self.skills.conditions.is_empty()
            || !self.skills.component_conditions.is_empty()
            || self
                .traits
                .iter()
                .any(|effects| !effects.conditions.is_empty())
    }

    /// The loader's two passes over the files in order.
    fn read_files<B: Into<crate::content::Bytes>>(
        files: Vec<(String, B)>,
        terrain: &dyn Fn(&str) -> Option<String>,
        registry: Registry<'_>,
        names: Arc<KeyNames>,
    ) -> Self {
        let files: Vec<(String, crate::content::Bytes)> = files.into_iter()
            .map(|(path, bytes)| (path, bytes.into())).collect();
        let mut leaders = Self {
            names,
            ..Self::default()
        };
        for (path, bytes) in &files {
            leaders.traits.register(bytes, path);
        }
        // Each trait's adjusters hold across its bodies and files.
        let mut adjusters = BTreeMap::new();
        let mut gaps = Vec::new();
        for (path, bytes) in &files {
            gaps.extend(leaders.read(bytes, path, terrain, registry, &mut adjusters));
        }
        // What a file's unread rest holds is unknown: any registered trait's
        // key there dispatches another body, which merges into that trait over
        // what it holds (the name pass registered every file's names first),
        // and a `leader_skills` there adds or replaces levels. So one gap
        // leaves every trait and the skill table unknown, whatever was read
        // before it.
        for reason in gaps {
            for effects in leaders.traits.traits.values_mut() {
                if !effects.unsupported.contains(&reason) {
                    effects.unsupported.push(reason.clone());
                }
            }
            leaders.skills.refuse(reason.clone());
            leaders.traits.unsupported.push(reason);
        }
        leaders.files = files;
        leaders
    }

    /// The body pass over one file. The place its reading cannot be
    /// followed from, if any.
    fn read(
        &mut self,
        bytes: &[u8],
        path: &str,
        terrain: &dyn Fn(&str) -> Option<String>,
        registry: Registry<'_>,
        adjusters: &mut BTreeMap<usize, Vec<String>>,
    ) -> Option<String> {
        let mut schema = Schema {
            path,
            traits: &self.traits,
            terrain,
            registry,
            bodies: Vec::new(),
            adjusters,
            pending: Vec::new(),
            conditions: BTreeMap::new(),
            skills_conditions: Vec::new(),
            component_conditions: Vec::new(),
            current: Current::Top,
            references: Vec::new(),
        };
        // Each file constructs the lexer in mode 0.
        let read = native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::File(Node::Top),
            Some(&self.names),
            &mut |node, field| schema.shape(node, field),
        );
        let Schema {
            bodies,
            conditions,
            skills_conditions,
            component_conditions,
            references,
            ..
        } = schema;
        self.misreads.extend(
            read.misreads
                .iter()
                .map(|misread| N::from_diagnostic((misread.kind, format!("{path} {misread}")))),
        );
        let mut body = bodies.into_iter();
        for item in &read.items {
            match item.keyword().map(|keyword| keyword.folded()).as_deref() {
                Some(b"leader_skills") => {
                    self.skills
                        .read(item.value.items().unwrap_or_default(), path, None)
                }
                Some(b"leader_traits") => {
                    for entry in item.value.items().unwrap_or_default() {
                        // Only an entry dispatched as a trait was read: a
                        // bare key a registered trait has.
                        let dispatched =
                            bare_key(entry).is_some_and(|key| self.traits.index(key).is_some());
                        let (true, Value::Block(fields)) = (dispatched, &entry.value) else {
                            continue;
                        };
                        let Some(index) = body.next() else {
                            continue;
                        };
                        let identity = &self.traits.order[index];
                        let effects = self
                            .traits
                            .traits
                            .get_mut(identity)
                            .expect("dispatched traits are registered");
                        effects.read_into(fields, terrain, &self.traits.names);
                    }
                }
                Some(key) => {
                    if let Some(component) = Component::from_key(key) {
                        self.skills.read(
                            item.value.items().unwrap_or_default(),
                            path,
                            Some(component),
                        );
                    }
                }
                None => {}
            }
        }
        for (index, conditions) in conditions {
            let identity = &self.traits.order[index];
            if let Some(effects) = self.traits.traits.get_mut(identity) {
                for condition in conditions {
                    if !effects.conditions.contains(&condition) {
                        effects.conditions.push(condition);
                    }
                }
            }
        }
        self.skills.depend_on(&skills_conditions);
        for condition in component_conditions {
            if !self.skills.component_conditions.contains(&condition) {
                self.skills.component_conditions.push(condition);
            }
        }
        // The advisor-trait references in reading order, the last kept.
        for (index, slot, reference) in references {
            let identity = &self.traits.order[index];
            if let Some(effects) = self.traits.traits.get_mut(identity) {
                match reference {
                    Ok(reference) => effects.advisor_traits[slot] = Some(reference),
                    Err(reason) => effects.unsupported.push(reason),
                }
            }
        }
        read.unsettled.map(|unsettled| {
            format!(
                "the game's reading of {path} cannot be followed from line {} ({}); everything after it in the file is unknown",
                unsettled.line, unsettled.what
            )
        })
    }
}

#[cfg(feature = "test-support")]
impl Leaders<String> {
    /// `(path, text)` files read through the loader's two passes, for other crates' tests.
    pub fn from_files(
        files: Vec<(String, Vec<u8>)>,
        terrain: &dyn Fn(&str) -> Option<String>,
    ) -> Self {
        Self::read_files(files, terrain, &|_| None, Arc::default())
    }
}

/// The content stack's `common/unit_leader` files: the folder's own
/// `.txt` files ([`script_files::paths`]), with their bytes; each path as text, for messages.
fn files(layers: &ResolvedLayers) -> io::Result<Vec<(String, crate::content::Bytes)>> {
    script_files::paths(layers, DIRECTORY, Some(".txt"))?
        .into_iter()
        .map(|path| {
            let bytes = path.read(layers)?;
            Ok((path.to_string(), bytes))
        })
        .collect()
}

/// Which reader a field goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    /// The body pass's file loop.
    Top,
    /// A skill table.
    SkillTable,
    /// One skill level.
    Level,
    /// The body pass's `leader_traits` loop.
    TraitTable,
    /// The registered trait at this index of [`Traits::order`].
    Trait(usize),
    /// The trait's modifier-block loop: adjusters, then shared modifiers.
    TraitModifier(usize),
    /// The modifier readers ([`modifiers::shape`]) and `hidden_modifier` loop.
    Modifier(modifiers::Node),
    /// A unit adjuster.
    Adjuster,
    /// The sub-unit modifier table, and one entry.
    SubUnitTable,
    SubUnit,
    /// The trait's `trait_xp_factor` loop.
    Experience,
    /// Parent-trait fields.
    Parent,
    /// Mean-time fields.
    Mtth,
    /// The trait's `unit_type` key loop; no child value reader is called.
    UnitTypes,
}

/// Which record a field read in pass 2 belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Current {
    Top,
    Skills,
    Component,
    Trait(usize),
}

/// Pass 2's field dispatch, and what it learns while the reader reads.
struct Schema<'a> {
    path: &'a str,
    traits: &'a Traits,
    terrain: &'a dyn Fn(&str) -> Option<String>,
    registry: Registry<'a>,
    /// Each trait body dispatched, by index into [`Traits::order`], in
    /// reading order.
    bodies: Vec<usize>,
    /// The adjuster keys each trait has read.
    adjusters: &'a mut BTreeMap<usize, Vec<String>>,
    /// Block values under modifier entries read so far in this file.
    pending: Vec<Conditional>,
    conditions: BTreeMap<usize, Vec<Conditional>>,
    skills_conditions: Vec<Conditional>,
    component_conditions: Vec<Conditional>,
    current: Current,
    /// Each `*_advisor_trait` field in reading order: the trait's index, the
    /// field's slot and what the lexer's current token resolves to
    /// ([`TraitReference::read`]).
    references: Vec<(usize, usize, Result<TraitReference, String>)>,
}

impl Schema<'_> {
    fn shape(&mut self, node: Node, field: &Field<'_>) -> Shape<Node> {
        let keyword = field.keyword();
        let keyword = keyword.as_deref();
        match node {
            Node::Top => {
                self.current = Current::Top;
                match keyword {
                    Some(b"leader_skills") => {
                        self.current = Current::Skills;
                        self.skills_conditions.extend(self.pending.iter().cloned());
                        Shape::Object(Node::SkillTable)
                    }
                    Some(b"leader_traits") => Shape::Object(Node::TraitTable),
                    // The body pass reads component tables through the same skill
                    // constructor.
                    Some(key) if Component::from_key(key).is_some() => {
                        self.current = Current::Component;
                        self.component_conditions
                            .extend(self.pending.iter().cloned());
                        Shape::Object(Node::SkillTable)
                    }
                    // Any other key, a quoted one included, is reported and skipped.
                    _ => Shape::Skip,
                }
            }
            // A positive level through signed integer conversion; any other
            // key is reported and skipped.
            Node::SkillTable if crate::fixed5::decimal_i32(&field.key.text) > 0 => {
                Shape::Object(Node::Level)
            }
            Node::SkillTable => Shape::Skip,
            Node::Level => match keyword {
                Some(b"modifier" | b"building_module_modifier") => {
                    Shape::Object(Node::Modifier(modifiers::Node::Modifier))
                }
                // `cost` uses signed integer field conversion.
                Some(b"cost") => Shape::One,
                // `type` dispatches on the value token's id: a category keyword is
                // stored and nothing more read; any other value, a `{` included,
                // is reported, with a current block skipped.
                Some(b"type") if leader_category(field.value_keyword().as_deref()).is_some() => {
                    Shape::One
                }
                Some(b"type") => Shape::Skip,
                _ => Shape::Skip,
            },
            // Find the trait by the key's token id: a bare key registered by
            // the name pass; any other is reported and skipped.
            Node::TraitTable => match keyword.and(self.traits.index(&field.key.text)) {
                Some(index) => {
                    self.current = Current::Trait(index);
                    self.bodies.push(index);
                    let pending = self.pending.clone();
                    self.conditions.entry(index).or_default().extend(pending);
                    Shape::Object(Node::Trait(index))
                }
                None => Shape::Skip,
            },
            Node::Trait(index) => {
                // Resolve the lexer's current token here.
                let slot = match keyword {
                    Some(b"specialist_advisor_trait") => Some(0),
                    Some(b"expert_advisor_trait") => Some(1),
                    Some(b"genius_advisor_trait") => Some(2),
                    _ => None,
                };
                if let Some(slot) = slot {
                    self.references
                        .push((index, slot, TraitReference::read(field.current)));
                }
                trait_field(index, keyword)
            }
            Node::TraitModifier(index) => {
                // Find the adjuster by the key's token id: static condition tokens
                // and the content's terrain types.
                let adjuster = keyword.and_then(|_| {
                    let text = std::str::from_utf8(&field.key.text).ok()?;
                    condition_adjuster(text)
                        .map(str::to_owned)
                        .or_else(|| (self.terrain)(text))
                });
                match adjuster {
                    Some(key) => {
                        let seen = self.adjusters.entry(index).or_default();
                        let default = key.eq_ignore_ascii_case(DEFAULT_TOKEN_KEY);
                        let repeated = seen.contains(&key);
                        seen.push(key);
                        match (repeated, default) {
                            // Read a repeat only while its token is the default `none`.
                            (true, true) => Shape::Object(Node::Adjuster),
                            (true, false) => Shape::One,
                            // The new adjuster is read, then read again by the trait loop.
                            (false, true) => Shape::Twice(Node::Adjuster),
                            (false, false) => Shape::Object(Node::Adjuster),
                        }
                    }
                    None => self.modifier(modifiers::Node::Modifier, field),
                }
            }
            Node::Modifier(node) => self.modifier(node, field),
            Node::Adjuster => match keyword {
                Some(b"attack" | b"defence" | b"movement") => Shape::One,
                _ => Shape::Skip,
            },
            Node::SubUnitTable => Shape::Object(Node::SubUnit),
            Node::SubUnit => match keyword {
                Some(b"units") => Shape::Object(Node::Adjuster),
                _ => self.modifier(modifiers::Node::Modifier, field),
            },
            // Find the trait by the key's token id; a key naming none is
            // reported and skipped.
            Node::Experience => match keyword.and(self.traits.index(&field.key.text)) {
                Some(_) => Shape::One,
                None => Shape::Skip,
            },
            // Parent fields: `traits` reads a string list, `num_parents_needed`
            // uses signed integer conversion; any other key returns at once,
            // its value token read and nothing skipped.
            Node::Parent => match keyword {
                Some(b"traits") => Shape::List,
                _ => Shape::One,
            },
            Node::Mtth => native::mean_time_to_happen(field),
            Node::UnitTypes => Shape::One,
        }
    }

    /// The modifier readers' dispatch ([`modifiers::shape`]) with the registry.
    /// A block value the registry does not settle is read as the skip and
    /// becomes a [`Conditional`] of the record being read.
    fn modifier(&mut self, node: modifiers::Node, field: &Field<'_>) -> Shape<Node> {
        let mut undecided = Vec::new();
        let shape = modifiers::shape(node, field, self.registry, &mut undecided);
        for undecided in undecided {
            let condition = Conditional {
                modifier: undecided.name,
                place: format!("{} line {}", self.path, undecided.line),
            };
            if !self.pending.contains(&condition) {
                self.pending.push(condition.clone());
            }
            match self.current {
                Current::Trait(index) => self.conditions.entry(index).or_default().push(condition),
                Current::Skills => self.skills_conditions.push(condition),
                Current::Component => self.component_conditions.push(condition),
                Current::Top => {}
            }
        }
        shape.map(Node::Modifier)
    }
}

/// Trait-field dispatch, per key.
fn trait_field(index: usize, keyword: Option<&[u8]>) -> Shape<Node> {
    match keyword {
        Some(
            b"modifier"
            | b"non_shared_modifier"
            | b"field_marshal_modifier"
            | b"corps_commander_modifier",
        ) => Shape::Object(Node::TraitModifier(index)),
        Some(b"sub_unit_modifiers") => Shape::Object(Node::SubUnitTable),
        Some(b"trait_xp_factor") => Shape::Object(Node::Experience),
        Some(b"parent") => Shape::Object(Node::Parent),
        // A category token, else a list only on `{`.
        Some(b"type") => Shape::ListIfOpen,
        // String and raw-token lists loop to `}` whatever the value was.
        Some(b"any_parent" | b"all_parents" | b"allowed_ship_equipments") => Shape::List,
        // Mean-time fields read through an object reader.
        Some(b"ai_will_do" | b"new_commander_weight") => Shape::Object(Node::Mtth),
        // Each condition reads a conjunction body.
        Some(
            b"allowed" | b"prerequisites" | b"gain_xp" | b"gain_xp_leader" | b"unit_trigger"
            | b"country_trigger",
        ) => Shape::Trigger,
        // Read keys unconditionally to `}` and look up each key token as a
        // ship class, consuming no child body.
        Some(b"unit_type") => Shape::Object(Node::UnitTypes),
        // Effects use the shared effect-script reader in character scope.
        Some(b"on_add" | b"on_remove" | b"daily_effect") => {
            Shape::EffectIn(ScopeKind::Character.bit())
        }
        // num_parents_needed belongs inside a parent body; at trait level
        // it is reported and its block skipped.
        Some(b"num_parents_needed") => Shape::Skip,
        // `slot` and the advisor-trait references read the lexer's current
        // token, not the value slot.
        Some(
            b"slot"
            | b"specialist_advisor_trait"
            | b"expert_advisor_trait"
            | b"genius_advisor_trait",
        ) => Shape::Current,
        Some(name)
            if skill_slot(name).is_some()
                || matches!(
                    name,
                    b"leader_default_proximity_offset"
                        | b"override_effect_tooltip"
                        | b"custom_effect_tooltip"
                )
                || undisplayed_field(name) =>
        {
            Shape::One
        }
        // Hand to the base reader, which reports it and skips its block.
        _ => Shape::Skip,
    }
}

#[cfg(test)]
mod trigger_reader_tests;

/// Leader categories by the value token's id: `field_marshal` is 0,
/// `corps_commander` is 1, `navy` is 2 and `operative` is 3; any other
/// token is none and is reported as an unknown leader type.
fn leader_category(keyword: Option<&[u8]>) -> Option<i32> {
    match keyword? {
        b"field_marshal" => Some(0),
        b"corps_commander" => Some(1),
        b"navy" => Some(2),
        b"operative" => Some(3),
        _ => None,
    }
}

/// A block value under a modifier entry, whose reading depends on the
/// modifier registry, which the content does not hold. A name that is no
/// modifier is reported and its block skipped, defining nothing, which is
/// how the file is read here. A registered modifier's value goes through
/// fixed5 conversion, which takes the `{` and skips nothing, so the game
/// reads the block's contents as the enclosing entries and everything after
/// it in the file differs. The consumer settles it with the recorded
/// modifier definitions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conditional {
    /// The entry's name as authored; the registry finds it without case.
    pub modifier: String,
    /// The file and line that hold it.
    pub place: String,
}

impl std::fmt::Display for Conditional {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} gives {} a block value, which the game's reader takes as one value when {} is a registered modifier, reading the rest of the file differently",
            self.place, self.modifier, self.modifier
        )
    }
}

/// A `specialist_advisor_trait`, `expert_advisor_trait` or
/// `genius_advisor_trait` value: the lexer's current token's id against
/// the country-leader trait database, loaded before this pass. Lookup
/// gives the first trait of that token or the null object, which is
/// reported and stored as null.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraitReference {
    /// A bare token the lexer looked up, as authored: the trait registered
    /// under that name without case, if any.
    Name(String),
    /// A token no trait has, as authored: a quoted token (0xF), a number
    /// (0xC) or punctuation. The field is null.
    Null(String),
}

impl TraitReference {
    /// The current token resolved by the advisor-trait reader, or why a bare
    /// name cannot be held. An `@name` is the variable's own token (resolution
    /// only updates the value slot), which no trait has: a trait key starting
    /// with `@` is a definition to the trait loader.
    fn read(current: &Token) -> Result<Self, String> {
        let shown = String::from_utf8_lossy(crate::parser::c_string(&current.text)).into_owned();
        if current.kind != Kind::Bare
            || !crate::parser::looked_up(&current.text, false)
            || current.text.first() == Some(&b'@')
        {
            return Ok(Self::Null(shown));
        }
        std::str::from_utf8(&current.text)
            .map(|name| Self::Name(name.to_owned()))
            .map_err(|_| format!("advisor trait {shown} is a name that is not UTF-8"))
    }
}

/// A terrain or combat-condition adjuster: `attack`, `defence` and
/// `movement`, fixed5.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Adjuster {
    pub key: String,
    pub attack: i64,
    pub defence: i64,
    pub movement: i64,
}

/// One `sub_unit_modifiers` entry: `units = {}` is the adjuster,
/// the rest is a modifier block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubUnitModifiers {
    /// The sub-unit key as authored; the adjuster's own key is unused.
    pub adjuster: Adjuster,
    /// The entry's modifier block, as [`Modifiers::read`] stores it.
    pub block: Modifiers,
}

/// What trait descriptions and the trait item hover describe, as read
/// by the shared trait-field reader.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraitEffects {
    /// The trait's registered spelling: the first definition's key.
    /// Registration keeps the first spelling of a name differing only in case.
    pub name: String,
    /// `type` category mask equals 1: warns outside contexts 0 and 4.
    /// Omitted type has mask 0xF, so the warning is off.
    pub army_only: bool,
    /// `override_effect_tooltip`: when present, the only lines. Each is the
    /// value slot's text to its NUL, with no line-feed check.
    pub override_tooltips: Vec<String>,
    /// `custom_effect_tooltip`, appended and read as `override_effect_tooltip`.
    pub custom_tooltips: Vec<String>,
    /// `modifier`, `non_shared_modifier`, `field_marshal_modifier` and
    /// `corps_commander_modifier`, in the description's order: each block's
    /// modifiers as [`Modifiers::read`] stores them, the adjusters apart.
    pub blocks: [Modifiers; 4],
    /// Adjusters of every block in first-appearance order; a repeated key
    /// re-reads the same adjuster.
    pub adjusters: Vec<Adjuster>,
    /// `sub_unit_modifiers` entries in read order, repeats included: the
    /// consumer resolves each key to its database index, keeps the first of
    /// a repeated sub-unit and drops unknown keys.
    pub sub_units: Vec<SubUnitModifiers>,
    /// `attack_skill`, `defense_skill`, `logistics_skill`, `planning_skill`,
    /// `maneuvering_skill`, `coordination_skill`, in component order.
    pub skills: [i32; 6],
    /// `leader_default_proximity_offset`.
    pub proximity_offset: i32,
    /// Signed whole-point threshold from `cost`; omitted input keeps the native default.
    pub required_experience: Option<i32>,
    /// `trait_xp_factor = { trait = value }`, authored order, each named
    /// by the trait's registered spelling: lookup uses the key's token
    /// and reports any other key.
    pub experience_factors: Vec<(String, i64)>,
    /// `slot`: the current token's text to its NUL; `None` when empty.
    pub advisor_slot: Option<String>,
    /// The three advisor traits as read; `None` for a field never read,
    /// which stays null.
    pub advisor_traits: [Option<TraitReference>; 3],
    /// Block values under modifier entries read before or inside a body of
    /// this trait in its file: a registered one leaves the trait unknown.
    pub conditions: Vec<Conditional>,
    /// Forms whose effects the port cannot keep (text that is not UTF-8, a
    /// reading the port cannot follow); the effects are refused.
    pub unsupported: Vec<String>,
    /// Entries the native reader reports and ignores or reads as its default,
    /// which the description therefore omits or shows at that default.
    pub ignored: Vec<String>,
}

const DEFAULT_TRAIT_EXPERIENCE_COST: i32 = 1000;

impl TraitEffects {
    /// The constructor's whole-point default, independently of authored content.
    pub fn required_experience(&self) -> i32 {
        self.required_experience
            .unwrap_or(DEFAULT_TRAIT_EXPERIENCE_COST)
    }
}

/// Every `leader_traits` definition of the content stack in load order.
/// The name pass keeps every definition, and the body pass reads each body
/// into the first trait of that key, so a repeated key merges into the first.
#[derive(Clone, Debug, Default)]
pub struct Traits {
    /// Keyed by the ASCII-lowercased name: token identity hashes a folded
    /// copy and compares without ASCII case, so names that differ only in
    /// case are one trait.
    traits: BTreeMap<Vec<u8>, TraitEffects>,
    /// Every registered identity, in registration order.
    order: Vec<Vec<u8>>,
    /// Every registered name's identity and first spelling from the name
    /// pass, which `trait_xp_factor` finds a trait among.
    names: BTreeMap<Vec<u8>, Result<String, String>>,
    /// Every key the name pass constructs a definition for, in order and
    /// ASCII-lowercased, a repeated or non-UTF-8 name included: the
    /// definition's ordinal is its position after the null object at index 0.
    definitions: Vec<Vec<u8>>,
    /// Readings the port cannot follow, and entries it cannot keep.
    pub unsupported: Vec<String>,
}

/// Condition adjusters use folded names; terrain adjusters come from content.
fn condition_adjuster(text: &str) -> Option<&'static str> {
    match text.to_ascii_lowercase().as_str() {
        "night" => Some("night"),
        "amphibious" => Some("amphibious"),
        "river" => Some("river"),
        "fort" => Some("fort"),
        "snow" => Some("snow"),
        _ => None,
    }
}

/// Skill fields select the corresponding slot in [`TraitEffects::skills`].
fn skill_slot(keyword: &[u8]) -> Option<usize> {
    match keyword {
        b"attack_skill" => Some(0),
        b"defense_skill" => Some(1),
        b"logistics_skill" => Some(2),
        b"planning_skill" => Some(3),
        b"maneuvering_skill" => Some(4),
        b"coordination_skill" => Some(5),
        _ => None,
    }
}

/// Admitted scalar fields whose values the trait hover does not display.
fn undisplayed_field(keyword: &[u8]) -> bool {
    matches!(
        keyword,
        b"trait_type"
            | b"mutually_exclusive"
            | b"gui_row"
            | b"gui_column"
            | b"gain_xp_on_spotting"
            | b"cost"
            | b"enable_ability"
            | b"attack_skill_factor"
            | b"defense_skill_factor"
            | b"planning_skill_factor"
            | b"logistics_skill_factor"
            | b"maneuvering_skill_factor"
            | b"coordination_skill_factor"
            | b"show_in_combat"
            | b"custom_prerequisite_tooltip"
            | b"custom_gain_xp_trigger_tooltip"
    )
}

/// Static token 0x165, the unit adjuster's default token, with spelling
/// `none`. The trait reader reads an adjuster's block again only while
/// its token is this default. The identity is a static-token registration,
/// independent of content names.
const DEFAULT_TOKEN_KEY: &str = "none";

/// A trait or sub-unit name as its registry finds it: ASCII-lowercased text.
fn identity(name: &[u8]) -> Vec<u8> {
    name.to_ascii_lowercase()
}

/// Every key the readers of the trait hover do not show but the game reads,
/// with the reason, for [`TraitEffects::ignored`].
fn unknown(key: &Item) -> String {
    format!(
        "unknown trait key {}",
        String::from_utf8_lossy(key.key.as_deref().unwrap_or_default())
    )
}

impl Traits {
    pub fn spelling(&self, token: &str) -> Result<Option<&str>, &str> {
        self.names
            .get(&identity(crate::parser::c_string(token.as_bytes())))
            .map(|name| name.as_deref().map_err(String::as_str))
            .transpose()
    }

    /// Full trait name: try the current country tag, then the registered
    /// name, without cosmetics.
    pub fn name_key(
        &self,
        token: &str,
        tag: &str,
        mut defines: impl FnMut(&str) -> bool,
    ) -> Result<Option<String>, &str> {
        let Some(name) = self.spelling(token)? else {
            return Ok(None);
        };
        let own = format!("{tag}_{name}");
        Ok(Some(if defines(&own) { own } else { name.to_owned() }))
    }

    /// The learned-trait assignment tree uses this sprite for every kind,
    /// including exile traits. Its ordinary roster grid has a different
    /// exile flag path.
    pub fn assignment_sprite(&self, token: &str) -> Result<Option<String>, &str> {
        Ok(self
            .spelling(token)?
            .map(|name| format!("GFX_trait_{name}")))
    }

    /// The trait named `name` in any case, if the content defines it.
    pub fn get(&self, name: &str) -> Option<&TraitEffects> {
        self.traits.get(&identity(name.as_bytes()))
    }

    /// Every trait, by its registered spelling's identity.
    pub fn iter(&self) -> impl Iterator<Item = &TraitEffects> {
        self.traits.values()
    }

    /// The name pass alone over the content stack's files: every
    /// registered trait, its effects unread.
    pub fn registered(layers: &ResolvedLayers) -> io::Result<Self> {
        layers.recorded_product("registered unit leader traits", 1, &[], |layers| {
            let mut traits = Self::default();
            for (path, bytes) in files(layers)? {
                traits.register(&bytes, &path);
            }
            Ok(traits)
        }).map(|traits| traits.as_ref().clone())
    }

    /// Whether a trait of this name, in any case, is registered.
    pub fn contains(&self, name: &[u8]) -> bool {
        self.names.contains_key(&identity(name))
    }

    /// The ordinal of the first appended definition whose key equals `name`
    /// without ASCII case; `None` for the null object (no such trait). A
    /// repeated key is a later definition that this lookup never finds.
    /// The register pass skips every body, so it is always followed to the
    /// end; `Err` only for an ordinal past i32. `name` is compared as a C string.
    pub fn first_ordinal(&self, name: &[u8]) -> Result<Option<i32>, String> {
        let folded = crate::parser::c_string(name).to_ascii_lowercase();
        match self.definitions.iter().position(|key| *key == folded) {
            Some(index) => Ok(Some(i32::try_from(index + 1).map_err(|_| {
                format!(
                    "trait {} is definition {index}, beyond the ordinal range",
                    String::from_utf8_lossy(name)
                )
            })?)),
            None => Ok(None),
        }
    }

    /// Every displayable registered trait's first spelling, in registration order.
    pub fn spellings(&self) -> impl Iterator<Item = &str> {
        self.order
            .iter()
            .filter_map(|identity| self.names.get(identity)?.as_deref().ok())
    }

    /// The registered trait a key token names, by index into `order`.
    fn index(&self, name: &[u8]) -> Option<usize> {
        let identity = identity(name);
        self.order.iter().position(|known| *known == identity)
    }

    /// The name pass: the file's `leader_traits` entries by key, each body
    /// skipped, every key registered with its first spelling. Nothing here
    /// is read by key, so the reading is always followed to the end
    /// ([`native::Read::unsettled`]).
    fn register(&mut self, bytes: &[u8], path: &str) {
        #[derive(Clone, Copy)]
        enum Pass {
            Top,
            Table,
        }
        let mut names = Vec::new();
        // Each file constructs the lexer in mode 0.
        native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::File(Pass::Top),
            None,
            &mut |pass, field| match pass {
                Pass::Top if field.keyword_is("leader_traits") => Shape::Object(Pass::Table),
                Pass::Top => Shape::Skip,
                Pass::Table => {
                    names.push(field.key.text.clone());
                    Shape::Skip
                }
            },
        );
        // The name pass constructs a definition for every key, as a C string
        // read to the first NUL.
        self.definitions.extend(
            names
                .iter()
                .map(|name| crate::parser::c_string(name).to_ascii_lowercase()),
        );
        for name in names {
            // The name pass registers byte names, and the body pass still reads
            // their bodies by token. Refusing the displayed name must not skip
            // its variable definitions or change later token reading. Byte
            // identities also keep distinct invalid UTF-8 names apart.
            let identity = identity(&name);
            let spelling = crate::parser::text(&name).map(str::to_owned).map_err(|why| {
                format!(
                    "{path}: leader_traits entry {} has a name that is not UTF-8; its effects cannot be displayed ({why})",
                    String::from_utf8_lossy(&name),
                )
            });
            if !self.names.contains_key(&identity) {
                let mut unsupported = Vec::new();
                if let Err(reason) = &spelling {
                    self.unsupported.push(reason.clone());
                    unsupported.push(reason.clone());
                }
                self.names.insert(identity.clone(), spelling.clone());
                self.order.push(identity.clone());
                self.traits.insert(
                    identity.clone(),
                    TraitEffects {
                        name: String::from_utf8_lossy(&name).into_owned(),
                        unsupported,
                        ..TraitEffects::default()
                    },
                );
            }
        }
    }
}

/// A value copied as a C string (the value slot's text, or the current
/// token's for [`Shape::Current`], to its NUL, with no line-feed check:
/// [`Value::c_str`]), or why the port cannot hold it.
fn c_string<'a>(name: &str, item: &'a Item) -> Result<&'a str, String> {
    item.value.c_str().map_err(|why| match why {
        crate::parser::NotText::NotUtf8(_) => format!(
            "{name} value {} is not UTF-8",
            String::from_utf8_lossy(crate::parser::c_string(scalar(item)))
        ),
        crate::parser::NotText::Block => format!("{name} value: {why}"),
    })
}

/// A field's value as the reader saw it; a skipped block reads as empty.
fn scalar(item: &Item) -> &[u8] {
    item.value.scalar().unwrap_or_default()
}

/// A bare key as the reader dispatched it: not quoted, and no punctuation
/// token (a key read from `=`, `{` or `}` is none of the reader's names).
fn bare_key(item: &Item) -> Option<&[u8]> {
    let key = item.key.as_deref()?;
    let punctuation = matches!(
        key,
        b"=" | b"<" | b">" | b"{" | b"}" | b"(" | b")" | b"," | b"?="
    );
    (!item.key_quoted && !punctuation && item.keyword().is_some()).then_some(key)
}

/// A field's folded keyword ([`Item::keyword`]).
fn keyword(item: &Item) -> Option<Vec<u8>> {
    item.keyword().map(|keyword| keyword.folded())
}

impl TraitEffects {
    /// One body as the trait reader read it, into this trait.
    fn read_into(
        &mut self,
        fields: &[Item],
        terrain: &dyn Fn(&str) -> Option<String>,
        traits: &BTreeMap<Vec<u8>, Result<String, String>>,
    ) {
        for field in fields {
            let name = keyword(field);
            let Some(name) = name.as_deref() else {
                // A quoted key, or one the lexer never looks up: reported and
                // its block skipped.
                self.ignored.push(format!(
                    "trait key {} is not a keyword (quoted, or never looked up by the lexer)",
                    String::from_utf8_lossy(field.key.as_deref().unwrap_or_default())
                ));
                continue;
            };
            let name_text = String::from_utf8_lossy(name).into_owned();
            let block = match name {
                b"modifier" => Some(0),
                b"non_shared_modifier" => Some(1),
                b"field_marshal_modifier" => Some(2),
                b"corps_commander_modifier" => Some(3),
                _ => None,
            };
            if let Some(block) = block {
                self.read_block(
                    block,
                    &name_text,
                    field.value.items().unwrap_or_default(),
                    terrain,
                );
                continue;
            }
            let skill = skill_slot(name);
            if skill.is_some() || matches!(name, b"leader_default_proximity_offset" | b"cost") {
                // A failed signed integer scan is reported and leaves the field
                // as it was; EOF stores nothing and reports nothing.
                let value = scalar(field);
                match crt::scan_i32(value) {
                    Scanned::Stored(read) => match skill {
                        Some(index) => self.skills[index] = read,
                        None if name == b"cost" => self.required_experience = Some(read),
                        None => self.proximity_offset = read,
                    },
                    Scanned::End => {}
                    Scanned::NoMatch => self.ignored.push(format!(
                        "{name_text} value {} does not scan as an integer; the game keeps the previous value",
                        String::from_utf8_lossy(value)
                    )),
                }
                continue;
            }
            match name {
                b"type" => {
                    // One category, or OR the valid list members. Empty/invalid input
                    // stores 0xF; invalid members are reported and skipped.
                    let categories = field.value.items().unwrap_or(std::slice::from_ref(field));
                    let mut mask = None;
                    for category in categories {
                        let bits = match category.value_keyword().as_deref() {
                            Some("field_marshal") => Some(1),   // 0x34E2
                            Some("corps_commander") => Some(2), // 0x34C7
                            Some("navy") => Some(4),            // 0x289E
                            Some("operative") => Some(8),       // 0x3D13
                            Some("land") => Some(3),            // 0x2E94
                            Some("all") => Some(15),            // 0x2B97
                            _ => None,
                        };
                        if let Some(bits) = bits {
                            mask = Some(mask.unwrap_or(0) | bits);
                        } else {
                            self.ignored.push(format!(
                                "type value {} is not a category keyword; the game ignores it",
                                String::from_utf8_lossy(scalar(category))
                            ));
                        }
                    }
                    self.army_only = mask.unwrap_or(15) == 1;
                }
                b"override_effect_tooltip" | b"custom_effect_tooltip" => {
                    match c_string(&name_text, field) {
                        Ok(token) if name == b"override_effect_tooltip" => {
                            self.override_tooltips.push(token.into())
                        }
                        Ok(token) => self.custom_tooltips.push(token.into()),
                        Err(reason) => self.unsupported.push(reason),
                    }
                }
                b"sub_unit_modifiers" => {
                    self.read_sub_units(field.value.items().unwrap_or_default())
                }
                b"trait_xp_factor" => {
                    for item in field.value.items().unwrap_or_default() {
                        let key = String::from_utf8_lossy(item.key.as_deref().unwrap_or_default());
                        // The trait is found by the key's token; a quoted key, or a name
                        // no trait has, was reported and skipped.
                        let named = item
                            .keyword()
                            .and_then(|keyword| traits.get(&identity(keyword.text())));
                        let Some(named) = named else {
                            self.ignored.push(format!(
                                "trait_xp_factor {key} names no trait (none matches its token)"
                            ));
                            continue;
                        };
                        let named = match named {
                            Ok(named) => named.clone(),
                            Err(reason) => {
                                self.unsupported.push(format!("trait_xp_factor: {reason}"));
                                continue;
                            }
                        };
                        // Fixed5 conversion into a zero: a failed scan pushes that zero.
                        let value = scalar(item);
                        let read = crate::fixed5::script_fixed5(value).unwrap_or_else(|| {
                            self.ignored.push(format!(
                                "trait_xp_factor {key} value {} does not scan; the game reads zero",
                                String::from_utf8_lossy(value)
                            ));
                            0
                        });
                        self.experience_factors.push((named, read));
                    }
                }
                // An empty slot is no slot: show advisor lines only when the
                // slot string has a length.
                b"slot" => match c_string(&name_text, field) {
                    Ok(slot) => self.advisor_slot = Some(slot.to_owned()).filter(|s| !s.is_empty()),
                    Err(reason) => self.unsupported.push(reason),
                },
                // The advisor traits were resolved as the reader dispatched
                // them, from the current token's kind (Schema::references).
                name if trait_field(0, Some(name)) == Shape::Skip => {
                    self.ignored.push(unknown(field))
                }
                // Read by the game for what this hover does not display.
                _ => {}
            }
        }
    }

    /// The modifier-block loop: adjuster keys first, else the block's shared
    /// modifiers. An adjuster read is a block; a repeat the game leaves
    /// unread (only the default token reads a repeat) is its `{` alone.
    fn read_block(
        &mut self,
        block: usize,
        name: &str,
        items: &[Item],
        terrain: &dyn Fn(&str) -> Option<String>,
    ) {
        for item in items {
            let adjuster_key = item.keyword().and_then(|keyword| {
                let text = std::str::from_utf8(keyword.text()).ok()?;
                condition_adjuster(text)
                    .map(str::to_owned)
                    .or_else(|| terrain(text))
            });
            if let Some(adjuster_key) = adjuster_key {
                let index = match self
                    .adjusters
                    .iter()
                    .position(|adjuster| adjuster.key == adjuster_key)
                {
                    Some(index) => index,
                    None => {
                        self.adjusters.push(Adjuster {
                            key: adjuster_key,
                            ..Adjuster::default()
                        });
                        self.adjusters.len() - 1
                    }
                };
                if let Value::Block(fields) = &item.value {
                    let mut adjuster = std::mem::take(&mut self.adjusters[index]);
                    read_adjuster(&mut adjuster, fields, &mut self.ignored);
                    self.adjusters[index] = adjuster;
                }
                continue;
            }
            self.blocks[block].read(item);
            self.note_modifier(name, item);
        }
        // A key, value or tooltip text Modifiers::read cannot hold.
        if let Some(cause) = self.blocks[block].unheld.cause() {
            let reason = format!("{name}: {cause}");
            if !self.unsupported.contains(&reason) {
                self.unsupported.push(reason);
            }
        }
    }

    /// What the modifier reader reports and goes on from in one entry
    /// [`Modifiers::read`] stored.
    fn note_modifier(&mut self, name: &str, item: &Item) {
        let entry = String::from_utf8_lossy(item.key.as_deref().unwrap_or_default()).into_owned();
        match keyword(item).as_deref() {
            Some(b"name" | b"data" | b"added_modifier" | b"custom_modifier_tooltip") => {}
            // A `{` value is the loop; any other value is reported.
            Some(b"hidden_modifier") => match item.value.items() {
                Some(hidden) => {
                    for inner in hidden {
                        let key = String::from_utf8_lossy(inner.key.as_deref().unwrap_or_default())
                            .into_owned();
                        self.note_value(name, &key, inner);
                    }
                }
                None => self.ignored.push(format!(
                    "{name}.{entry} is not a block; the game's reader records it and reads nothing more"
                )),
            },
            _ => self.note_value(name, &entry, item),
        }
    }

    /// Fixed5 conversion reads into a zero, so a value that does not
    /// convert is a zero entry; a skipped block (a [`Conditional`])
    /// is the zero of a name that is no modifier.
    fn note_value(&mut self, name: &str, entry: &str, item: &Item) {
        if item.keyword().is_none() && !item.key_quoted {
            return;
        }
        let Value::Scalar(value) = &item.value else {
            self.ignored.push(format!(
                "{name}.{entry} is a block, which the game's reader skips when {entry} is no modifier"
            ));
            return;
        };
        if modifiers::read_fixed5(item).is_some() {
            return;
        }
        let text = String::from_utf8_lossy(value);
        self.ignored.push(if !item.quoted() && text.starts_with('@') {
            format!(
                "{name}.{entry} names the variable {text}, which is not defined before it; the game reads zero"
            )
        } else {
            format!("{name}.{entry} value {text} does not scan as a number; the game reads zero")
        });
    }

    /// Each `key = { units = {...} modifiers }`, as read.
    fn read_sub_units(&mut self, items: &[Item]) {
        for item in items {
            let unit_bytes = item.key.as_deref().unwrap_or_default();
            let unit = String::from_utf8_lossy(unit_bytes).into_owned();
            let label = format!("sub_unit_modifiers {unit}");
            let mut entry = SubUnitModifiers {
                adjuster: Adjuster {
                    key: unit.clone(),
                    ..Adjuster::default()
                },
                block: Modifiers::default(),
            };
            for field in item.value.items().unwrap_or_default() {
                // `units` is a keyword.
                if field.keyword_is("units") {
                    read_adjuster(
                        &mut entry.adjuster,
                        field.value.items().unwrap_or_default(),
                        &mut self.ignored,
                    );
                } else {
                    entry.block.read(field);
                    self.note_modifier(&label, field);
                }
            }
            if let Some(cause) = entry.block.unheld.cause() {
                self.unsupported.push(format!("{label}: {cause}"));
            }
            if std::str::from_utf8(unit_bytes).is_err() {
                // No sub-unit the port can name: its lines are refused.
                self.unsupported
                    .push(format!("{label} has a key that is not UTF-8"));
            }
            self.sub_units.push(entry);
        }
    }
}

/// Unit adjuster fields: `attack`, `defence` and `movement`; a failed scan
/// keeps the previous value, and any other key is reported and skipped.
fn read_adjuster(adjuster: &mut Adjuster, items: &[Item], ignored: &mut Vec<String>) {
    let label = format!("adjuster {}", adjuster.key);
    for item in items {
        let field = keyword(item);
        let target = match field.as_deref() {
            Some(b"attack") => &mut adjuster.attack,
            Some(b"defence") => &mut adjuster.defence,
            Some(b"movement") => &mut adjuster.movement,
            _ => {
                ignored.push(format!(
                    "{label} has the unknown entry {}",
                    String::from_utf8_lossy(item.key.as_deref().unwrap_or_default())
                ));
                continue;
            }
        };
        let field = String::from_utf8_lossy(&field.unwrap_or_default()).into_owned();
        let value = scalar(item);
        match crate::fixed5::script_fixed5(value) {
            Some(read) => *target = read,
            None => ignored.push(format!(
                "{label} {field} value {} does not scan as a number; the game keeps the previous value",
                String::from_utf8_lossy(value)
            )),
        }
    }
}

#[cfg(test)]
mod tests;
