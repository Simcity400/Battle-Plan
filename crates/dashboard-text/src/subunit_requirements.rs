//! Canonical sub-unit equipment requirements, from recorded simulation
//! state and the matching `common/units` content.
//!
//! A country-local clone is a separate definition
//! from its global one: it groups apart, and its requirements are the
//! recorded override when `CountryLocalSubunits` names it, else the global
//! authored ones.

#[cfg(test)]
use crate::subunit_files::subunit_keys;
use crate::subunit_files::subunit_paths;
pub use installed_content_equipment_graphics::equipment_graphics::authored_fixed5;
use std::sync::{Arc, OnceLock};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content_core::{
    ResolvedLayers, equipment::Database, equipment::Equipment, parser, parser::Item, parser::Value,
};
use replay_data::{rows::DivisionTemplateIconRow, rows::VariantDefinitionRow, rows::tables};
use replay_frame::frame::DecodedFrame;

use crate::table_access::rows;
use crate::table_access::{TokenKeys, TokenNames};

#[derive(Clone, Debug)]
pub struct SubUnit {
    pub key: String,
    pub sprite: String,
    pub category: u64,
    /// `division_3d_model_priority`, which replaces the equipment score.
    pub model_priority: Option<i32>,
    /// `can_be_parachuted`: a template whose every grouped definition may be parachuted can
    /// drop from the air. An omitted flag is no; a value other than yes or no is refused.
    pub can_be_parachuted: Result<bool, String>,
    /// What the division stat calculator reads of the definition.
    pub sheet: Sheet,
    /// Canonical (archetype, fixed5 amount) needs in native order.
    requirements: Vec<(String, i64)>,
}

impl Default for SubUnit {
    fn default() -> Self {
        Self {
            key: String::new(),
            sprite: String::new(),
            category: 0,
            model_priority: None,
            can_be_parachuted: Ok(false),
            sheet: Sheet::default(),
            requirements: Vec::new(),
        }
    }
}

/// A support company's authored grant to the definitions of one category.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grant {
    pub category: String,
    /// `add = yes` adds to their base words; otherwise each value is a factor.
    pub add: bool,
    /// Every other field as authored, key and scalar text, in order: the stat words, which
    /// their consumer resolves against the executable's stat registry.
    pub words: Vec<(String, String)>,
}

/// A definition's authored operands of the division stat calculator. An authored form the sheet
/// cannot read is refused by name and leaves the sheet unknown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sheet {
    /// Every other scalar field as authored, key and text, in order: the stat words among them,
    /// which their consumer resolves against the executable's stat registry (a later word for
    /// the same stat replaces an earlier one).
    pub words: Vec<(String, String)>,
    pub combat_width: i64,
    pub manpower: i32,
    pub training_time: i32,
    pub group: Option<String>,
    pub categories: Vec<String>,
    /// The critical equipment types (`essential`), as authored, in order.
    pub essential: Vec<String>,
    pub grants: Vec<Grant>,
    pub transport: Option<String>,
    pub own_fuel_consumption: Option<i64>,
    pub refused: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct EquipmentModel<'a> {
    pub archetype: &'a str,
    /// Native archetype registration order is the first production comparator.
    pub archetype_ordinal: usize,
    /// The type itself unless an authored `family`
    /// or a duplicated archetype names another.
    pub family: &'a str,
    pub model_priority: i32,
    pub visual_level: i32,
    pub model: &'a str,
    pub depth: i32,
    /// The type's authored `derived_variant_name` and `variant_name` bytes.
    pub derived_variant_name: &'a [u8],
    pub variant_name: &'a [u8],
}

/// Authored sub-units, and the content's equipment database as
/// `installed_content_core::equipment` loads it, `duplicate_archetypes` copies included.
#[derive(Clone, Debug, Default)]
pub struct Content {
    subunits: BTreeMap<String, SubUnit>,
    /// Sub-unit keys in database order (the game creates each on first
    /// sight: files in path byte order, keys in declaration order), which
    /// The game walks from index 1 (0 is the null object).
    order: Vec<String>,
    pub equipment: Arc<Database>,
    /// The sub-unit keys by the lexer's token identity, built on first use.
    subunit_tokens: OnceLock<TokenKeys>,
    /// The `need` entries the game reports and drops, each with why.
    pub dropped: Vec<String>,
}

