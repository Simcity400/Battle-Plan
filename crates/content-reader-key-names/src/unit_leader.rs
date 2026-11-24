use crate::key_name_products::{KeyNameProducts, KeyNamesExt as _};

use std::{collections::BTreeMap, io, sync::Arc};

use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_parser::{crt::{self, Scanned}, parser::{Item, Value}};
use content_reader_script::{modifiers::{self, Modifiers}, native::{self, Field, Kind, Shape, Token}, scope::ScopeKind};

mod division_experience;
pub mod needs;
mod skills;
mod sub_units;

pub use division_experience::{DivisionExperience, DivisionExperienceProgress};
pub use skills::{Component, Skills};
pub use sub_units::{AirWings, SubUnit, SubUnits, sub_unit_files, sub_units};

use content_reader_script::sub_units::identity;

const DIRECTORY: &str = "common/unit_leader/";

#[derive(Clone, Debug)]
pub struct Leaders<N = String> {
    pub traits: Traits,
    pub skills: Skills,
    pub misreads: Vec<N>,
    files: Vec<(String, content_reader_layers::content::Bytes)>,
    names: Arc<KeyNameProducts>,
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

pub use content_reader_script::modifiers::Registry;

impl<N: content_reader_parser::DiagnosticMessage> Leaders<N> {
    pub fn load(
        layers: &ResolvedLayers,
        terrain: &dyn Fn(&str) -> Option<String>,
    ) -> io::Result<Self> {
        let names = layers.key_names()?;
        Ok(Self::read_files(files(layers)?, terrain, &|_| None, names))
    }

    pub fn reread(&self, terrain: &dyn Fn(&str) -> Option<String>, registry: Registry<'_>) -> Self {
        Self::read_files(self.files.clone(), terrain, registry, self.names.clone())
    }

    pub fn conditional(&self) -> bool {
        !self.skills.conditions.is_empty()
            || !self.skills.component_conditions.is_empty()
            || self
                .traits
                .iter()
                .any(|effects| !effects.conditions.is_empty())
    }

    fn read_files<B: Into<content_reader_layers::content::Bytes>>(
        files: Vec<(String, B)>,
        terrain: &dyn Fn(&str) -> Option<String>,
        registry: Registry<'_>,
        names: Arc<KeyNameProducts>,
    ) -> Self {
        let files: Vec<(String, content_reader_layers::content::Bytes)> = files.into_iter()
            .map(|(path, bytes)| (path, bytes.into())).collect();
        let mut leaders = Self {
            names,
            ..Self::default()
        };
        for (path, bytes) in &files {
            leaders.traits.register(bytes, path);
        }
        let mut adjusters = BTreeMap::new();
        let mut gaps = Vec::new();
        for (path, bytes) in &files {
            gaps.extend(leaders.read(bytes, path, terrain, registry, &mut adjusters));
        }
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
    pub fn from_files(
        files: Vec<(String, Vec<u8>)>,
        terrain: &dyn Fn(&str) -> Option<String>,
    ) -> Self {
        Self::read_files(files, terrain, &|_| None, Arc::default())
    }
}

fn files(layers: &ResolvedLayers) -> io::Result<Vec<(String, content_reader_layers::content::Bytes)>> {
    script_files::paths(layers, DIRECTORY, Some(".txt"))?
        .into_iter()
        .map(|path| {
            let bytes = path.read(layers)?;
            Ok((path.to_string(), bytes))
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Top,
    SkillTable,
    Level,
    TraitTable,
    Trait(usize),
    TraitModifier(usize),
    Modifier(modifiers::Node),
    Adjuster,
    SubUnitTable,
    SubUnit,
    Experience,
    Parent,
    Mtth,
    UnitTypes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Current {
    Top,
    Skills,
    Component,
    Trait(usize),
}

struct Schema<'a> {
    path: &'a str,
    traits: &'a Traits,
    terrain: &'a dyn Fn(&str) -> Option<String>,
    registry: Registry<'a>,
    bodies: Vec<usize>,
    adjusters: &'a mut BTreeMap<usize, Vec<String>>,
    pending: Vec<Conditional>,
    conditions: BTreeMap<usize, Vec<Conditional>>,
    skills_conditions: Vec<Conditional>,
    component_conditions: Vec<Conditional>,
    current: Current,
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
                    Some(key) if Component::from_key(key).is_some() => {
                        self.current = Current::Component;
                        self.component_conditions
                            .extend(self.pending.iter().cloned());
                        Shape::Object(Node::SkillTable)
                    }
                    _ => Shape::Skip,
                }
            }
            Node::SkillTable if content_reader_parser::fixed5::decimal_i32(&field.key.text) > 0 => {
                Shape::Object(Node::Level)
            }
            Node::SkillTable => Shape::Skip,
            Node::Level => match keyword {
                Some(b"modifier" | b"building_module_modifier") => {
                    Shape::Object(Node::Modifier(modifiers::Node::Modifier))
                }
                Some(b"cost") => Shape::One,
                Some(b"type") if leader_category(field.value_keyword().as_deref()).is_some() => {
                    Shape::One
                }
                Some(b"type") => Shape::Skip,
                _ => Shape::Skip,
            },
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
                            (true, true) => Shape::Object(Node::Adjuster),
                            (true, false) => Shape::One,
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
            Node::Experience => match keyword.and(self.traits.index(&field.key.text)) {
                Some(_) => Shape::One,
                None => Shape::Skip,
            },
            Node::Parent => match keyword {
                Some(b"traits") => Shape::List,
                _ => Shape::One,
            },
            Node::Mtth => native::mean_time_to_happen(field),
            Node::UnitTypes => Shape::One,
        }
    }

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
        Some(b"type") => Shape::ListIfOpen,
        Some(b"any_parent" | b"all_parents" | b"allowed_ship_equipments") => Shape::List,
        Some(b"ai_will_do" | b"new_commander_weight") => Shape::Object(Node::Mtth),
        Some(
            b"allowed" | b"prerequisites" | b"gain_xp" | b"gain_xp_leader" | b"unit_trigger"
            | b"country_trigger",
        ) => Shape::Trigger,
        Some(b"unit_type") => Shape::Object(Node::UnitTypes),
        Some(b"on_add" | b"on_remove" | b"daily_effect") => {
            Shape::EffectIn(ScopeKind::Character.bit())
        }
        Some(b"num_parents_needed") => Shape::Skip,
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
        _ => Shape::Skip,
    }
}

