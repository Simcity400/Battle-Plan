//! Canonical sub-unit equipment requirements, from recorded simulation
//! state and the matching sub-unit database content.
//!
//! A country-local clone is a separate definition
//! from its global one: it groups apart, and its requirements are the
//! recorded override when `CountryLocalSubunits` names it, else the global
//! authored ones.

#[cfg(test)]
use crate::subunit_files::subunit_keys;
pub use installed_content_equipment_graphics::equipment_graphics::authored_fixed5;
use std::sync::Arc;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content_core::{
    ResolvedLayers, equipment::Database, equipment::Equipment, parser, parser::Item, parser::Value,
    unit_leader::{SubUnits, needs},
};
use replay_data::{rows::DivisionTemplateIconRow, rows::VariantDefinitionRow, rows::tables};
use replay_data::rows::TableId;
use replay_frame::frame::{DecodedFrame, HourFrame};

use crate::table_access::rows;
use crate::table_access::{TokenNames, fold_token};

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
    pub refused: Vec<Refusal>,
}

/// A field of the sheet that cannot be read, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub field: String,
    pub why: String,
}

fn refuse(refused: &mut Vec<Refusal>, field: &str, why: String) {
    refused.push(Refusal { field: field.to_owned(), why });
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

// Loaded once a run, and handed through a preparation's spool as it is.
impl replay_spool::Spooled for Content {
    const RESIDENT: bool = true;

    fn spool(&self, _: &mut replay_spool::Writer<'_>) -> std::io::Result<()> {
        Err(replay_spool::invalid("Content is resident, never written"))
    }

    fn unspool(_: &mut replay_spool::Reader<'_>) -> std::io::Result<Self> {
        Err(replay_spool::invalid("Content is resident, never read"))
    }
}

/// Authored sub-units, and the content's equipment database as
/// `installed_content_core::equipment` loads it, `duplicate_archetypes` copies included.
#[derive(Clone, Debug, Default)]
pub struct Content {
    /// Each sub-unit by its key under the lexer's ASCII fold, the database's identity: a key
    /// that differs only in case declares the same one again, under its first spelling.
    subunits: BTreeMap<String, SubUnit>,
    /// Sub-unit identities in database order (the game creates each on first
    /// sight: the database's files in its order, keys in declaration order), which
    /// The game walks from index 1 (0 is the null object).
    order: Vec<String>,
    pub equipment: Arc<Database>,
    /// The `need` entries the game reports and drops, each with why.
    pub dropped: Vec<String>,
}

impl Content {
    /// The sub-unit database's files in its order; type masks and groups are the database's own.
    pub fn load(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut content = Self {
            equipment: layers
                .derived::<crate::catalogue::CachedResult<Database>>()
                .get_or_init(|| Database::load_complete(layers))?,
            ..Self::default()
        };
        for path in installed_content_core::unit_leader::sub_unit_files(layers)? {
            let shown = path.text();
            let document = parser::parse(&path.read(layers)?)
                .map_err(|error| invalid(format!("{shown}: {error}")))?;
            content
                .read_subunits(&document)
                .map_err(|error| invalid(format!("{shown}: {error}")))?;
        }
        let units = layers
            .derived::<crate::catalogue::CachedResult<SubUnits>>()
            .get_or_init(|| installed_content_core::unit_leader::sub_units(layers))?;
        content.take_database(&units)?;
        content.canonicalize_requirements()?;
        Ok(content)
    }

    fn read_subunits(&mut self, document: &parser::Document) -> io::Result<()> {
        for root in document
            .items
            .iter()
            .filter(|root| root.keyword_is("sub_units"))
        {
            for item in root.value.items().unwrap_or_default() {
                let (Some(key), Some(fields)) = (item.key.as_deref(), item.value.items()) else {
                    continue;
                };
                let spelling = text(key)?;
                let identity = fold_token(spelling);
                let previous = self.subunits.get(&identity);
                let key = previous.map_or(spelling, |previous| previous.key.as_str());
                let subunit = read_subunit(key, previous, fields)?;
                if previous.is_none() {
                    self.order.push(identity.clone());
                }
                self.subunits.insert(identity, subunit);
            }
        }
        Ok(())
    }

    /// The database's type masks (by lexer token kind) and groups (kept by a redefinition).
    fn take_database(&mut self, units: &SubUnits) -> io::Result<()> {
        for subunit in self.subunits.values_mut() {
            let key = &subunit.key;
            subunit.category = units
                .category(key.as_bytes())
                .map_err(|why| invalid(format!("sub-unit {key} type: {why}")))?;
            match units.group(subunit.key.as_bytes()) {
                Ok(group) => subunit.sheet.group = group.map(str::to_owned),
                Err(why) => refuse(&mut subunit.sheet.refused, "group", format!("sub-unit {key} group: {why}")),
            }
        }
        Ok(())
    }

    fn canonicalize_requirements(&mut self) -> io::Result<()> {
        let mut dropped = Vec::new();
        let equipment = Arc::clone(&self.equipment);
        let archetype = |name: &str| needs::archetype(&equipment, name);
        for unit in self.subunits.values_mut() {
            unit.requirements = needs::canonical(&unit.key, &unit.requirements, &archetype, &mut dropped)?;
        }
        self.dropped = dropped;
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
        let mut units = SubUnits::default();
        for (document, path) in documents.iter().zip(&paths) {
            content
                .read_subunits(&parser::parse(document).unwrap())
                .unwrap();
            units.read(document, path);
        }
        content.take_database(&units).unwrap();
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

    /// The authored sub-unit a recorded definition token names, under the
    /// lexer's fold like [`Self::equipment_key`].
    pub fn subunit_of_token(&self, tokens: TokenNames<'_>, token: u32) -> io::Result<&SubUnit> {
        let spelling = tokens
            .name(token)
            .map_err(|reason| invalid(format!("sub-unit: {reason}")))?;
        self.subunit(spelling)
            .ok_or_else(|| invalid(format!("missing authored sub-unit {spelling}")))
    }

    /// The sub-unit a key or recorded spelling names: keys are one token under the lexer's
    /// ASCII fold.
    pub fn subunit(&self, key: &str) -> Option<&SubUnit> {
        self.subunits.get(&fold_token(key))
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

/// Scalars the game's reset leaves as an earlier declaration set them: kept until the new
/// declaration assigns them again.
const KEPT_SCALARS: [&str; 6] = [
    "affects_speed",
    "can_exfiltrate_from_coast",
    "hit_profile_mult",
    "critical_part_damage_chance_mult",
    "abbreviation",
    "own_equipment_fuel_consumption_mult",
];

/// Lists the reset leaves and a redefinition extends: their entries and refusals all stay.
const KEPT_LISTS: [&str; 7] = [
    "categories",
    "essential",
    "battalion_mult",
    "same_support_type",
    "critical_parts",
    "required_dlc",
    "allowed_battalion_groups",
];

fn kept(names: &[&str], field: &str) -> bool {
    names.iter().any(|name| field.eq_ignore_ascii_case(name))
}

/// One declaration replayed onto `previous`, as the game's reset leaves it: model priority,
/// transport, the own-fuel multiplier and the kept words stay; categories, essential and
/// battalion multipliers are extended; everything else starts again.
fn read_subunit(key: &str, previous: Option<&SubUnit>, fields: &[Item]) -> io::Result<SubUnit> {
    let mut subunit = SubUnit {
        key: key.to_owned(),
        ..SubUnit::default()
    };
    if let Some(previous) = previous {
        subunit.model_priority = previous.model_priority;
        subunit.sheet.categories.clone_from(&previous.sheet.categories);
        subunit.sheet.essential.clone_from(&previous.sheet.essential);
        subunit.sheet.grants.clone_from(&previous.sheet.grants);
        subunit.sheet.transport.clone_from(&previous.sheet.transport);
        subunit.sheet.own_fuel_consumption = previous.sheet.own_fuel_consumption;
    }
    // What the earlier declaration left of the kept scalars and lists; a scalar the new
    // declaration assigns replaces its earlier word and refusal.
    let (carried_words, carried_refused) = previous.map_or_else(Default::default, |previous| {
        let words: Vec<_> = previous.sheet.words.iter()
            .filter(|(word, _)| kept(&KEPT_SCALARS, word) || kept(&KEPT_LISTS, word)).cloned().collect();
        let refused: Vec<_> = previous.sheet.refused.iter()
            .filter(|refusal| kept(&KEPT_SCALARS, &refusal.field) || kept(&KEPT_LISTS, &refusal.field)).cloned().collect();
        (words, refused)
    });
    let mut assigned = Vec::new();
    for field in fields {
        let Some(keyword) = field.keyword() else {
            continue;
        };
        let keyword = keyword.folded();
        match keyword.as_slice() {
            b"sprite" => subunit.sprite = scalar(&field.value)?.to_owned(),
            b"division_3d_model_priority" => {
                subunit.model_priority = Some(integer(&field.value)?);
            }
            b"type" | b"group" => {}
            b"can_be_parachuted" => {
                subunit.can_be_parachuted =
                    yes(&field.value).map_err(|why| format!("sub-unit {key}: {why}"));
            }
            b"combat_width" => {
                subunit.sheet.combat_width = sheet_fixed5(
                    key,
                    "combat_width",
                    &field.value,
                    &mut subunit.sheet.refused,
                );
            }
            b"manpower" => {
                subunit.sheet.manpower =
                    sheet_integer(key, "manpower", &field.value, &mut subunit.sheet.refused);
            }
            b"training_time" => {
                subunit.sheet.training_time = sheet_integer(
                    key,
                    "training_time",
                    &field.value,
                    &mut subunit.sheet.refused,
                );
            }
            b"transport" => {
                subunit.sheet.transport = field.value.scalar_str().map(str::to_owned)
            }
            b"own_equipment_fuel_consumption_mult" => {
                assigned.push("own_equipment_fuel_consumption_mult".to_owned());
                subunit.sheet.own_fuel_consumption = Some(sheet_fixed5(
                    key,
                    "own_equipment_fuel_consumption_mult",
                    &field.value,
                    &mut subunit.sheet.refused,
                ));
            }
            b"categories" => extend_tokens(
                key,
                "categories",
                field,
                &mut subunit.sheet.categories,
                &mut subunit.sheet.refused,
            ),
            b"essential" => extend_tokens(
                key,
                "essential",
                field,
                &mut subunit.sheet.essential,
                &mut subunit.sheet.refused,
            ),
            b"battalion_mult" => {
                let grant = read_grant(key, &field.value, &mut subunit.sheet.refused);
                subunit.sheet.grants.extend(grant);
            }
            // Lexical order until references resolve to archetypes.
            b"need" => subunit.requirements.extend(needs::entries(&field.value)?),
            word => {
                if let Some(value) = field.value.scalar_str() {
                    let word = String::from_utf8_lossy(word).into_owned();
                    if kept(&KEPT_SCALARS, &word) {
                        assigned.push(word.clone());
                    }
                    subunit.sheet.words.push((word, value.to_owned()));
                }
            }
        }
    }
    let replaced = |field: &str| kept(&KEPT_SCALARS, field) && assigned.iter().any(|name| field.eq_ignore_ascii_case(name));
    let mut words: Vec<_> = carried_words.into_iter().filter(|(word, _)| !replaced(word)).collect();
    words.append(&mut subunit.sheet.words);
    subunit.sheet.words = words;
    let mut refused: Vec<_> = carried_refused.into_iter().filter(|refusal| !replaced(&refusal.field)).collect();
    refused.append(&mut subunit.sheet.refused);
    subunit.sheet.refused = refused;
    Ok(subunit)
}

/// Names resolved by token: only bare entries count, and one already held (without case) is skipped.
fn extend_tokens(
    key: &str,
    field_name: &str,
    field: &Item,
    names: &mut Vec<String>,
    refused: &mut Vec<Refusal>,
) {
    let Some(items) = field.value.items() else {
        refuse(refused, field_name, format!("sub-unit {key}: {field_name} is not a list"));
        return;
    };
    for item in items {
        if item.value_keyword().is_none() {
            continue;
        }
        let Some(name) = item.value.scalar_str() else {
            refuse(refused, field_name, format!("sub-unit {key}: a {field_name} entry is not UTF-8"));
            continue;
        };
        if !names.iter().any(|held| held.eq_ignore_ascii_case(name)) {
            names.push(name.to_owned());
        }
    }
}

/// An authored fixed5 word of the sheet, or zero with its refusal noted.
fn sheet_fixed5(key: &str, field: &str, value: &Value, refused: &mut Vec<Refusal>) -> i64 {
    match value
        .scalar_str()
        .map(|text| authored_fixed5(text.as_bytes()))
    {
        Some(Ok(value)) => value,
        _ => {
            refuse(refused, field, format!("sub-unit {key}: {field} is not a number"));
            0
        }
    }
}

/// An authored integer of the sheet, or zero with its refusal noted.
fn sheet_integer(key: &str, field: &str, value: &Value, refused: &mut Vec<Refusal>) -> i32 {
    match value.scalar_str().map(str::parse::<i32>) {
        Some(Ok(value)) => value,
        _ => {
            refuse(refused, field, format!("sub-unit {key}: {field} is not a whole number"));
            0
        }
    }
}

/// One `battalion_mult` block: its category, its kind and its stat values.
fn read_grant(key: &str, value: &Value, refused: &mut Vec<Refusal>) -> Option<Grant> {
    let Some(items) = value.items() else {
        refuse(refused, "battalion_mult", format!("sub-unit {key}: battalion_mult is not a block"));
        return None;
    };
    let mut grant = Grant::default();
    for item in items {
        let Some(keyword) = item.keyword() else {
            refuse(refused, "battalion_mult", format!(
                "sub-unit {key}: battalion_mult has a field that is no keyword"
            ));
            continue;
        };
        match keyword.folded().as_slice() {
            b"category" => match item.value.scalar_str() {
                Some(category) => grant.category = category.to_owned(),
                None => refuse(refused, "battalion_mult", format!(
                    "sub-unit {key}: battalion_mult category is not a name"
                )),
            },
            b"add" => match yes(&item.value) {
                Ok(add) => grant.add = add,
                Err(why) => refuse(refused, "battalion_mult", format!("sub-unit {key}: battalion_mult {why}")),
            },
            word => match item.value.scalar_str() {
                Some(value) => grant
                    .words
                    .push((String::from_utf8_lossy(word).into_owned(), value.to_owned())),
                None => refuse(refused, "battalion_mult", format!(
                    "sub-unit {key}: battalion_mult field {} is not a scalar",
                    String::from_utf8_lossy(word)
                )),
            },
        }
    }
    if grant.category.is_empty() {
        refuse(refused, "battalion_mult", format!("sub-unit {key}: battalion_mult names no category"));
        return None;
    }
    Some(grant)
}

// ---------------------------------------------------------- the recorded template

/// Native sub-unit definition identity: a local clone and the global
/// definition share a token but are separate definitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Definition {
    Global(u32),
    Local { country: u32, token: u32 },
}

/// The hour's tables a template's definitions resolve through.
pub struct Sources<'a> {
    pub tokens: TokenNames<'a>,
    index: Arc<SourceIndex>,
}

/// The hour's country links and local sub-unit overrides, as [`Sources`] reads them.
#[derive(PartialEq, replay_spool::Spooled)]
pub struct SourceIndex {
    links: BTreeMap<u32, u32>,
    /// (canonical country, token) -> the override's (archetype token, amount).
    local: BTreeMap<(u32, u32), Vec<(u32, i64)>>,
    local_complete: bool,
}

impl SourceIndex {
    /// Its links and overrides.
    pub fn len(&self) -> usize {
        self.links.len() + self.local.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<'a> Sources<'a> {
    pub fn read(frame: &'a DecodedFrame) -> io::Result<Self> {
        let tokens = TokenNames::new(frame)?;
        Ok(Self {
            tokens,
            index: Arc::new(SourceIndex::read(frame)?),
        })
    }

    /// The sources of an hour whose links, local sub-units and tokens are those an earlier
    /// [`Self::read`] read `index` with: its tokens are taken as that read checked them.
    pub fn reuse(frame: &'a DecodedFrame, index: Arc<SourceIndex>) -> io::Result<Self> {
        let tokens = rows::<tables::LexicalTokens>(frame)?;
        Ok(Self {
            tokens: TokenNames::from_rows(tokens.rows(), tokens.is_complete()),
            index,
        })
    }

    /// [`Self::read`] of any frame's tables, a walk's views among them.
    pub fn read_in(frame: HourFrame<'a>) -> io::Result<Self> {
        if let Some(decoded) = frame.decoded() {
            return Self::read(decoded.decoded_frame());
        }
        let tokens = TokenNames::new_in(frame)?;
        Ok(Self {
            tokens,
            index: Arc::new(SourceIndex::read_in(frame)?),
        })
    }

    /// [`Self::reuse`] of any frame's tables, a walk's views among them.
    pub fn reuse_in(frame: HourFrame<'a>, index: Arc<SourceIndex>) -> io::Result<Self> {
        if let Some(decoded) = frame.decoded() {
            return Self::reuse(decoded.decoded_frame(), index);
        }
        Ok(Self {
            tokens: TokenNames::new_in(frame)?,
            index,
        })
    }

    /// Its links and overrides, kept for [`Self::reuse`].
    pub fn index(&self) -> &Arc<SourceIndex> {
        &self.index
    }
}

/// The CountryLinks fields the links are read by.
const LINK: u64 = replay_data::rows::mask::<replay_data::rows::CountryLinkRow>(&["canonical_slot"]);

impl SourceIndex {
    fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let links: BTreeMap<u32, u32> = frame
            .complete::<tables::CountryLinks>()?
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        let local_rows = rows::<tables::CountryLocalSubunits>(frame)?;
        Self::index(
            links,
            local_rows.rows().iter().map(|row| Ok(std::borrow::Cow::Borrowed(row))),
            local_rows.is_complete(),
        )
    }

    /// [`Self::read`] of a walk's views: its local sub-units are a truncated table's read
    /// rows, as a decoded frame's are.
    fn read_in(frame: HourFrame<'_>) -> io::Result<Self> {
        let mut links = BTreeMap::new();
        for row in frame.complete::<tables::CountryLinks>(LINK)?.iter() {
            let row = row?;
            links.insert(row.id, row.canonical_slot);
        }
        let table = TableId::CountryLocalSubunits;
        let local = frame.held::<tables::CountryLocalSubunits>(u64::MAX);
        Self::index(links, local.iter(), frame.status(table).is_complete())
    }

    fn index<'r>(
        links: BTreeMap<u32, u32>,
        local_rows: impl Iterator<
            Item = io::Result<std::borrow::Cow<'r, replay_data::rows::CountryLocalSubunitRow>>,
        >,
        local_complete: bool,
    ) -> io::Result<Self> {
        let mut local = BTreeMap::new();
        for row in local_rows {
            let row = row?;
            let country = *links.get(&row.country).ok_or_else(|| {
                invalid("a country's own sub-unit requirements name a country with no recorded country link")
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
            links,
            local,
            local_complete,
        })
    }
}

impl Sources<'_> {

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
            .and_then(|raw| self.index.links.get(&raw))
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
        let Some(entries) = sources.index.local.get(&(country, token)) else {
            // A complete table names every clone that differs.
            return if sources.index.local_complete {
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
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests;