impl Content {
    pub fn load(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut content = Self {
            equipment: layers
                .derived::<crate::catalogue::CachedResult<Database>>()
                .get_or_init(|| Database::load_complete(layers))?,
            ..Self::default()
        };
        for path in subunit_paths(layers.paths()) {
            let shown = String::from_utf8_lossy(path);
            let document = parser::parse(&layers.read_path(path)?)
                .map_err(|error| invalid(format!("{shown}: {error}")))?;
            content
                .read_subunits(&document)
                .map_err(|error| invalid(format!("{shown}: {error}")))?;
        }
        content.canonicalize_requirements()?;
        Ok(content)
    }

    fn read_subunits(&mut self, document: &parser::Document) -> io::Result<()> {
        for root in document
            .items
            .iter()
            .filter(|root| root.key.as_deref() == Some(b"sub_units"))
        {
            for item in root.value.items().unwrap_or_default() {
                let (Some(key), Some(fields)) = (item.key.as_deref(), item.value.items()) else {
                    continue;
                };
                let key = text(key)?.to_owned();
                let subunit = read_subunit(&key, fields)?;
                if !self.subunits.contains_key(&key) {
                    self.order.push(key.clone());
                }
                self.subunits.insert(key, subunit);
            }
        }
        Ok(())
    }

    fn canonicalize_requirements(&mut self) -> io::Result<()> {
        // The game stores archetypes, preserving first insertion
        // order while later references to the same archetype replace its value.
        let mut canonical = BTreeMap::new();
        let mut dropped = Vec::new();
        for (key, unit) in &self.subunits {
            let mut requirements = Vec::<(String, i64)>::new();
            for (name, amount) in &unit.requirements {
                // A name with no registered equipment type is reported, and its entry is
                // dropped.
                if self.equipment.named(name).is_none() {
                    dropped.push(format!(
                        "sub-unit {key} needs {name}, which is no equipment"
                    ));
                    continue;
                }
                let archetype = self.archetype(name)?;
                if let Some((_, value)) =
                    requirements.iter_mut().find(|(name, _)| name == archetype)
                {
                    *value = *amount;
                } else {
                    requirements.push((archetype.to_owned(), *amount));
                }
            }
            canonical.insert(key.clone(), requirements);
        }
        self.dropped = dropped;
        for (key, requirements) in canonical {
            self.subunits
                .get_mut(&key)
                .expect("existing authored subunit")
                .requirements = requirements;
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.subunits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.subunits.is_empty()
    }

    /// The existing recorded equipment database, borrowed by native consumers.
    pub fn equipment_content(&self) -> &Database {
        &self.equipment
    }

    /// Sub-units and equipment from documents read in order: the
    /// equipment through the database's load, each document an equipment
    /// file in that order, the sub-units as [`Content::load`] reads them;
    /// for other modules' fixtures.
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_test_documents(documents: &[&[u8]]) -> Self {
        let paths: Vec<_> = (0..documents.len())
            .map(|index| format!("common/units/equipment/{index:03}.txt"))
            .collect();
        let files: Vec<_> = paths
            .iter()
            .zip(documents)
            .map(|(path, text)| (path.as_str(), std::str::from_utf8(text).unwrap()))
            .collect();
        let (equipment, _) = installed_content_core::equipment::fixture::load(&files);
        assert!(equipment.lost.is_none(), "{:?}", equipment.lost);
        let mut content = Self {
            equipment: Arc::new(equipment),
            ..Self::default()
        };
        for document in documents {
            content
                .read_subunits(&parser::parse(document).unwrap())
                .unwrap();
        }
        content.canonicalize_requirements().unwrap();
        content
    }

    /// The archetype's sub-unit list contains every sub-unit whose `need`
    /// specifies a positive amount of it, in database order. Other equipment
    /// types have an empty list.
    pub fn archetype_subunits(&self, key: &str) -> io::Result<Vec<&SubUnit>> {
        if !self.equipment(key)?.is_archetype {
            return Ok(Vec::new());
        }
        Ok(self
            .order
            .iter()
            .filter_map(|name| self.subunits.get(name))
            .filter(|unit| {
                unit.requirements
                    .iter()
                    .any(|(archetype, amount)| archetype == key && *amount > 0)
            })
            .collect())
    }

    /// Model selection: DLC eligibility is tested only for definitions that
    /// actually have copied/authored module slots, using their effective type.
    pub fn has_enabled_module_slots(&self, key: &str, dlc_mask: u64) -> io::Result<bool> {
        let slots = &self.equipment(key)?.module_slots;
        Ok(!slots.is_empty() && self.equipment_category(key)? & dlc_mask != 0)
    }

    /// The finalized model inputs on a type: its
    /// archetype and that archetype's registration order, its family, the
    /// model priority and parent depth, the visual
    /// level and the model.
    pub fn equipment_model(&self, key: &str) -> io::Result<EquipmentModel<'_>> {
        let equipment = self.equipment(key)?;
        let archetype = self.archetype_of(key, equipment)?;
        let model = installed_content_core::parser::text(&equipment.model).map_err(|refused| {
            invalid(format!("equipment {key} model cannot be read: {refused}"))
        })?;
        Ok(EquipmentModel {
            archetype: self.name(archetype)?,
            archetype_ordinal: archetype,
            family: self.name(equipment.family)?,
            model_priority: equipment.model_priority,
            visual_level: equipment.visual_level,
            model,
            depth: equipment.depth,
            derived_variant_name: &equipment.derived_variant_name,
            variant_name: &equipment.variant_name,
        })
    }

