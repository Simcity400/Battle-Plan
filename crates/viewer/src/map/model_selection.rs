//! The ordinary land-model equipment/sub-unit pair, from recorded simulation
//! state and the matching `common/units` content.
//!
//! Native `141918950` takes the template's stored visual representative
//! (`140B91150`), filters the division's actual equipment, stably orders it by
//! signed quantity descending and returns the first eligible variant/sub-unit
//! pair, scoring the template's stored grouped entries (composition `+0xA8`)
//! when a model category is saved. The representative and the grouped entries
//! are recorded; the pair itself is computed only while drawing, so it is
//! rebuilt here from those inputs.
//!
//! A country-local clone (`CDeploymentStatus+0x20`) is a separate definition
//! from its global one: it groups apart, and its requirements are the
//! recorded override when `CountryLocalSubunits` names it, else the global
//! authored ones.

use std::{
    borrow::Cow,
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::{
    ResolvedLayers,
    parser::{self, Item, Value},
};
use replay::{
    frame::DecodedFrame,
    rows::{
        DivisionEquipmentRow, DivisionTemplateIconRow, LexicalTokenRow, VariantDefinitionRow,
        tables,
    },
};

use super::tables::rows;

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

#[derive(Clone, Debug, Default)]
struct Equipment {
    /// Native definition insertion order, retained across later redefinitions.
    ordinal: usize,
    is_archetype: bool,
    archetype: Option<String>,
    parent: Option<String>,
    /// The original definition at +4E8 for an authored archetype duplicate.
    family: Option<String>,
    primary_category: u64,
    module_slots: BTreeSet<String>,
    default_modules: BTreeMap<String, String>,
    visual_addition: i32,
    visual_level: i32,
    model: String,
}

#[derive(Clone, Copy, Debug)]
pub struct EquipmentModel<'a> {
    pub archetype: &'a str,
    /// Native archetype registration order is the first production comparator.
    pub archetype_ordinal: usize,
    pub family: Option<&'a str>,
    pub model_priority: i32,
    pub visual_level: i32,
    pub model: &'a str,
    pub depth: i32,
}

/// Authored sub-units and equipment in the engine's load order.
#[derive(Clone, Debug, Default)]
pub struct Content {
    subunits: BTreeMap<String, SubUnit>,
    equipment: BTreeMap<String, Equipment>,
    module_categories: BTreeMap<String, u64>,
}

fn model_definition_source(path: &str) -> Option<(u8, &str)> {
    let lower = path.to_ascii_lowercase();
    if !lower.ends_with(".txt") {
        return None;
    }
    if lower.starts_with("common/units/equipment/") {
        // The native equipment database descends into child directories.
        Some((1, path))
    } else {
        let relative = lower.strip_prefix("common/units/")?;
        (!relative.contains('/')).then_some((0, path))
    }
}

fn ordered_model_definition_paths<'a>(paths: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut sources: Vec<_> = paths.filter_map(model_definition_source).collect();
    sources.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.as_bytes().cmp(right.1.as_bytes()))
    });
    sources.into_iter().map(|(_, path)| path).collect()
}

impl Content {
    pub fn load(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut content = Self::default();
        // ResolvedLayers has already applied same-path mount replacement.
        // effective filenames by source kind and path bytes, not layer order.
        // Sub-units live directly under common/units; equipment definitions
        // and modules live under equipment. Other children hold historical
        // names and unrelated data.
        for path in ordered_model_definition_paths(layers.paths()) {
            let document = parser::parse(&layers.read(path)?)
                .map_err(|error| invalid(format!("{path}: {error}")))?;
            content
                .read_document(&document)
                .map_err(|error| invalid(format!("{path}: {error}")))?;
        }
        content.canonicalize_requirements()?;
        Ok(content)
    }