#[cfg(test)]
mod trigger_reader_tests;

fn leader_category(keyword: Option<&[u8]>) -> Option<i32> {
    match keyword? {
        b"field_marshal" => Some(0),
        b"corps_commander" => Some(1),
        b"navy" => Some(2),
        b"operative" => Some(3),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conditional {
    pub modifier: String,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraitReference {
    Name(String),
    Null(String),
}

impl TraitReference {
    fn read(current: &Token) -> Result<Self, String> {
        let shown = String::from_utf8_lossy(content_reader_parser::parser::c_string(&current.text)).into_owned();
        if current.kind != Kind::Bare
            || !content_reader_parser::parser::looked_up(&current.text, false)
            || current.text.first() == Some(&b'@')
        {
            return Ok(Self::Null(shown));
        }
        std::str::from_utf8(&current.text)
            .map(|name| Self::Name(name.to_owned()))
            .map_err(|_| format!("advisor trait {shown} is a name that is not UTF-8"))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Adjuster {
    pub key: String,
    pub attack: i64,
    pub defence: i64,
    pub movement: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubUnitModifiers {
    pub adjuster: Adjuster,
    pub block: Modifiers,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraitEffects {
    pub name: String,
    pub army_only: bool,
    pub override_tooltips: Vec<String>,
    pub custom_tooltips: Vec<String>,
    pub blocks: [Modifiers; 4],
    pub adjusters: Vec<Adjuster>,
    pub sub_units: Vec<SubUnitModifiers>,
    pub skills: [i32; 6],
    pub proximity_offset: i32,
    pub required_experience: Option<i32>,
    pub experience_factors: Vec<(String, i64)>,
    pub advisor_slot: Option<String>,
    pub advisor_traits: [Option<TraitReference>; 3],
    pub conditions: Vec<Conditional>,
    pub unsupported: Vec<String>,
    pub ignored: Vec<String>,
}

const DEFAULT_TRAIT_EXPERIENCE_COST: i32 = 1000;

impl TraitEffects {
    pub fn required_experience(&self) -> i32 {
        self.required_experience
            .unwrap_or(DEFAULT_TRAIT_EXPERIENCE_COST)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Traits {
    traits: BTreeMap<Vec<u8>, TraitEffects>,
    order: Vec<Vec<u8>>,
    names: BTreeMap<Vec<u8>, Result<String, String>>,
    definitions: Vec<Vec<u8>>,
    pub unsupported: Vec<String>,
}

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

const DEFAULT_TOKEN_KEY: &str = "none";

fn unknown(key: &Item) -> String {
    format!(
        "unknown trait key {}",
        String::from_utf8_lossy(key.key.as_deref().unwrap_or_default())
    )
}

impl Traits {
    pub fn spelling(&self, token: &str) -> Result<Option<&str>, &str> {
        self.names
            .get(&identity(content_reader_parser::parser::c_string(token.as_bytes())))
            .map(|name| name.as_deref().map_err(String::as_str))
            .transpose()
    }

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

    pub fn assignment_sprite(&self, token: &str) -> Result<Option<String>, &str> {
        Ok(self
            .spelling(token)?
            .map(|name| format!("GFX_trait_{name}")))
    }

    pub fn get(&self, name: &str) -> Option<&TraitEffects> {
        self.traits.get(&identity(name.as_bytes()))
    }

    pub fn iter(&self) -> impl Iterator<Item = &TraitEffects> {
        self.traits.values()
    }

    pub fn registered(layers: &ResolvedLayers) -> io::Result<Self> {
        layers.recorded_product("registered unit leader traits", 1, &[], |layers| {
            let mut traits = Self::default();
            for (path, bytes) in files(layers)? {
                traits.register(&bytes, &path);
            }
            Ok(traits)
        }).map(|traits| traits.as_ref().clone())
    }

    pub fn contains(&self, name: &[u8]) -> bool {
        self.names.contains_key(&identity(name))
    }

    pub fn first_ordinal(&self, name: &[u8]) -> Result<Option<i32>, String> {
        let folded = content_reader_parser::parser::c_string(name).to_ascii_lowercase();
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

    pub fn spellings(&self) -> impl Iterator<Item = &str> {
        self.order
            .iter()
            .filter_map(|identity| self.names.get(identity)?.as_deref().ok())
    }

    fn index(&self, name: &[u8]) -> Option<usize> {
        let identity = identity(name);
        self.order.iter().position(|known| *known == identity)
    }

    fn register(&mut self, bytes: &[u8], path: &str) {
        #[derive(Clone, Copy)]
        enum Pass {
            Top,
            Table,
        }
        let mut names = Vec::new();
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
        self.definitions.extend(
            names
                .iter()
                .map(|name| content_reader_parser::parser::c_string(name).to_ascii_lowercase()),
        );
        for name in names {
            let identity = identity(&name);
            let spelling = content_reader_parser::parser::text(&name).map(str::to_owned).map_err(|why| {
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

fn c_string<'a>(name: &str, item: &'a Item) -> Result<&'a str, String> {
    item.value.c_str().map_err(|why| match why {
        content_reader_parser::parser::NotText::NotUtf8(_) => format!(
            "{name} value {} is not UTF-8",
            String::from_utf8_lossy(content_reader_parser::parser::c_string(scalar(item)))
        ),
        content_reader_parser::parser::NotText::Block => format!("{name} value: {why}"),
    })
}

fn scalar(item: &Item) -> &[u8] {
    item.value.scalar().unwrap_or_default()
}

fn bare_key(item: &Item) -> Option<&[u8]> {
    let key = item.key.as_deref()?;
    let punctuation = matches!(
        key,
        b"=" | b"<" | b">" | b"{" | b"}" | b"(" | b")" | b"," | b"?="
    );
    (!item.key_quoted && !punctuation && item.keyword().is_some()).then_some(key)
}

fn keyword(item: &Item) -> Option<Vec<u8>> {
    item.keyword().map(|keyword| keyword.folded())
}

impl TraitEffects {
    fn read_into(
        &mut self,
        fields: &[Item],
        terrain: &dyn Fn(&str) -> Option<String>,
        traits: &BTreeMap<Vec<u8>, Result<String, String>>,
    ) {
        for field in fields {
            let name = keyword(field);
            let Some(name) = name.as_deref() else {
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
                    let categories = field.value.items().unwrap_or(std::slice::from_ref(field));
                    let mut mask = None;
                    for category in categories {
                        let bits = match category.value_keyword().as_deref() {
                            Some("field_marshal") => Some(1),
                            Some("corps_commander") => Some(2),
                            Some("navy") => Some(4),
                            Some("operative") => Some(8),
                            Some("land") => Some(3),
                            Some("all") => Some(15),
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
                        let value = scalar(item);
                        let read = content_reader_parser::fixed5::script_fixed5(value).unwrap_or_else(|| {
                            self.ignored.push(format!(
                                "trait_xp_factor {key} value {} does not scan; the game reads zero",
                                String::from_utf8_lossy(value)
                            ));
                            0
                        });
                        self.experience_factors.push((named, read));
                    }
                }
                b"slot" => match c_string(&name_text, field) {
                    Ok(slot) => self.advisor_slot = Some(slot.to_owned()).filter(|s| !s.is_empty()),
                    Err(reason) => self.unsupported.push(reason),
                },
                name if trait_field(0, Some(name)) == Shape::Skip => {
                    self.ignored.push(unknown(field))
                }
                _ => {}
            }
        }
    }

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
        if let Some(cause) = self.blocks[block].unheld.cause() {
            let reason = format!("{name}: {cause}");
            if !self.unsupported.contains(&reason) {
                self.unsupported.push(reason);
            }
        }
    }

    fn note_modifier(&mut self, name: &str, item: &Item) {
        let entry = String::from_utf8_lossy(item.key.as_deref().unwrap_or_default()).into_owned();
        match keyword(item).as_deref() {
            Some(b"name" | b"data" | b"added_modifier" | b"custom_modifier_tooltip") => {}
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
                self.unsupported
                    .push(format!("{label} has a key that is not UTF-8"));
            }
            self.sub_units.push(entry);
        }
    }
}

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
        match content_reader_parser::fixed5::script_fixed5(value) {
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