    /// The authored equipment type a variant records by token. The engine
    /// builds a type's name and token from one authored key, and the
    /// database finds a name without case, as
    /// the lexer's fold makes one token of names equal under it, so this
    /// hour's spelling (`TokenNames`) names that type; an unspelled token or
    /// none is refused.
    pub fn equipment_key<'a>(
        &'a self,
        tokens: &TokenNames<'_>,
        variant: &VariantDefinitionRow,
    ) -> Result<&'a str, String> {
        let spelling = tokens
            .name(variant.equipment_token)
            .map_err(|reason| format!("variant {} equipment type: {reason}", variant.id))?;
        let at = self.equipment.get(spelling).ok_or_else(|| {
            format!(
                "variant {} equipment type {spelling} is not authored equipment",
                variant.id
            )
        })?;
        self.name(at).map_err(|error| error.to_string())
    }

    /// Each set of sub-unit keys equal under the lexer's fold, which a
    /// recorded token names at once and so refuses.
    pub fn refusals(&self) -> Vec<String> {
        TokenKeys::new(self.subunits.keys())
            .ambiguous()
            .map(|keys| {
                format!(
                    "sub-units {} are one token under the lexer's fold",
                    keys.join(", ")
                )
            })
            .collect()
    }

    /// The authored sub-unit a recorded definition token names, under the
    /// lexer's fold like [`Self::equipment_key`].
    pub fn subunit_of_token(&self, tokens: TokenNames<'_>, token: u32) -> io::Result<&SubUnit> {
        let spelling = tokens
            .name(token)
            .map_err(|reason| invalid(format!("sub-unit: {reason}")))?;
        let key = self
            .subunit_tokens
            .get_or_init(|| TokenKeys::new(self.subunits.keys()))
            .resolve(spelling)
            .map_err(invalid)?
            .ok_or_else(|| invalid(format!("missing authored sub-unit {spelling}")))?;
        Ok(&self.subunits[key])
    }

    pub fn subunit(&self, key: &str) -> Option<&SubUnit> {
        self.subunits.get(key)
    }

    /// The sub-unit a script key names: keys are one token under the lexer's ASCII fold.
    pub fn subunit_named(&self, name: &str) -> Option<&SubUnit> {
        self.subunits
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, subunit)| subunit)
    }

    fn equipment(&self, key: &str) -> io::Result<&Equipment> {
        self.equipment
            .named(key)
            .ok_or_else(|| invalid(format!("missing authored equipment {key}")))
    }

    /// A type's name as the port's text, or why it is not.
    fn name(&self, at: usize) -> io::Result<&str> {
        let equipment = &self.equipment.equipment[at];
        equipment.name_str().map_err(|refused| {
            invalid(format!(
                "equipment {} cannot be named: {refused}",
                String::from_utf8_lossy(&equipment.name)
            ))
        })
    }

    /// The equipment type's archetype; the null object is refused.
    fn archetype_of(&self, key: &str, equipment: &Equipment) -> io::Result<usize> {
        equipment
            .archetype
            .ok_or_else(|| invalid(format!("equipment {key} has no archetype in the content")))
    }

    /// The equipment type's archetype name.
    pub fn archetype(&self, key: &str) -> io::Result<&str> {
        let archetype = self.archetype_of(key, self.equipment(key)?)?;
        self.name(archetype)
    }

    /// The equipment type's effective category mask.
    fn equipment_category(&self, key: &str) -> io::Result<u64> {
        Ok(self.equipment.category(self.equipment(key)?))
    }
}

