//! The ordinary land-model equipment/sub-unit pair, from recorded simulation
//! state and the matching `common/units` content.
//!
//! Model selection takes the template's stored visual representative,
//! filters the division's actual equipment, stably orders it by
//! signed quantity descending and returns the first eligible variant/sub-unit
//! pair, scoring the template's stored grouped entries
//! when a model category is saved. The representative and the grouped entries
//! are recorded; the pair itself is computed only while drawing, so it is
//! rebuilt here from those inputs.
//!
//! A country-local clone is a separate definition
//! from its global one: it groups apart, and its requirements are the
//! recorded override when `CountryLocalSubunits` names it, else the global
//! authored ones.

use std::sync::{Arc, OnceLock};
use std::{
    borrow::Cow,
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    io,
};
#[cfg(test)]
use viewer_foundation::map::subunit_files::subunit_keys;
use viewer_foundation::map::subunit_files::subunit_paths;

use installed_content::{
    ResolvedLayers,
    equipment::{Database, Equipment},
    parser::{self, Item, Value},
};
use replay::{
    frame::DecodedFrame,
    rows::{DivisionEquipmentRow, DivisionTemplateIconRow, VariantDefinitionRow, tables},
};

use viewer_foundation::map::tables::rows;
use viewer_foundation::table_access::{TokenKeys, TokenNames};

#[derive(Clone, Debug, Default)]
pub struct SubUnit {
    pub key: String,
    pub sprite: String,
    pub category: u64,
    /// `division_3d_model_priority`, which replaces the equipment score.
    pub model_priority: Option<i32>,
    /// Canonical (archetype, fixed5 amount) needs in native order.
    requirements: Vec<(String, i64)>,
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
/// `installed_content::equipment` loads it, `duplicate_archetypes` copies included.
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
            equipment: Arc::new(Database::load_complete(layers)?),
            ..Self::default()
        };
        for path in subunit_paths(layers.paths()) {
            let document = parser::parse(&layers.read(path)?)
                .map_err(|error| invalid(format!("{path}: {error}")))?;
            content
                .read_subunits(&document)
                .map_err(|error| invalid(format!("{path}: {error}")))?;
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
        let equipment = viewer_foundation::map::test_support::equipment_from_documents(documents);
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
        let model = installed_content::parser::text(&equipment.model).map_err(|refused| {
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
            Some(b"need") => {
                for need in block(&field.value)? {
                    let name = need
                        .key
                        .as_deref()
                        .ok_or_else(|| invalid("unkeyed subunit equipment requirement"))?;
                    let amount = super::equipment_graphics::authored_fixed5(
                        scalar(&need.value)?.as_bytes(),
                    )?;
                    // Lexical order until references resolve to archetypes.
                    subunit.requirements.push((text(name)?.to_owned(), amount));
                }
            }
            _ => {}
        }
    }
    Ok(subunit)
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

#[derive(Clone, Copy, Debug)]
pub struct SelectedPair<'a> {
    pub variant: &'a VariantDefinitionRow,
    pub subunit: &'a SubUnit,
    pub identity: Definition,
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

    fn group<'a>(
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

    /// Supply only this division's equipment rows, in any order;
    /// the recorded ordinal recovers native order before the stable quantity sort.
    /// Missing authored/recorded identities are errors, never silent re-ranking.
    pub fn select_pair<'a>(
        &'a self,
        template: &Template<'a>,
        model_category: Option<u64>,
        excluded_variants: &[u64],
        equipment: &[&DivisionEquipmentRow],
        variants: &BTreeMap<u64, &'a VariantDefinitionRow>,
        tokens: &TokenNames<'_>,
    ) -> io::Result<Option<SelectedPair<'a>>> {
        let Some(representative) = &template.representative else {
            return Ok(None);
        };
        // The first strictly largest positive requirement.
        let mut largest = None;
        let mut largest_amount = 0;
        for (archetype, amount) in representative.requirements.iter() {
            if *amount > largest_amount {
                largest = Some(archetype.as_str());
                largest_amount = *amount;
            }
        }
        let mut candidates = Vec::new();
        for row in equipment {
            let variant = *variants.get(&row.variant_handle).ok_or_else(|| {
                invalid(format!("missing equipment variant {}", row.variant_handle))
            })?;
            let key = self.equipment_key(tokens, variant).map_err(invalid)?;
            let archetype = self.archetype(key)?;
            let matches = match model_category {
                Some(category) => self.equipment_category(key)? == category,
                None => largest == Some(archetype),
            };
            if matches {
                candidates.push((*row, variant, key, archetype));
            }
        }
        candidates.sort_by_key(|(row, _, _, _)| row.ordinal);
        candidates.sort_by_key(|(row, _, _, _)| Reverse(row.amount_raw));
        for (_, variant, key, archetype) in candidates {
            if model_category.is_none() {
                return Ok(Some(SelectedPair {
                    variant,
                    subunit: representative.subunit,
                    identity: representative.identity,
                }));
            }
            if excluded_variants.contains(&variant.id) {
                continue;
            }
            let equipment_priority = self.equipment(key)?.model_priority;
            let mut selected = None;
            let mut best = -1;
            for group in &template.groups {
                if !group
                    .requirements
                    .iter()
                    .any(|(key, amount)| key == archetype && *amount > 0)
                {
                    continue;
                }
                // Native signed dword arithmetic, including its wrap behavior.
                let score = group.subunit.model_priority.unwrap_or_else(|| {
                    equipment_priority
                        .wrapping_add(10)
                        .wrapping_add(group.count.wrapping_mul(100))
                        .wrapping_mul(10)
                });
                if score > best {
                    best = score;
                    selected = Some(group);
                }
            }
            if let Some(group) = selected {
                return Ok(Some(SelectedPair {
                    variant,
                    subunit: group.subunit,
                    identity: group.identity,
                }));
            }
        }
        Ok(None)
    }
}

/// Model selection prefers a signed-positive variant origin over its creator. A missing
/// origin branch does not remap the ordinary roster country to its original tag.
pub fn model_country(pair: SelectedPair<'_>, roster_owner: u32) -> u32 {
    variant_country(pair.variant, pair.subunit, roster_owner)
}

/// The same rule for any (variant, sub-unit) pair and fallback country.
pub fn variant_country(variant: &VariantDefinitionRow, subunit: &SubUnit, fallback: u32) -> u32 {
    let category = variant.classifier_flags;
    let source = if (variant.origin_link as i32) > 0 {
        variant.origin_link
    } else {
        variant.creator_link
    };
    if category & 0x20 == 0 && category == subunit.category && (source as i32) > 0 {
        source
    } else {
        fallback
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
    installed_content::equipment::category_mask(name)
        .ok_or_else(|| invalid(format!("unknown equipment category {name}")))
}

#[cfg(test)]
mod tests;