    fn read_document(&mut self, document: &parser::Document) -> io::Result<()> {
        for root in &document.items {
            let Some(items) = root.value.items() else {
                continue;
            };
            for item in items {
                let (Some(key), Some(fields)) = (item.key.as_deref(), item.value.items()) else {
                    continue;
                };
                let key = text(key)?.to_owned();
                match root.key.as_deref() {
                    Some(b"sub_units") => {
                        let subunit = read_subunit(&key, fields)?;
                        self.subunits.insert(key, subunit);
                    }
                    Some(b"equipments") => {
                        let equipment = self.read_equipment(&key, fields)?;
                        self.insert_equipment(key, equipment);
                    }
                    Some(b"duplicate_archetypes") => self.duplicate_archetype(&key, fields)?,
                    Some(b"equipment_modules") => {
                        let mut mask = 0;
                        for field in fields
                            .iter()
                            .filter(|field| field.key.as_deref() == Some(b"add_equipment_type"))
                        {
                            // 14151D190/140F75BC0 replaces this field, unlike type's OR.
                            mask = category(&field.value, true)?;
                        }
                        self.module_categories.insert(key, mask);
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn read_equipment(&self, key: &str, fields: &[Item]) -> io::Result<Equipment> {
        let mut equipment = Equipment {
            visual_addition: 1,
            ..Equipment::default()
        };
        for field in fields {
            match field.key.as_deref() {
                Some(b"is_archetype") => {
                    equipment.is_archetype = scalar(&field.value)? == "yes";
                }
                Some(b"archetype") => equipment.archetype = Some(scalar(&field.value)?.to_owned()),
                Some(b"parent") => equipment.parent = Some(scalar(&field.value)?.to_owned()),
                Some(b"type") => equipment.primary_category |= category(&field.value, false)?,
                Some(b"visual_tech_level_addition") => {
                    equipment.visual_addition = integer(&field.value)?
                }
                Some(b"visual_level") => equipment.visual_level = integer(&field.value)?,
                Some(b"model") => equipment.model = scalar(&field.value)?.to_owned(),
                Some(b"module_slots") => {
                    self.read_module_slots(key, &mut equipment, &field.value)?
                }
                Some(b"default_modules") => {
                    for entry in block(&field.value)? {
                        let slot = entry
                            .key
                            .as_deref()
                            .ok_or_else(|| invalid("unkeyed default module"))?;
                        equipment
                            .default_modules
                            .insert(text(slot)?.to_owned(), scalar(&entry.value)?.to_owned());
                    }
                }
                _ => {}
            }
        }
        Ok(equipment)
    }

    fn insert_equipment(&mut self, key: String, mut equipment: Equipment) {
        equipment.ordinal = self
            .equipment
            .get(&key)
            .map_or(self.equipment.len(), |previous| previous.ordinal);
        self.equipment.insert(key, equipment);
    }

    fn read_module_slots(
        &self,
        key: &str,
        equipment: &mut Equipment,
        value: &Value,
    ) -> io::Result<()> {
        // 140C89CC0/1409EB0E0 copy slots immediately, using the parent or
        // archetype already specified at this point in the authored definition.
        let source = equipment
            .parent
            .as_deref()
            .filter(|parent| *parent != key)
            .or_else(|| {
                equipment
                    .archetype
                    .as_deref()
                    .filter(|archetype| *archetype != key)
            });
        let inherited = |source: Option<&str>| -> io::Result<&BTreeSet<String>> {
            let source = source.ok_or_else(|| {
                invalid(format!(
                    "equipment {key} inherits module slots before specifying a parent/archetype"
                ))
            })?;
            Ok(&self.equipment(source)?.module_slots)
        };
        if let Some(entries) = value.items() {
            for entry in entries {
                let name = text(
                    entry
                        .key
                        .as_deref()
                        .ok_or_else(|| invalid("unkeyed equipment module slot"))?,
                )?;
                // 140C8C260 retains the earlier same-name slot.
                if equipment.module_slots.contains(name) {
                    continue;
                }
                if entry.value.items().is_none() {
                    let alias = scalar(&entry.value)?;
                    let exists = if alias == "inherit" {
                        inherited(source)?.contains(name)
                    } else {
                        equipment.module_slots.contains(alias)
                    };
                    if !exists {
                        return Err(invalid(format!(
                            "equipment {key} module slot {name} has no source {alias}"
                        )));
                    }
                }
                equipment.module_slots.insert(name.to_owned());
            }
        } else if scalar(value)? == "inherit" {
            equipment.module_slots = inherited(source)?.clone();
        }
        Ok(())
    }

    /// 1409EA240/140C84DD0 expand matching content into real equipment types.
    /// The clone retains its original family but remaps archetype and parent
    /// to already-created clones; `None` models a native self-link for an
    /// unmapped parent or the root's archetype.
    fn duplicate_archetype(&mut self, name: &str, fields: &[Item]) -> io::Result<()> {
        let mut source = None;
        let mut primary_category = 0;
        let mut only_archetype = false;
        let mut clear_modules = false;
        for field in fields {
            match field.key.as_deref() {
                Some(b"archetype") => source = Some(scalar(&field.value)?),
                // CDuplicateArchetypeDefinition uses replacement, not OR.
                Some(b"type") => primary_category = category(&field.value, true)?,
                Some(b"only_duplicate_archetype") => {
                    only_archetype = scalar(&field.value)? == "yes";
                }
                Some(b"module_slots") => clear_modules = scalar(&field.value)? == "none",
                // for_each changes numeric equipment stats and variant_name,
                // neither of which supplies one of this module's operands.
                _ => {}
            }
        }
        let source =
            source.ok_or_else(|| invalid(format!("duplicate archetype {name} has no source")))?;
        if !self.equipment(source)?.is_archetype {
            return Err(invalid(format!(
                "duplicate source {source} is not an archetype"
            )));
        }
        let mut originals = Vec::new();
        for (key, equipment) in &self.equipment {
            if equipment.family.is_none()
                && (key == source || equipment.archetype.as_deref() == Some(source))
                && (!only_archetype || key == source)
            {
                originals.push((
                    self.equipment_hierarchy_depth(key)?,
                    equipment.ordinal,
                    key.clone(),
                    equipment.clone(),
                ));
            }
        }
        // Native's stable depth sort preserves database insertion order for
        // equal-depth definitions, including generated-name collisions.
        originals.sort_by_key(|(depth, ordinal, _, _)| (*depth, *ordinal));
        let mut remapped = BTreeMap::<String, String>::new();
        for (_, _, key, mut equipment) in originals {
            // Native removes the longest common byte prefix, including when
            // the equipment's name is not prefixed by its archetype's name.
            let common = key
                .bytes()
                .zip(source.bytes())
                .take_while(|(a, b)| a == b)
                .count();
            let clone_name = format!("{name}{}", text(&key.as_bytes()[common..])?);
            // Cold native content load diagnoses and keeps an existing valid
            // definition. It does not remap that source to the colliding one.
            if self.equipment.contains_key(&clone_name) {
                continue;
            }
            remapped.insert(key.clone(), clone_name.clone());
            equipment.archetype = remapped
                .get(source)
                .cloned()
                .filter(|key| key != &clone_name);
            equipment.is_archetype = equipment.archetype.is_none();
            equipment.parent = equipment
                .parent
                .as_ref()
                .and_then(|parent| remapped.get(parent))
                .cloned()
                .filter(|key| key != &clone_name);
            equipment.family = Some(key);
            equipment.primary_category = primary_category;
            if clear_modules {
                equipment.module_slots.clear();
                equipment.default_modules.clear();
            }
            self.insert_equipment(clone_name, equipment);
        }
        Ok(())
    }

    fn equipment_hierarchy_depth(&self, key: &str) -> io::Result<u32> {
        let mut depth = 0;
        let mut current = key;
        // 1409E0F10 sorts ancestors before descendants with a bounded walk.
        while depth < 64 {
            let equipment = self.equipment(current)?;
            let next = equipment
                .parent
                .as_deref()
                .filter(|parent| *parent != current)
                .or_else(|| {
                    equipment
                        .archetype
                        .as_deref()
                        .filter(|archetype| *archetype != current)
                });
            let Some(next) = next else { break };
            current = next;
            depth += 1;
        }
        Ok(depth)
    }

    fn canonicalize_requirements(&mut self) -> io::Result<()> {
        // 140FFC070 -> 140FFB0B0 stores archetypes, preserving first insertion
        // order while later references to the same archetype replace its value.
        let mut canonical = BTreeMap::new();
        for (key, unit) in &self.subunits {
            let mut requirements = Vec::<(String, i64)>::new();
            for (name, amount) in &unit.requirements {
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

    /// Native 140C88540: DLC eligibility is tested only for definitions that
    /// actually have copied/authored module slots, using their effective type.
    pub fn has_enabled_module_slots(&self, key: &str, dlc_mask: u64) -> io::Result<bool> {
        Ok(!self.equipment(key)?.module_slots.is_empty()
            && self.equipment_category(key)? & dlc_mask != 0)
    }

    pub fn equipment_model<'a>(&'a self, key: &'a str) -> io::Result<EquipmentModel<'a>> {
        let equipment = self.equipment(key)?;
        let (model_priority, depth) = self.equipment_priority_and_depth(key)?;
        let archetype = self.archetype(key)?;
        Ok(EquipmentModel {
            archetype,
            archetype_ordinal: self.equipment(archetype)?.ordinal,
            family: equipment.family.as_deref(),
            model_priority,
            visual_level: equipment.visual_level,
            model: &equipment.model,
            depth,
        })
    }

    pub fn subunit(&self, key: &str) -> Option<&SubUnit> {
        self.subunits.get(key)
    }

    fn equipment(&self, key: &str) -> io::Result<&Equipment> {
        self.equipment
            .get(key)
            .ok_or_else(|| invalid(format!("missing authored equipment {key}")))
    }

    pub(crate) fn archetype<'a>(&'a self, key: &'a str) -> io::Result<&'a str> {
        let equipment = self.equipment(key)?;
        let archetype = equipment.archetype.as_deref().unwrap_or(key);
        self.equipment(archetype)?;
        Ok(archetype)
    }

    fn equipment_priority(&self, key: &str) -> io::Result<i32> {
        self.equipment_priority_and_depth(key)
            .map(|(priority, _)| priority)
    }

    fn equipment_priority_and_depth(&self, key: &str) -> io::Result<(i32, i32)> {
        let mut priority = 0_i32;
        let mut current = key;
        for depth in 0..=self.equipment.len() {
            let equipment = self.equipment(current)?;
            priority = priority.wrapping_add(equipment.visual_addition);
            match equipment.parent.as_deref() {
                Some(parent) if parent != current => current = parent,
                _ => return Ok((priority.max(1), depth as i32)),
            }
        }
        Err(invalid(format!("equipment parent cycle for {key}")))
    }

    fn equipment_category(&self, key: &str) -> io::Result<u64> {
        let equipment = self.equipment(key)?;
        let mut category = equipment.primary_category;
        if category == 0 {
            category = self.equipment(self.archetype(key)?)?.primary_category;
        }
        for module in equipment.default_modules.values() {
            if module == "empty" {
                continue;
            }
            // 140C85280 resolves each literal default-module token directly.
            // Missing tokens, including the authored value `inherit`, resolve
            // to the inactive NullObject and contribute no category bits.
            if let Some(mask) = self.module_categories.get(module) {
                category |= *mask;
            }
        }
        Ok(normalize_category(category))
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
            Some(b"type") => subunit.category |= category(&field.value, false)?,
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

/// The lexer dictionary, read in place: replay rows are in ascending key order,
/// so a lookup is a binary search rather than a copy of every literal.
#[derive(Clone, Copy, Debug)]
pub struct Tokens<'a>(&'a [LexicalTokenRow]);

impl<'a> Tokens<'a> {
    /// Rows must be in ascending token order, as every replay table is.
    pub fn new(rows: &'a [LexicalTokenRow]) -> Self {
        Self(rows)
    }

    pub fn read(frame: &'a DecodedFrame) -> io::Result<Self> {
        Ok(Self::new(rows::<tables::LexicalTokens>(frame)?.rows()))
    }

    pub fn get(self, token: u32) -> Option<&'a [u8]> {
        self.0
            .binary_search_by_key(&token, |row| row.token)
            .ok()
            .map(|index| self.0[index].literal.as_slice())
    }
}

/// The hour's tables a template's definitions resolve through.
pub struct Sources<'a> {
    pub tokens: Tokens<'a>,
    links: BTreeMap<u32, u32>,
    /// (canonical country, token) -> the override's (archetype token, amount).
    local: BTreeMap<(u32, u32), Vec<(u32, i64)>>,
    local_complete: bool,
}

impl<'a> Sources<'a> {
    pub fn read(frame: &'a DecodedFrame) -> io::Result<Self> {
        let tokens = Tokens::read(frame)?;
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
pub(crate) struct Group<'a> {
    pub identity: Definition,
    pub subunit: &'a SubUnit,
    pub count: i32,
    pub requirements: Cow<'a, [(String, i64)]>,
}

/// A recorded template resolved against content.
#[derive(Debug)]
pub struct Template<'a> {
    /// `140B91150`'s representative; `None` is the null object.
    pub(crate) representative: Option<Group<'a>>,
    /// The stored grouped entries, null definitions dropped.
    pub(crate) groups: Vec<Group<'a>>,
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
    fn requirements<'a>(
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
            let literal = sources.tokens.get(archetype_token).ok_or_else(|| {
                invalid(format!(
                    "local sub-unit {country}:{token} requirement token {archetype_token} is missing"
                ))
            })?;
            let archetype = self.archetype(text(literal)?)?;
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
        let literal = sources
            .tokens
            .get(token)
            .ok_or_else(|| invalid(format!("missing sub-unit token {token}")))?;
        let key = text(literal)?;
        let subunit = self
            .subunits
            .get(key)
            .ok_or_else(|| invalid(format!("missing authored sub-unit {key}")))?;
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

    /// `141918950`. Supply only this division's equipment rows, in any order;
    /// the recorded ordinal recovers native order before the stable quantity sort.
    /// Missing authored/recorded identities are errors, never silent re-ranking.
    pub fn select_pair<'a>(
        &'a self,
        template: &Template<'a>,
        model_category: Option<u64>,
        excluded_variants: &[u64],
        equipment: &[&DivisionEquipmentRow],
        variants: &BTreeMap<u64, &'a VariantDefinitionRow>,
    ) -> io::Result<Option<SelectedPair<'a>>> {
        let Some(representative) = &template.representative else {
            return Ok(None);
        };
        // 140FFABF0: the first strictly largest positive requirement.
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
            let key = text(&variant.equipment_name)?;
            let archetype = self.archetype(key)?;
            let matches = match model_category {
                Some(category) => self.equipment_category(key)? == category,
                None => largest == Some(archetype),
            };
            if matches {
                candidates.push((*row, variant, archetype));
            }
        }
        candidates.sort_by_key(|(row, _, _)| row.ordinal);
        candidates.sort_by_key(|(row, _, _)| Reverse(row.amount_raw));
        for (_, variant, archetype) in candidates {
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
            let equipment_priority = self.equipment_priority(text(&variant.equipment_name)?)?;
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

/// Native `14191A770`: +20 wins over +1C only when signed-positive. A missing
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

fn normalize_category(mut category: u64) -> u64 {
    // 140F75EA0; only these mutually exclusive ship categories are normalized.
    if category & 0x0080_0040_03c1 != 0 {
        if category & 0x40 != 0 {
            category &= !0x181;
        } else if category & 0x80 != 0 {
            category &= !0x101;
        } else if category & 0x100 != 0 {
            category &= !1;
        }
    }
    category
}

pub(crate) fn text(bytes: &[u8]) -> io::Result<&str> {
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

fn category(value: &Value, numeric: bool) -> io::Result<u64> {
    if let Some(items) = value.items() {
        return items
            .iter()
            .try_fold(0, |mask, item| Ok(mask | category(&item.value, numeric)?));
    }
    let name = scalar(value)?;
    if numeric && let Ok(mask) = name.parse::<u64>() {
        return Ok(mask);
    }
    category_name(name).ok_or_else(|| invalid(format!("unknown equipment category {name}")))
}

fn category_name(name: &str) -> Option<u64> {
    // 140F731B0's supported-build switch. These keywords are the executable's,
    // not content: an unknown one is refused, never mapped to a fallback bit.
    Some(match name {
        "unknown" => 0,
        "convoy" => 1,
        "armor" => 1 << 2,
        "motorized" => 1 << 3,
        "mechanized" => 1 << 4,
        "infantry" => 1 << 5,
        "capital_ship" => 1 << 6,
        "submarine" => 1 << 7,
        "screen_ship" => 1 << 8,
        "floating_harbor" => 1 << 9,
        "fighter" => 1 << 10,
        "interceptor" => 1 << 11,
        "tactical_bomber" => 1 << 12,
        "strategic_bomber" => 1 << 13,
        "cas" => 1 << 14,
        "naval_bomber" => 1 << 15,
        "missile" => 1 << 16,
        "suicide" => 1 << 17,
        "scout_plane" => 1 << 18,
        "railway_gun" => 1 << 19,
        "air_transport" => 1 << 20,
        "maritime_patrol_plane" => 1 << 21,
        "carrier" => 1 << 22,
        "support" => 1 << 24,
        "flame" => 1 << 25,
        "amphibious" => 1 << 26,
        "anti_air" => 1 << 27,
        "artillery" => 1 << 28,
        "anti_tank" => 1 << 29,
        "rocket" => 1 << 30,
        "train" => 1 << 31,
        "heavy_fighter" => 1 << 32,
        "emplacement_gun_ammo" => 1 << 33,
        "ballistic_missile" => 1 << 34,
        "nuclear_missile" => 1 << 35,
        "sam_missile" => 1 << 36,
        "missile_launcher" => 1 << 37,
        "land_cruiser" => 1 << 38,
        "support_ship" => 1 << 39,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::TemplateSubunit;

    fn content() -> Content {
        let mut content = Content::default();
        content
            .read_document(
                &parser::parse(
                    br#"
            sub_units = {
                infantry = { type = infantry need = { rifles = 100 } }
                armor = { type = armor need = { tanks = 50 rifles = 20 } }
                support = { type = { infantry support }
                    division_3d_model_priority = 99999 need = { rifles = 20 } }
            }
            equipments = {
                rifles = { is_archetype = yes type = infantry }
                tanks = { is_archetype = yes type = armor }
                rifles_1 = { archetype = rifles }
                tanks_1 = { archetype = tanks }
                tanks_2 = { archetype = tanks parent = tanks_1 visual_tech_level_addition = 2 }
            }
        "#,
                )
                .unwrap(),
            )
            .unwrap();
        content.canonicalize_requirements().unwrap();
        content
    }

    type Overrides = [((u32, u32), Vec<(u32, i64)>)];

    fn sources(local: &Overrides) -> Sources<'static> {
        let tokens: &'static [LexicalTokenRow] = Box::leak(
            [
                &b"infantry"[..],
                b"armor",
                b"support",
                b"rifles",
                b"tanks",
                b"rifles_1",
            ]
            .iter()
            .zip(1..)
            .map(|(literal, token)| LexicalTokenRow {
                token,
                literal: literal.to_vec(),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        );
        Sources {
            tokens: Tokens(tokens),
            links: [(10, 7)].into(),
            local: local.iter().cloned().collect(),
            local_complete: true,
        }
    }

    fn variants() -> BTreeMap<u64, &'static VariantDefinitionRow> {
        [(10, &b"rifles_1"[..]), (20, b"tanks_1"), (30, b"tanks_2")]
            .into_iter()
            .map(|(id, name)| {
                let row: &'static VariantDefinitionRow =
                    Box::leak(Box::new(VariantDefinitionRow {
                        id,
                        equipment_name: name.to_vec(),
                        ..Default::default()
                    }));
                (id, row)
            })
            .collect()
    }

    fn row(representative: (u32, u8), subunits: &[(u32, u8, i32)]) -> DivisionTemplateIconRow {
        DivisionTemplateIconRow {
            country: 10,
            representative: representative.0,
            representative_local: representative.1,
            subunits: subunits
                .iter()
                .map(|&(token, local, count)| TemplateSubunit {
                    token,
                    local,
                    count,
                })
                .collect(),
            ..Default::default()
        }
    }

    fn equipment(handle: u64, amount_raw: i64, ordinal: u32) -> DivisionEquipmentRow {
        DivisionEquipmentRow {
            variant_handle: handle,
            amount_raw,
            ordinal,
            ..Default::default()
        }
    }

    #[test]
    fn an_authored_need_past_five_decimals_truncates_like_the_engine() {
        let document = parser::parse(b"x = { need = { rifles = 0.1234567 } }").unwrap();
        let fields = document.items[0].value.items().unwrap();
        let subunit = read_subunit("x", fields).unwrap();
        assert_eq!(subunit.requirements, [("rifles".to_owned(), 12_345)]);
    }

    #[test]
    fn model_definitions_exclude_historical_names_but_keep_nested_equipment() {
        for path in [
            "common/units/infantry.txt",
            "common/units/equipment/infantry.txt",
            "common/units/equipment/modules/00_tank_modules.txt",
        ] {
            assert!(model_definition_source(path).is_some(), "{path}");
        }
        for path in [
            "common/units/names_divisions/BRA_names_divisions.txt",
            "common/units/codenames_operatives/GER.txt",
            "common/units/infantry.asset",
        ] {
            assert!(model_definition_source(path).is_none(), "{path}");
        }
        assert_eq!(
            ordered_model_definition_paths(
                [
                    "common/units/equipment/z_base.txt",
                    "common/units/equipment/nested/a.txt",
                    "common/units/direct.txt",
                ]
                .into_iter()
            ),
            [
                "common/units/direct.txt",
                "common/units/equipment/nested/a.txt",
                "common/units/equipment/z_base.txt",
            ]
        );
    }

    #[test]
    fn duplicate_archetypes_copy_model_inputs_and_remap_the_parent_chain() {
        let mut content = Content::default();
        content
            .read_document(
                &parser::parse(
                    br#"
            equipment_modules = { turret = { add_equipment_type = artillery } }
            equipments = {
                tanks = { is_archetype = yes type = armor model = tank_mesh }
                tanks_0 = { archetype = tanks visual_level = 3 model = tier_mesh
                    default_modules = { weapon = turret } }
                tanks_1 = { archetype = tanks parent = tanks_0 visual_tech_level_addition = 2 }
                infantry = { is_archetype = yes type = infantry }
                old_tank = { archetype = tanks parent = infantry }
            }
            duplicate_archetypes = {
                flame = { archetype = tanks type = { armor flame } }
                amphibious = { archetype = tanks type = { armor amphibious } module_slots = none }
                root_only = { archetype = tanks type = armor only_duplicate_archetype = yes }
            }
        "#,
                )
                .unwrap(),
            )
            .unwrap();
        let model = content.equipment_model("flame_1").unwrap();
        assert_eq!(model.archetype, "flame");
        assert_eq!(model.family, Some("tanks_1"));
        assert_eq!((model.model_priority, model.depth), (3, 1));
        let first = content.equipment_model("flame_0").unwrap();
        assert_eq!((first.visual_level, first.model), (3, "tier_mesh"));
        assert_eq!(
            content.equipment_category("flame_0").unwrap(),
            (1 << 2) | (1 << 25) | (1 << 28)
        );
        assert_eq!(
            content.equipment_category("amphibious_0").unwrap(),
            (1 << 2) | (1 << 26)
        );
        // An original parent outside the cloned family becomes a native self-link.
        assert_eq!(content.equipment("flameold_tank").unwrap().parent, None);
        assert!(content.equipment_model("root_only").is_ok());
        assert!(content.equipment_model("root_only_0").is_err());
    }

    #[test]
    fn enabled_module_slots_follow_copied_slots_and_effective_category() {
        let mut content = Content::default();
        content
            .read_document(
                &parser::parse(
                    br#"
            equipments = {
                tanks = { is_archetype = yes type = armor
                    module_slots = { main = {} secondary = main } }
                empty = { is_archetype = yes type = infantry }
                tanks_0 = { archetype = tanks module_slots = inherit }
                tanks_empty_parent = { archetype = tanks parent = empty module_slots = inherit }
            }
            duplicate_archetypes = { legacy = { archetype = tanks type = armor module_slots = none } }
        "#,
                )
                .unwrap(),
            )
            .unwrap();
        for key in ["tanks", "tanks_0"] {
            assert!(
                content.has_enabled_module_slots(key, 1 << 2).unwrap(),
                "{key}"
            );
            assert!(
                !content.has_enabled_module_slots(key, 1 << 5).unwrap(),
                "{key}"
            );
        }
        for key in ["empty", "tanks_empty_parent", "legacy", "legacy_0"] {
            assert!(
                !content.has_enabled_module_slots(key, u64::MAX).unwrap(),
                "{key}"
            );
        }
        assert!(
            content
                .has_enabled_module_slots("missing", u64::MAX)
                .is_err()
        );
    }

    #[test]
    fn requirement_aliases_overwrite_in_source_order_after_archetype_resolution() {
        let mut content = content();
        content
            .read_document(
                &parser::parse(
                    b"sub_units = { aliases = { need = { rifles = 1 tanks = 9 rifles_1 = 2 rifles = 3 } } }",
                )
                .unwrap(),
            )
            .unwrap();
        content.canonicalize_requirements().unwrap();
        assert_eq!(
            content.subunit("aliases").unwrap().requirements,
            [
                ("rifles".to_owned(), 300_000),
                ("tanks".to_owned(), 900_000)
            ]
        );
        assert!(
            content
                .read_document(&parser::parse(b"sub_units = { bad = { type = 4 } }").unwrap())
                .is_err()
        );
    }

    #[test]
    fn without_a_category_the_representative_largest_need_filters_by_quantity_then_order() {
        let content = content();
        let template = content
            .template(&row((2, 0), &[(2, 0, 1), (1, 0, 1)]), &sources(&[]))
            .unwrap();
        let variants = variants();
        // The representative is armor: tanks is its largest need, so rifles
        // never qualify, and equal quantities keep the older ordinal.
        let rows = [
            equipment(10, 900, 0),
            equipment(30, 500, 7),
            equipment(20, 500, 2),
        ];
        let refs: Vec<_> = rows.iter().collect();
        let pair = content
            .select_pair(&template, None, &[20], &refs, &variants)
            .unwrap()
            .unwrap();
        assert_eq!((pair.variant.id, pair.subunit.key.as_str()), (20, "armor"));
        // A null representative draws no pair at all.
        let null = content
            .template(&row((0, 0), &[(2, 0, 1)]), &sources(&[]))
            .unwrap();
        assert!(
            content
                .select_pair(&null, None, &[], &refs, &variants)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_category_scores_the_stored_groups_and_skips_excluded_variants() {
        let content = content();
        let template = content
            .template(&row((3, 0), &[(2, 0, 1), (1, 0, 2)]), &sources(&[]))
            .unwrap();
        let variants = variants();
        let rows = [
            equipment(20, 500, 0),
            equipment(30, 500, 1),
            equipment(10, 1, 2),
        ];
        let refs: Vec<_> = rows.iter().collect();
        // Armor wins the tanks pair: the only group needing tanks.
        let pair = content
            .select_pair(&template, Some(1 << 2), &[20], &refs, &variants)
            .unwrap()
            .unwrap();
        assert_eq!((pair.variant.id, pair.subunit.key.as_str()), (30, "armor"));
        // Rifles: infantry's two entries outscore armor's one.
        let pair = content
            .select_pair(&template, Some(1 << 5), &[], &refs, &variants)
            .unwrap()
            .unwrap();
        assert_eq!(pair.subunit.key, "infantry");
        let missing = [equipment(99, 1, 0)];
        assert!(
            content
                .select_pair(&template, Some(1 << 5), &[], &[&missing[0]], &variants)
                .is_err()
        );
    }

    #[test]
    fn local_clones_group_apart_and_take_their_recorded_requirements() {
        let content = content();
        // Local infantry needs tanks, not rifles.
        let local = sources(&[((7, 1), vec![(5, 200_000)])]);
        let template = content
            .template(&row((1, 1), &[(1, 0, 1), (1, 1, 1), (2, 0, 1)]), &local)
            .unwrap();
        assert_eq!(
            template.groups[1].identity,
            Definition::Local {
                country: 7,
                token: 1
            }
        );
        let representative = template.representative.as_ref().unwrap();
        assert_eq!(Template::first_requirement(representative), Some("tanks"));
        let variants = variants();
        let rows = [equipment(10, 900, 0), equipment(20, 100, 1)];
        let refs: Vec<_> = rows.iter().collect();
        let pair = content
            .select_pair(&template, None, &[], &refs, &variants)
            .unwrap()
            .unwrap();
        assert_eq!(pair.variant.id, 20);
        assert_eq!(
            pair.identity,
            Definition::Local {
                country: 7,
                token: 1
            }
        );
        // A clone absent from a complete table has the global requirements;
        // from an incomplete one it is unknown.
        let equal = content.template(&row((1, 1), &[]), &sources(&[])).unwrap();
        assert_eq!(
            Template::first_requirement(equal.representative.as_ref().unwrap()),
            Some("rifles")
        );
        let incomplete = Sources {
            local_complete: false,
            ..sources(&[])
        };
        assert!(content.template(&row((1, 1), &[]), &incomplete).is_err());
        // Two tokens resolving to one archetype cannot come from a native pool.
        let repeated = sources(&[((7, 1), vec![(4, 1), (6, 2)])]);
        assert!(content.template(&row((1, 1), &[]), &repeated).is_err());
    }

    #[test]
    fn country_override_requires_exact_category_and_signed_positive_origin() {
        let content = content();
        let subunit = content.subunit("armor").unwrap();
        let mut variant = VariantDefinitionRow {
            origin_link: u32::MAX,
            creator_link: 7,
            classifier_flags: 4,
            ..Default::default()
        };
        assert_eq!(variant_country(&variant, subunit, 9), 7);
        variant.origin_link = 8;
        assert_eq!(variant_country(&variant, subunit, 9), 8);
        variant.classifier_flags |= 32;
        assert_eq!(variant_country(&variant, subunit, 9), 9);
        assert_eq!(normalize_category(0x1c1), 0x40);
    }
}