fn read_subunit(key: &str, fields: &[Item]) -> io::Result<SubUnit> {
    let mut subunit = SubUnit {
        key: key.to_owned(),
        ..SubUnit::default()
    };
    for field in fields {
        match field.key.as_deref() {
            Some(b"sprite") => subunit.sprite = scalar(&field.value)?.to_owned(),
            Some(b"division_3d_model_priority") => {
                subunit.model_priority = Some(integer(&field.value)?);
            }
            Some(b"type") => subunit.category |= category(&field.value)?,
            Some(b"can_be_parachuted") => {
                subunit.can_be_parachuted =
                    yes(&field.value).map_err(|why| format!("sub-unit {key}: {why}"));
            }
            Some(b"combat_width") => {
                subunit.sheet.combat_width = sheet_fixed5(
                    key,
                    "combat_width",
                    &field.value,
                    &mut subunit.sheet.refused,
                );
            }
            Some(b"manpower") => {
                subunit.sheet.manpower =
                    sheet_integer(key, "manpower", &field.value, &mut subunit.sheet.refused);
            }
            Some(b"training_time") => {
                subunit.sheet.training_time = sheet_integer(
                    key,
                    "training_time",
                    &field.value,
                    &mut subunit.sheet.refused,
                );
            }
            Some(b"group") => subunit.sheet.group = field.value.scalar_str().map(str::to_owned),
            Some(b"transport") => {
                subunit.sheet.transport = field.value.scalar_str().map(str::to_owned)
            }
            Some(b"own_equipment_fuel_consumption_mult") => {
                subunit.sheet.own_fuel_consumption = Some(sheet_fixed5(
                    key,
                    "own_equipment_fuel_consumption_mult",
                    &field.value,
                    &mut subunit.sheet.refused,
                ));
            }
            Some(b"categories") => match field.value.items() {
                Some(items) => subunit.sheet.categories.extend(
                    items
                        .iter()
                        .filter_map(|item| item.value.scalar_str())
                        .map(str::to_owned),
                ),
                None => subunit
                    .sheet
                    .refused
                    .push(format!("sub-unit {key}: categories is not a list")),
            },
            Some(b"essential") => match field.value.items() {
                Some(items) => subunit.sheet.essential.extend(
                    items
                        .iter()
                        .filter_map(|item| item.value.scalar_str())
                        .map(str::to_owned),
                ),
                None => subunit
                    .sheet
                    .refused
                    .push(format!("sub-unit {key}: essential is not a list")),
            },
            Some(b"battalion_mult") => {
                let grant = read_grant(key, &field.value, &mut subunit.sheet.refused);
                subunit.sheet.grants.extend(grant);
            }
            Some(b"need") => {
                for need in block(&field.value)? {
                    let name = need
                        .key
                        .as_deref()
                        .ok_or_else(|| invalid("unkeyed subunit equipment requirement"))?;
                    let amount = authored_fixed5(scalar(&need.value)?.as_bytes())?;
                    // Lexical order until references resolve to archetypes.
                    subunit.requirements.push((text(name)?.to_owned(), amount));
                }
            }
            Some(word) => {
                if let Some(value) = field.value.scalar_str() {
                    subunit
                        .sheet
                        .words
                        .push((String::from_utf8_lossy(word).into_owned(), value.to_owned()));
                }
            }
            None => {}
        }
    }
    Ok(subunit)
}

/// An authored fixed5 word of the sheet, or zero with its refusal noted.
fn sheet_fixed5(key: &str, field: &str, value: &Value, refused: &mut Vec<String>) -> i64 {
    match value
        .scalar_str()
        .map(|text| authored_fixed5(text.as_bytes()))
    {
        Some(Ok(value)) => value,
        _ => {
            refused.push(format!("sub-unit {key}: {field} is not a number"));
            0
        }
    }
}

/// An authored integer of the sheet, or zero with its refusal noted.
fn sheet_integer(key: &str, field: &str, value: &Value, refused: &mut Vec<String>) -> i32 {
    match value.scalar_str().map(str::parse::<i32>) {
        Some(Ok(value)) => value,
        _ => {
            refused.push(format!("sub-unit {key}: {field} is not a whole number"));
            0
        }
    }
}

/// One `battalion_mult` block: its category, its kind and its stat values.
fn read_grant(key: &str, value: &Value, refused: &mut Vec<String>) -> Option<Grant> {
    let Some(items) = value.items() else {
        refused.push(format!("sub-unit {key}: battalion_mult is not a block"));
        return None;
    };
    let mut grant = Grant::default();
    for item in items {
        match item.key.as_deref() {
            Some(b"category") => match item.value.scalar_str() {
                Some(category) => grant.category = category.to_owned(),
                None => refused.push(format!(
                    "sub-unit {key}: battalion_mult category is not a name"
                )),
            },
            Some(b"add") => match yes(&item.value) {
                Ok(add) => grant.add = add,
                Err(why) => refused.push(format!("sub-unit {key}: battalion_mult {why}")),
            },
            Some(word) => match item.value.scalar_str() {
                Some(value) => grant
                    .words
                    .push((String::from_utf8_lossy(word).into_owned(), value.to_owned())),
                None => refused.push(format!(
                    "sub-unit {key}: battalion_mult field {} is not a scalar",
                    String::from_utf8_lossy(word)
                )),
            },
            None => refused.push(format!(
                "sub-unit {key}: battalion_mult has an unkeyed field"
            )),
        }
    }
    if grant.category.is_empty() {
        refused.push(format!("sub-unit {key}: battalion_mult names no category"));
        return None;
    }
    Some(grant)
}

// ---------------------------------------------------------- the recorded template

/// Native sub-unit definition identity: a local clone and the global
/// definition share a token but are separate definitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Definition {
    Global(u32),
    Local { country: u32, token: u32 },
}

/// The hour's tables a template's definitions resolve through.
pub struct Sources<'a> {
    pub tokens: TokenNames<'a>,
    links: BTreeMap<u32, u32>,
    /// (canonical country, token) -> the override's (archetype token, amount).
    local: BTreeMap<(u32, u32), Vec<(u32, i64)>>,
    local_complete: bool,
}

impl<'a> Sources<'a> {
    pub fn read(frame: &'a DecodedFrame) -> io::Result<Self> {
        let tokens = TokenNames::new(frame)?;
        let links: BTreeMap<u32, u32> = frame
            .complete::<tables::CountryLinks>()?
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        let local_rows = rows::<tables::CountryLocalSubunits>(frame)?;
        let mut local = BTreeMap::new();
        for row in local_rows.rows() {
            let country = *links.get(&row.country).ok_or_else(|| {
                invalid(format!(
                    "local sub-unit country {} has no CountryLinks row",
                    row.country
                ))
            })?;
            let requirements = row
                .requirements
                .iter()
                .map(|entry| (entry.archetype_token, entry.amount_raw))
                .collect();
            if local.insert((country, row.token), requirements).is_some() {
                return Err(invalid(format!(
                    "duplicate local sub-unit {country}:{}",
                    row.token
                )));
            }
        }
        Ok(Self {
            tokens,
            links,
            local,
            local_complete: local_rows.is_complete(),
        })
    }

    fn identity(
        &self,
        template: &DivisionTemplateIconRow,
        token: u32,
        local: u8,
    ) -> io::Result<Definition> {
        if local == 0 {
            return Ok(Definition::Global(token));
        }
        let country = u32::try_from(template.country)
            .ok()
            .and_then(|raw| self.links.get(&raw))
            .ok_or_else(|| {
                invalid(format!(
                    "template {} country {} names no country for its local sub-units",
                    template.template, template.country
                ))
            })?;
        Ok(Definition::Local {
            country: *country,
            token,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Group<'a> {
    pub identity: Definition,
    pub subunit: &'a SubUnit,
    pub count: i32,
    pub requirements: Cow<'a, [(String, i64)]>,
}

/// A recorded template resolved against content.
#[derive(Debug)]
pub struct Template<'a> {
    /// the game's representative; `None` is the null object.
    pub representative: Option<Group<'a>>,
    /// The stored grouped entries, null definitions dropped.
    pub groups: Vec<Group<'a>>,
}

impl Template<'_> {
    /// Native consumers read the literal first ordered requirement, including
    /// a zero or negative amount; they do not select the largest requirement.
    pub fn first_requirement<'g>(group: &'g Group<'_>) -> Option<&'g str> {
        group.requirements.first().map(|(key, _)| key.as_str())
    }
}

impl Content {
    pub fn requirements<'a>(
        &'a self,
        identity: Definition,
        subunit: &'a SubUnit,
        sources: &Sources<'_>,
    ) -> io::Result<Cow<'a, [(String, i64)]>> {
        let Definition::Local { country, token } = identity else {
            return Ok(Cow::Borrowed(&subunit.requirements));
        };
        let Some(entries) = sources.local.get(&(country, token)) else {
            // A complete table names every clone that differs.
            return if sources.local_complete {
                Ok(Cow::Borrowed(&subunit.requirements))
            } else {
                Err(invalid(format!(
                    "local sub-unit {country}:{token} requirements are unavailable in an incomplete census"
                )))
            };
        };
        let mut requirements = Vec::with_capacity(entries.len());
        let mut seen = BTreeSet::new();
        for &(archetype_token, amount_raw) in entries {
            // The requirement names an equipment type by token, under the
            // lexer's fold like a variant's.
            let spelling = sources.tokens.name(archetype_token).map_err(|reason| {
                invalid(format!(
                    "local sub-unit {country}:{token} requirement: {reason}"
                ))
            })?;
            let key = self.equipment.get(spelling).ok_or_else(|| {
                invalid(format!(
                    "local sub-unit {country}:{token} requirement {spelling} is not authored equipment"
                ))
            })?;
            let archetype = self.archetype(self.name(key)?)?;
            if !seen.insert(archetype) {
                return Err(invalid(format!(
                    "local sub-unit {country}:{token} repeats archetype {archetype}"
                )));
            }
            requirements.push((archetype.to_owned(), amount_raw));
        }
        Ok(Cow::Owned(requirements))
    }

    pub fn group<'a>(
        &'a self,
        template: &DivisionTemplateIconRow,
        sources: &Sources<'_>,
        token: u32,
        local: u8,
        count: i32,
    ) -> io::Result<Group<'a>> {
        let subunit = self.subunit_of_token(sources.tokens, token)?;
        let identity = sources.identity(template, token, local)?;
        Ok(Group {
            identity,
            subunit,
            count,
            requirements: self.requirements(identity, subunit, sources)?,
        })
    }

    pub fn template<'a>(
        &'a self,
        template: &DivisionTemplateIconRow,
        sources: &Sources<'_>,
    ) -> io::Result<Template<'a>> {
        let representative = (template.representative != 0)
            .then(|| {
                self.group(
                    template,
                    sources,
                    template.representative,
                    template.representative_local,
                    0,
                )
            })
            .transpose()?;
        let groups = template
            .subunits
            .iter()
            .filter(|unit| unit.token != 0)
            .map(|unit| self.group(template, sources, unit.token, unit.local, unit.count))
            .collect::<io::Result<_>>()?;
        Ok(Template {
            representative,
            groups,
        })
    }
}

pub fn text(bytes: &[u8]) -> io::Result<&str> {
    std::str::from_utf8(bytes).map_err(|_| invalid("non-UTF8 authored model key"))
}
fn scalar(value: &Value) -> io::Result<&str> {
    value
        .scalar_str()
        .ok_or_else(|| invalid("model content field is not a UTF8 scalar"))
}
fn integer(value: &Value) -> io::Result<i32> {
    scalar(value)?
        .parse()
        .map_err(|_| invalid("model content field is not an i32"))
}
fn yes(value: &Value) -> Result<bool, String> {
    match value.scalar_str() {
        Some("yes") => Ok(true),
        Some("no") => Ok(false),
        other => Err(format!("a flag is {other:?}, not yes or no")),
    }
}
fn block(value: &Value) -> io::Result<&[Item]> {
    value
        .items()
        .ok_or_else(|| invalid("model content field is not a block"))
}
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// A sub-unit `type` ORs all named category flags. Unknown categories are refused.
fn category(value: &Value) -> io::Result<u64> {
    if let Some(items) = value.items() {
        return items
            .iter()
            .try_fold(0, |mask, item| Ok(mask | category(&item.value)?));
    }
    let name = scalar(value)?;
    installed_content_core::equipment::category_mask(name)
        .ok_or_else(|| invalid(format!("unknown equipment category {name}")))
}

#[cfg(test)]
mod tests;
