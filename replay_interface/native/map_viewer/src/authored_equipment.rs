mod generation;
mod designer;

use crate::{ActiveDlcPresentation, ContentAdmissionLease, GameFileResolver};
use portable_content_manifest::paradox_text::{self, Value};
use replay_core::UsedContentKey;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EquipmentDefinitionError {
    ContentAdmission(String),
    MalformedContent { source: String, detail: String },
    MissingDefinition(String),
    UnresolvedGeneration { token: String, sources: Vec<String> },
    AmbiguousDefinition { token: String, sources: Vec<String> },
    MalformedField { token: String, field: String },
    MissingField { token: String, field: String },
    VariableExpansion { source: String, detail: String },
    UnsupportedAuthored { token: String, field: String },
    RelationshipCycle(Vec<String>),
    NotArchetype(String),
    UnknownCategory { token: String, category: String },
    MissingArchetypeCategory(String),
    MissingActiveDlcProfile,
    ContentConditionProfileChanged,
    DisabledByContentCondition(String),
}

impl std::fmt::Display for EquipmentDefinitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "equipment definition: {self:?}")
    }
}
impl std::error::Error for EquipmentDefinitionError {}

type EquipmentResult<T> = Result<T, EquipmentDefinitionError>;

#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredEquipmentDefinition {
    pub token: String,
    pub source: UsedContentKey,
    pub authored: Value,
    pub generation: Option<EquipmentGeneration>,
    expanded: Value,
    generated_type_mask: Option<u64>,
    resolved_module_slots: Option<EquipmentResult<Vec<String>>>,
    container_limit: Option<Value>,
}

impl AuthoredEquipmentDefinition {
    fn identity(&self) -> EquipmentDefinitionIdentity {
        EquipmentDefinitionIdentity { token: self.token.clone(), source: self.source.clone() }
    }

    pub fn field(&self, name: &str) -> Option<&Value> {
        self.authored.fields().iter().rev()
            .find(|entry| entry.key_matches(name)).map(|entry| &entry.value)
    }

    pub fn scalar(&self, name: &str) -> EquipmentResult<Option<&str>> {
        let value = self.expanded.fields().iter().rev()
            .find(|entry| entry.key_matches(name)).map(|entry| &entry.value);
        value.map(|value| value.as_scalar().ok_or_else(|| self.malformed(name))).transpose()
    }

    pub fn signed_integer(&self, name: &str) -> EquipmentResult<Option<i32>> {
        self.scalar(name)?.map(|value| value.parse::<i32>().map_err(|_| self.malformed(name)))
            .transpose()
    }

    pub fn boolean(&self, name: &str) -> EquipmentResult<Option<bool>> {
        self.scalar(name)?.map(|value| match value {
            "yes" => Ok(true), "no" => Ok(false), _ => Err(self.malformed(name)),
        }).transpose()
    }

    pub fn type_mask(&self) -> EquipmentResult<u64> {
        if let Some(mask) = self.generated_type_mask { return Ok(mask) }
        let mut mask = 0;
        for entry in self.expanded.fields().iter().filter(|entry| entry.key_matches("type")) {
            mask |= self.category_value(&entry.value, "type", false)?;
        }
        Ok(mask)
    }

    pub fn additional_type_mask(&self) -> EquipmentResult<u64> {
        let mut mask = 0;
        for entry in self.expanded.fields().iter().filter(|entry| entry.key_matches("add_equipment_type")) {
            // 140f75bc0 clears the destination at each field, including empty lists.
            mask = self.category_value(&entry.value, "add_equipment_type", true)?;
        }
        Ok(mask)
    }

    fn category_value(&self, value: &Value, field: &str, numeric: bool) -> EquipmentResult<u64> {
        let read = |token: &str| {
            if token == "support" {
                return Err(EquipmentDefinitionError::UnknownCategory {
                    token: self.token.clone(), category: token.into(),
                });
            }
            equipment_category_mask(token).or_else(|| numeric.then(|| decimal_category_mask(token)))
                .ok_or_else(|| EquipmentDefinitionError::UnknownCategory {
                    token: self.token.clone(), category: token.into(),
                })
        };
        if let Some(token) = value.as_scalar() { return read(token) }
        let mut mask = 0;
        for entry in value.fields() {
            if entry.key.is_some() { return Err(self.malformed(field)) }
            mask |= read(entry.value.as_scalar().ok_or_else(|| self.malformed(field))?)?;
        }
        Ok(mask)
    }

    fn malformed(&self, field: &str) -> EquipmentDefinitionError {
        EquipmentDefinitionError::MalformedField { token: self.token.clone(), field: field.into() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EquipmentGeneration {
    pub recipe: EquipmentDefinitionIdentity,
    pub original: EquipmentDefinitionIdentity,
    pub authored_recipe: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquipmentDefinitionIdentity {
    pub token: String,
    pub source: UsedContentKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquipmentHierarchyProfile {
    pub definition: EquipmentDefinitionIdentity,
    pub parent: EquipmentDefinitionIdentity,
    pub archetype: Option<EquipmentDefinitionIdentity>,
    pub parent_chain: Vec<EquipmentDefinitionIdentity>,
    pub authored_priority: Option<i32>,
    pub authored_visual_tech_level_addition: Option<i32>,
    pub effective_model_priority: i32,
    pub authored_visual_level: Option<i32>,
    pub effective_visual_level: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquipmentModelProfile {
    pub hierarchy: EquipmentHierarchyProfile,
    pub own_type_mask: u64,
    pub archetype_type_mask: Option<u64>,
    pub additional_type_mask: u64,
    pub normalized_type_mask: u64,
    pub default_modules: Vec<EquipmentDefaultModule>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EquipmentDesignerClassification {
    pub ship: bool,
    pub tank: bool,
    pub plane: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EquipmentDefaultModule {
    Resolved { slot: String, definition: EquipmentDefinitionIdentity, additional_type_mask: u64 },
    Missing { slot: String, token: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredEquipmentSource {
    pub source: UsedContentKey,
    pub authored: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredEquipmentCatalog {
    sources: Vec<AuthoredEquipmentSource>,
    definitions: BTreeMap<String, Vec<AuthoredEquipmentDefinition>>,
    modules: BTreeMap<String, Vec<AuthoredEquipmentDefinition>>,
    active_dlcs: Option<BTreeSet<String>>,
    generation_resolved: bool,
    generation_dlcs: Option<BTreeSet<String>>,
    definition_order: Vec<String>,
}

impl AuthoredEquipmentCatalog {
    pub fn load(resolver: &GameFileResolver) -> EquipmentResult<Self> {
        let lease = resolver.content_admission_lease().ok_or_else(||
            EquipmentDefinitionError::ContentAdmission("equipment definitions require a validated content admission".into()))?;
        Self::from_admission_lease(&lease)
    }

    pub fn from_admission_lease(lease: &ContentAdmissionLease) -> EquipmentResult<Self> {
        let sources = lease.require_complete_admitted_dir(
            "common/units/equipment", Some("txt"), true, "equipment definitions",
        ).map_err(|error| EquipmentDefinitionError::ContentAdmission(error.to_string()))?;
        let mut catalog = Self { definitions: BTreeMap::new(), sources: Vec::new(), modules: BTreeMap::new(), active_dlcs: None, generation_resolved: false, generation_dlcs: None, definition_order: Vec::new() };
        for source in sources {
            let bytes = lease.read_used_content(&source)
                .map_err(|error| EquipmentDefinitionError::ContentAdmission(error.to_string()))?;
            catalog.insert_source(source, &bytes)?;
        }
        Ok(catalog)
    }

    pub fn sources(&self) -> &[AuthoredEquipmentSource] { &self.sources }

    pub fn with_active_dlcs(mut self, active: &ActiveDlcPresentation) -> Self {
        self.active_dlcs = Some(active.display_names().iter().cloned().collect());
        self
    }

    pub fn definition(&self, token: &str) -> EquipmentResult<&AuthoredEquipmentDefinition> {
        self.require_resolution_profile()?;
        let definition = self.admitted_definition(&self.definitions, token);
        if matches!(definition, Err(EquipmentDefinitionError::MissingDefinition(_))
            | Err(EquipmentDefinitionError::DisabledByContentCondition(_))) {
            self.require_resolved_generation(token)?;
        }
        definition
    }

    fn require_resolution_profile(&self) -> EquipmentResult<()> {
        if self.generation_resolved && self.generation_dlcs != self.active_dlcs {
            Err(EquipmentDefinitionError::ContentConditionProfileChanged)
        } else { Ok(()) }
    }

    fn require_resolved_generation(&self, token: &str) -> EquipmentResult<()> {
        self.require_resolution_profile()?;
        if self.generation_resolved { return Ok(()) }
        let sources: Vec<_> = self.sources.iter().filter(|source| {
            source.authored.fields().iter().any(|entry| entry.key_matches("duplicate_archetypes")
                && entry.value.as_object().is_none_or(|entries| !entries.is_empty()))
        }).map(|source| source.source.normalized_game_relative_path.clone()).collect();
        if sources.is_empty() { Ok(()) } else {
            Err(EquipmentDefinitionError::UnresolvedGeneration { token: token.into(), sources })
        }
    }

    pub fn module(&self, token: &str) -> EquipmentResult<&AuthoredEquipmentDefinition> {
        self.require_resolution_profile()?;
        self.admitted_definition(&self.modules, token)
    }

    fn admitted_definition<'a>(&self, definitions: &'a BTreeMap<String, Vec<AuthoredEquipmentDefinition>>,
        token: &str) -> EquipmentResult<&'a AuthoredEquipmentDefinition>
    {
        let definitions = definitions.get(token)
            .ok_or_else(|| EquipmentDefinitionError::MissingDefinition(token.into()))?;
        let mut enabled = Vec::new();
        for definition in definitions {
            if let Some(limit) = &definition.container_limit {
                if !self.content_limit(token, limit)? { continue }
            }
            enabled.push(definition);
        }
        match enabled.as_slice() {
            [] => Err(EquipmentDefinitionError::DisabledByContentCondition(token.into())),
            [definition] => Ok(*definition),
            _ => Err(EquipmentDefinitionError::AmbiguousDefinition {
                token: token.into(),
                sources: enabled.iter().map(|value| value.source.normalized_game_relative_path.clone()).collect(),
            }),
        }
    }

    fn content_limit(&self, token: &str, value: &Value) -> EquipmentResult<bool> {
        let unsupported = || EquipmentDefinitionError::UnsupportedAuthored {
            token: token.into(), field: "limit".into(),
        };
        let [entry] = value.as_object().ok_or_else(unsupported)? else { return Err(unsupported()) };
        if entry.key_matches("has_dlc") {
            let name = entry.value.as_scalar().filter(|name| !name.is_empty()).ok_or_else(unsupported)?;
            let active = self.active_dlcs.as_ref().ok_or(EquipmentDefinitionError::MissingActiveDlcProfile)?;
            Ok(active.contains(name))
        } else if entry.key_matches("NOT") {
            Ok(!self.content_limit(token, &entry.value)?)
        } else {
            Err(unsupported())
        }
    }

    pub fn hierarchy_profile(&self, token: &str) -> EquipmentResult<EquipmentHierarchyProfile> {
        self.require_resolved_generation(token)?;
        let definition = self.definition(token)?;
        let mut chain = Vec::new();
        let mut next = token.to_owned();
        let mut priority = 0i32;
        loop {
            if chain.iter().any(|entry: &EquipmentDefinitionIdentity| entry.token == next) {
                let mut cycle: Vec<_> = chain.iter().map(|entry| entry.token.clone()).collect();
                cycle.push(next);
                return Err(EquipmentDefinitionError::RelationshipCycle(cycle));
            }
            let current = self.definition(&next)?;
            // 140c84830 initializes +3cc to one; 140c85280 adds signed 32-bit values.
            priority = priority.wrapping_add(current.signed_integer("visual_tech_level_addition")?.unwrap_or(1));
            chain.push(current.identity());
            let parent = current.scalar("parent")?.unwrap_or(&current.token);
            if parent == current.token { break }
            next = parent.to_owned();
        }
        Ok(EquipmentHierarchyProfile {
            definition: definition.identity(),
            parent: chain.get(1).unwrap_or(&chain[0]).clone(),
            archetype: self.resolve_archetype(token, &mut Vec::new())?,
            parent_chain: chain,
            authored_priority: definition.signed_integer("priority")?,
            authored_visual_tech_level_addition: definition.signed_integer("visual_tech_level_addition")?,
            effective_model_priority: priority.max(1),
            authored_visual_level: definition.signed_integer("visual_level")?,
            effective_visual_level: definition.signed_integer("visual_level")?.unwrap_or(0),
        })
    }

    pub fn model_profile(&self, token: &str) -> EquipmentResult<EquipmentModelProfile> {
        let hierarchy = self.hierarchy_profile(token)?;
        let own_type_mask = self.definition(token)?.type_mask()?;
        let archetype_type_mask = hierarchy.archetype.as_ref()
            .map(|archetype| self.definition(&archetype.token)?.type_mask()).transpose()?;
        let (additional_type_mask, default_modules) = self.default_module_profile(token)?;
        let normalized_type_mask = crate::equipment_model_law::equipment_model_category(
            own_type_mask, archetype_type_mask, additional_type_mask,
        ).map_err(|_| EquipmentDefinitionError::MissingArchetypeCategory(token.into()))?;
        Ok(EquipmentModelProfile { hierarchy, own_type_mask, archetype_type_mask,
            additional_type_mask, normalized_type_mask, default_modules })
    }

    fn default_module_profile(&self, token: &str) -> EquipmentResult<(u64, Vec<EquipmentDefaultModule>)> {
        let mut slots = Vec::<(String, String)>::new();
        let definition = self.definition(token)?;
        for entry in definition.expanded.fields().iter().filter(|entry| entry.key_matches("default_modules")) {
            for assignment in entry.value.as_object().ok_or_else(|| definition.malformed("default_modules"))? {
                let slot = assignment.key.as_ref().ok_or_else(|| definition.malformed("default_modules"))?;
                let module = assignment.value.as_scalar().ok_or_else(|| definition.malformed("default_modules"))?;
                if let Some(existing) = slots.iter_mut().find(|(name, _)| name == slot) {
                    existing.1 = module.into();
                } else {
                    slots.push((slot.clone(), module.into()));
                }
            }
        }
        let mut additional_type_mask = 0;
        let mut default_modules = Vec::new();
        for (slot, token) in slots {
            match self.module(&token) {
                Ok(module) => {
                    let mask = module.additional_type_mask()?;
                    additional_type_mask |= mask;
                    default_modules.push(EquipmentDefaultModule::Resolved {
                        slot, definition: module.identity(), additional_type_mask: mask,
                    });
                }
                Err(EquipmentDefinitionError::MissingDefinition(_))
                | Err(EquipmentDefinitionError::DisabledByContentCondition(_)) => {
                    default_modules.push(EquipmentDefaultModule::Missing { slot, token });
                }
                Err(error) => return Err(error),
            }
        }
        Ok((additional_type_mask, default_modules))
    }

    pub fn designer_category_mask(&self, token: &str) -> EquipmentResult<u64> {
        self.require_resolved_generation(token)?;
        let own = self.definition(token)?.type_mask()?;
        let archetype = if own == 0 {
            self.resolve_archetype(token, &mut Vec::new())?
                .map(|row| self.definition(&row.token)?.type_mask()).transpose()?
        } else { None };
        let (additional, _) = self.default_module_profile(token)?;
        crate::equipment_model_law::equipment_model_category(own, archetype, additional)
            .map_err(|_| EquipmentDefinitionError::MissingArchetypeCategory(token.into()))
    }

    pub fn designer_classification(&self, token: &str) -> EquipmentResult<EquipmentDesignerClassification> {
        self.classify_designer_definition(token, &mut Vec::new())
    }

    fn classify_designer_definition(&self, token: &str, visiting: &mut Vec<String>)
        -> EquipmentResult<EquipmentDesignerClassification>
    {
        if visiting.iter().any(|value| value == token) {
            let mut cycle = visiting.clone(); cycle.push(token.into());
            return Err(EquipmentDefinitionError::RelationshipCycle(cycle));
        }
        visiting.push(token.into());
        self.require_resolved_generation(token)?;
        self.require_resolution_profile()?;
        let archetype = self.resolve_archetype(token, &mut Vec::new())?;
        let inherited = match archetype.as_ref() {
            Some(archetype) if archetype.token != token =>
                self.classify_designer_definition(&archetype.token, visiting)?,
            _ => EquipmentDesignerClassification::default(),
        };
        if inherited.ship && inherited.tank && inherited.plane {
            visiting.pop();
            return Ok(inherited);
        }
        let own_mask = self.definition(token)?.type_mask()?;
        let archetype_mask = archetype.as_ref().map(|row| self.definition(&row.token)?.type_mask()).transpose()?;
        let (additional_mask, _) = self.default_module_profile(token)?;
        let mask = crate::equipment_model_law::equipment_model_category(own_mask, archetype_mask, additional_mask)
            .map_err(|_| EquipmentDefinitionError::MissingArchetypeCategory(token.into()))?;
        visiting.pop();
        // Each engine classifier admits its archetype before testing the current mask.
        Ok(EquipmentDesignerClassification {
            ship: inherited.ship || mask & 0x80004003c1 != 0,
            tank: inherited.tank || mask & 0x408000003c != 0,
            plane: inherited.plane || mask & 0x1f0037fc00 != 0,
        })
    }

    fn resolve_archetype(&self, token: &str, visiting: &mut Vec<String>)
        -> EquipmentResult<Option<EquipmentDefinitionIdentity>>
    {
        if visiting.iter().any(|value| value == token) {
            let mut cycle = visiting.clone();
            cycle.push(token.into());
            return Err(EquipmentDefinitionError::RelationshipCycle(cycle));
        }
        visiting.push(token.into());
        let definition = self.definition(token)?;
        let mut archetype = None;
        // The loader assigns these pointers as authored fields are encountered.
        for entry in definition.expanded.fields() {
            if entry.key_matches("archetype") {
                let target = entry.value.as_scalar().ok_or_else(|| definition.malformed("archetype"))?;
                let target = self.definition(target)?;
                if target.boolean("is_archetype")? != Some(true) {
                    return Err(EquipmentDefinitionError::NotArchetype(target.token.clone()));
                }
                archetype = Some(target.identity());
            } else if entry.key_matches("is_archetype") {
                match entry.value.as_scalar() {
                    Some("yes") => archetype = Some(definition.identity()),
                    Some("no") => {},
                    _ => return Err(definition.malformed("is_archetype")),
                }
            } else if entry.key_matches("parent") {
                let parent = entry.value.as_scalar().ok_or_else(|| definition.malformed("parent"))?;
                self.definition(parent)?;
                if archetype.is_none() && parent != token {
                    archetype = self.resolve_archetype(parent, visiting)?;
                }
            }
        }
        visiting.pop();
        Ok(archetype)
    }

    fn insert_source(&mut self, source: UsedContentKey, bytes: &[u8]) -> EquipmentResult<()> {
        let malformed = |detail: String| EquipmentDefinitionError::MalformedContent {
            source: source.normalized_game_relative_path.clone(), detail,
        };
        let text = paradox_text::decode_text_strict(bytes).map_err(|error| malformed(error.to_string()))?;
        let root = paradox_text::parse_str_strict(&text).map_err(|error| malformed(error.message))?;
        let mut expanded = root.clone();
        paradox_text::expand_file_scoped_variables(&mut expanded).map_err(|error| {
            EquipmentDefinitionError::VariableExpansion { source: source.normalized_game_relative_path.clone(), detail: error.to_string() }
        })?;
        for (entry, expanded_entry) in root.fields().iter().zip(expanded.fields()) {
            let Some(key) = entry.key.as_deref() else { continue };
            if key.starts_with('@') { continue }
            let modules = entry.key_matches("equipment_modules");
            if !entry.key_matches("equipments") && !modules { continue }
            let entries = entry.value.as_object().ok_or_else(|| malformed("equipments is not an object".into()))?;
            let limit = expanded_entry.value.first_value_ci("limit").cloned();
            for (definition, expanded_definition) in entries.iter().zip(expanded_entry.value.fields()) {
                let token = definition.key.as_deref().ok_or_else(|| malformed("equipment definition has no token".into()))?;
                if token.starts_with('@') || definition.key_matches("limit") { continue }
                if definition.value.as_object().is_none() {
                    return Err(malformed(format!("equipment {token:?} is not an object")));
                }
                let definitions = if modules { &mut self.modules } else { &mut self.definitions };
                definitions.entry(token.into()).or_default().push(AuthoredEquipmentDefinition {
                    token: token.into(), source: source.clone(), authored: definition.value.clone(),
                    generation: None, generated_type_mask: None, resolved_module_slots: None,
                    expanded: expanded_definition.value.clone(), container_limit: limit.clone(),
                });
            }
        }
        self.sources.push(AuthoredEquipmentSource { source, authored: root });
        Ok(())
    }
}

fn equipment_category_mask(token: &str) -> Option<u64> {
    Some(match token {
        "convoy" => 0x1, "infantry" => 0x20, "artillery" => 0x10000000,
        "armor" => 0x4, "amphibious" => 0x4000000, "anti_air" => 0x8000000,
        "anti_tank" => 0x20000000, "rocket" => 0x40000000, "flame" => 0x2000000,
        "mechanized" => 0x10, "motorized" => 0x8, "capital_ship" => 0x40,
        "submarine" => 0x80, "carrier" => 0x400000, "fighter" => 0x400,
        "cas" => 0x4000, "naval_bomber" => 0x8000, "interceptor" => 0x800,
        "tactical_bomber" => 0x1000, "strategic_bomber" => 0x2000,
        "air_transport" => 0x100000, "maritime_patrol_plane" => 0x200000,
        "heavy_fighter" => 0x100000000, "land_cruiser" => 0x4000000000,
        "emplacement_gun_ammo" => 0x200000000, "ballistic_missile" => 0x400000000,
        "nuclear_missile" => 0x800000000, "sam_missile" => 0x1000000000,
        "screen_ship" => 0x100, "support_ship" => 0x8000000000, "scout_plane" => 0x40000,
        "floating_harbor" => 0x200, "railway_gun" => 0x80000, "suicide" => 0x20000,
        "train" => 0x80000000, "missile_launcher" => 0x2000000000, "missile" => 0x10000,
        _ => return None,
    })
}

fn decimal_category_mask(token: &str) -> u64 {
    // 1424b00c0 accepts the decimal prefix; 14253d1fc saturates unsigned overflow.
    let bytes = token.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']).as_bytes();
    let negative = bytes.first() == Some(&b'-');
    let bytes = if matches!(bytes.first(), Some(b'+' | b'-')) { &bytes[1..] } else { bytes };
    let mut value = 0u64;
    let mut overflow = false;
    for &byte in bytes.iter().take_while(|byte| byte.is_ascii_digit()) {
        match value.checked_mul(10).and_then(|value| value.checked_add(u64::from(byte - b'0'))) {
            Some(next) => value = next,
            None => overflow = true,
        }
    }
    if overflow { u64::MAX } else if negative { value.wrapping_neg() } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;

    fn source(path: &str) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "1".repeat(64),
            stored_game_relative_path: path.into(), normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 0, layer_kind: ContentLayerKind::Base, layer_id: "base".into(),
            source_sha256: "2".repeat(64), payload_sha256: "2".repeat(64), payload_bytes: 0,
        }
    }

    pub(super) fn catalog(text: &str) -> AuthoredEquipmentCatalog {
        let mut catalog = AuthoredEquipmentCatalog { definitions: BTreeMap::new(), sources: Vec::new(), modules: BTreeMap::new(), active_dlcs: None, generation_resolved: false, generation_dlcs: None, definition_order: Vec::new() };
        catalog.insert_source(source("common/units/equipment/a.txt"), text.as_bytes()).unwrap();
        catalog
    }

    #[test]
    fn retains_unconsumed_fields_and_distinguishes_absent_zero_and_invalid() {
        let catalog = catalog(r#"equipments = {
            hull = { visual_level = 0 other_mod_field = { value = arbitrary } }
            unknown = { visual_level = { no = scalar } }
        }"#);
        let hull = catalog.definition("hull").unwrap();
        assert_eq!(hull.signed_integer("visual_level").unwrap(), Some(0));
        assert_eq!(hull.signed_integer("unwritten").unwrap(), None);
        assert_eq!(hull.field("other_mod_field").unwrap().first_scalar_ci("value"), Some("arbitrary"));
        assert!(matches!(catalog.definition("unknown").unwrap().signed_integer("visual_level"),
            Err(EquipmentDefinitionError::MalformedField { .. })));
        assert!(matches!(catalog.definition("missing"), Err(EquipmentDefinitionError::MissingDefinition(_))));
    }

    #[test]
    fn repeated_definition_is_named_ambiguous_and_retains_both_sources() {
        let mut catalog = catalog("equipments = { hull = { visual_level = 1 } }");
        catalog.insert_source(source("common/units/equipment/b.txt"),
            b"equipments = { hull = { visual_level = 2 } }").unwrap();
        assert_eq!(catalog.definition("hull").unwrap_err(), EquipmentDefinitionError::AmbiguousDefinition {
            token: "hull".into(), sources: vec!["common/units/equipment/a.txt".into(), "common/units/equipment/b.txt".into()],
        });
    }

    #[test]
    fn signed_fields_preserve_extrema_and_refuse_overflow() {
        let catalog = catalog("equipments = { hull = { low = -2147483648 high = 2147483647 overflow = 2147483648 } }");
        let hull = catalog.definition("hull").unwrap();
        assert_eq!(hull.signed_integer("low").unwrap(), Some(i32::MIN));
        assert_eq!(hull.signed_integer("high").unwrap(), Some(i32::MAX));
        assert!(hull.signed_integer("overflow").is_err());
    }

    #[test]
    fn malformed_source_is_not_an_empty_catalogue() {
        let mut catalog = AuthoredEquipmentCatalog { definitions: BTreeMap::new(), sources: Vec::new(), modules: BTreeMap::new(), active_dlcs: None, generation_resolved: false, generation_dlcs: None, definition_order: Vec::new() };
        assert!(matches!(catalog.insert_source(source("common/units/equipment/a.txt"),
            b"equipments = { hull = { visual_level ="), Err(EquipmentDefinitionError::MalformedContent { .. })));
    }

    #[test]
    fn shared_variable_expansion_preserves_quoted_literals_and_reports_unknowns() {
        let catalog = catalog("@level = -2 equipments = { hull = { visual_level = @level literal = \"@level\" } }");
        let hull = catalog.definition("hull").unwrap();
        assert_eq!(hull.signed_integer("visual_level").unwrap(), Some(-2));
        assert_eq!(hull.scalar("literal").unwrap(), Some("@level"));
        assert_eq!(hull.field("visual_level").unwrap().as_scalar(), Some("@level"));
        let mut catalog = AuthoredEquipmentCatalog { definitions: BTreeMap::new(), sources: Vec::new(), modules: BTreeMap::new(), active_dlcs: None, generation_resolved: false, generation_dlcs: None, definition_order: Vec::new() };
        assert!(matches!(catalog.insert_source(source("common/units/equipment/a.txt"),
            b"equipments = { hull = { value = @absent } }"), Err(EquipmentDefinitionError::VariableExpansion { .. })));
    }

    #[test]
    fn retains_generated_roots_and_does_not_admit_unevaluated_limits() {
        let catalog = catalog(r#"equipments = { limit = { has_dlc = "DLC" } hull = { type = armor } }
            duplicate_archetypes = { derived = { archetype = hull for_each = { x = { set = 2 } } } }"#);
        assert_eq!(catalog.definition("hull").unwrap_err(), EquipmentDefinitionError::MissingActiveDlcProfile);
        assert!(catalog.sources()[0].authored.first_value_ci("duplicate_archetypes").is_some());
        assert!(matches!(catalog.definition("limit"), Err(EquipmentDefinitionError::UnresolvedGeneration { .. })));
    }
    #[test]
    fn hierarchy_preserves_raw_priority_and_sums_only_visual_tech_additions() {
        let catalog = catalog("equipments = { root = { is_archetype = yes priority = 900 visual_tech_level_addition = 4 } mid = { archetype = root parent = root visual_tech_level_addition = -2 } leaf = { parent = mid priority = -700 visual_level = 0 } }");
        let profile = catalog.hierarchy_profile("leaf").unwrap();
        assert_eq!(profile.parent_chain.iter().map(|v| v.token.as_str()).collect::<Vec<_>>(), ["leaf", "mid", "root"]);
        assert_eq!(profile.parent.token, "mid");
        assert_eq!(profile.archetype.unwrap().token, "root");
        assert_eq!(profile.authored_priority, Some(-700));
        assert_eq!(profile.authored_visual_tech_level_addition, None);
        assert_eq!(profile.effective_model_priority, 3);
        assert_eq!(profile.authored_visual_level, Some(0));
        assert_eq!(profile.definition.source, source("common/units/equipment/a.txt"));
    }

    #[test]
    fn model_priority_wraps_as_signed_engine_addition_then_clamps() {
        let catalog = catalog("equipments = { root = { is_archetype = yes visual_tech_level_addition = 2147483647 } leaf = { parent = root visual_tech_level_addition = 1 } negative = { visual_tech_level_addition = -2 } zero = { visual_tech_level_addition = 0 } }");
        assert_eq!(catalog.hierarchy_profile("leaf").unwrap().effective_model_priority, 1);
        assert_eq!(catalog.hierarchy_profile("negative").unwrap().effective_model_priority, 1);
        let zero = catalog.hierarchy_profile("zero").unwrap();
        assert_eq!(zero.authored_visual_tech_level_addition, Some(0));
        assert_eq!(zero.effective_model_priority, 1);
        assert_eq!(zero.archetype, None);
        assert_eq!(zero.authored_visual_level, None);
        assert_eq!(zero.effective_visual_level, 0);
    }

    #[test]
    fn relationship_missing_malformed_and_cycle_are_distinct() {
        let catalog = catalog("equipments = { a = { parent = b } b = { parent = a } lost = { parent = missing } malformed = { parent = {} } nonroot = {} wrong = { archetype = nonroot } }");
        assert_eq!(catalog.hierarchy_profile("a").unwrap_err(), EquipmentDefinitionError::RelationshipCycle(vec!["a".into(), "b".into(), "a".into()]));
        assert_eq!(catalog.hierarchy_profile("lost").unwrap_err(), EquipmentDefinitionError::MissingDefinition("missing".into()));
        assert!(matches!(catalog.hierarchy_profile("malformed"), Err(EquipmentDefinitionError::MalformedField { .. })));
        assert_eq!(catalog.hierarchy_profile("wrong").unwrap_err(), EquipmentDefinitionError::NotArchetype("nonroot".into()));
    }

    #[test]
    fn archetype_assignments_follow_authored_order_and_parent_only_fills_unknown() {
        let catalog = catalog("equipments = { a = { is_archetype = yes } b = { is_archetype = yes } p = { archetype = a } first = { parent = p archetype = b } second = { archetype = b parent = p } self_root = { archetype = a is_archetype = yes } }");
        assert_eq!(catalog.hierarchy_profile("first").unwrap().archetype.unwrap().token, "b");
        assert_eq!(catalog.hierarchy_profile("second").unwrap().archetype.unwrap().token, "b");
        assert_eq!(catalog.hierarchy_profile("self_root").unwrap().archetype.unwrap().token, "self_root");
    }

    #[test]
    fn module_dlc_limit_keeps_unknown_disabled_and_measured_empty_profile_distinct() {
        let mut catalog = catalog(r#"equipment_modules = { limit = { has_dlc = "Pack" } engine = { add_equipment_type = fighter mod_field = yes } }
            equipment_modules = { limit = { NOT = { has_dlc = "Pack" } } fallback = {} }"#);
        assert_eq!(catalog.module("engine").unwrap_err(), EquipmentDefinitionError::MissingActiveDlcProfile);
        catalog.active_dlcs = Some(BTreeSet::new());
        assert_eq!(catalog.module("engine").unwrap_err(), EquipmentDefinitionError::DisabledByContentCondition("engine".into()));
        assert!(catalog.module("fallback").is_ok());
        catalog.active_dlcs = Some(BTreeSet::from(["Pack".into()]));
        assert_eq!(catalog.module("engine").unwrap().scalar("mod_field").unwrap(), Some("yes"));
        assert_eq!(catalog.module("fallback").unwrap_err(), EquipmentDefinitionError::DisabledByContentCondition("fallback".into()));
        assert!(matches!(catalog.definition("engine"), Err(EquipmentDefinitionError::MissingDefinition(_))));
    }

    #[test]
    fn partial_dlc_trigger_is_not_mistaken_for_complete_condition() {
        let mut catalog = catalog(r#"equipment_modules = { limit = { has_dlc = "Pack" has_idea = mutable } engine = {} }"#);
        catalog.active_dlcs = Some(BTreeSet::from(["Pack".into()]));
        assert!(matches!(catalog.module("engine"), Err(EquipmentDefinitionError::UnsupportedAuthored { .. })));
    }

    #[test]
    fn complementary_dlc_containers_resolve_one_same_token_before_ambiguity() {
        let mut catalog = catalog(r#"equipment_modules = { limit = { has_dlc = "Pack" } engine = { add_equipment_type = fighter } }
            equipment_modules = { limit = { NOT = { has_dlc = "Pack" } } engine = { add_equipment_type = cas } }"#);
        assert_eq!(catalog.module("engine").unwrap_err(), EquipmentDefinitionError::MissingActiveDlcProfile);
        catalog.active_dlcs = Some(BTreeSet::new());
        assert_eq!(catalog.module("engine").unwrap().additional_type_mask().unwrap(), 0x4000);
        catalog.active_dlcs = Some(BTreeSet::from(["Pack".into()]));
        assert_eq!(catalog.module("engine").unwrap().additional_type_mask().unwrap(), 0x400);
        catalog.insert_source(source("common/units/equipment/other.txt"), b"equipment_modules = { engine = {} }").unwrap();
        assert!(matches!(catalog.module("engine"), Err(EquipmentDefinitionError::AmbiguousDefinition { .. })));
        let mut refused = self::catalog("equipment_modules = { engine = {} } equipment_modules = { limit = { has_idea = mutable } engine = {} }");
        refused.active_dlcs = Some(BTreeSet::new());
        assert!(matches!(refused.module("engine"), Err(EquipmentDefinitionError::UnsupportedAuthored { .. })));
    }

    #[test]
    fn model_profile_retains_raw_masks_and_uses_shared_normalization() {
        let catalog = catalog("equipments = { root = { is_archetype = yes type = { convoy screen_ship } } inherited = { archetype = root default_modules = { engine = naval engine = capital extra = absent } } own = { type = support_ship } zero = { is_archetype = yes type = {} } no_archetype = {} } equipment_modules = { naval = { add_equipment_type = submarine } capital = { add_equipment_type = capital_ship } }");
        let inherited = catalog.model_profile("inherited").unwrap();
        assert_eq!((inherited.own_type_mask, inherited.archetype_type_mask, inherited.additional_type_mask, inherited.normalized_type_mask), (0, Some(0x101), 0x40, 0x40));
        assert!(matches!(&inherited.default_modules[0], EquipmentDefaultModule::Resolved { slot, definition, additional_type_mask: 0x40 } if slot == "engine" && definition.token == "capital"));
        assert_eq!(inherited.default_modules[1], EquipmentDefaultModule::Missing { slot: "extra".into(), token: "absent".into() });
        let own = catalog.model_profile("own").unwrap();
        assert_eq!((own.own_type_mask, own.archetype_type_mask, own.normalized_type_mask), (0x8000000000, None, 0x8000000000));
        assert_eq!(catalog.model_profile("zero").unwrap().normalized_type_mask, 0);
        assert_eq!(catalog.model_profile("no_archetype").unwrap_err(), EquipmentDefinitionError::MissingArchetypeCategory("no_archetype".into()));
    }

    #[test]
    fn category_fields_preserve_lists_repetition_empty_and_full_unsigned_width() {
        let catalog = catalog("equipments = { e = { type = { armor fighter } type = naval_bomber } invalid = { type = 4 } malformed = { type = { armor = yes } } } equipment_modules = { high = { add_equipment_type = { 9223372036854775808 fighter } } replace = { add_equipment_type = armor add_equipment_type = {} } repeated = { add_equipment_type = armor add_equipment_type = fighter } }");
        assert_eq!(catalog.definition("e").unwrap().type_mask().unwrap(), 0x8404);
        assert!(matches!(catalog.definition("invalid").unwrap().type_mask(), Err(EquipmentDefinitionError::UnknownCategory { .. })));
        assert!(matches!(catalog.definition("malformed").unwrap().type_mask(), Err(EquipmentDefinitionError::MalformedField { .. })));
        assert_eq!(catalog.module("high").unwrap().additional_type_mask().unwrap(), (1u64 << 63) | 0x400);
        assert_eq!(catalog.module("replace").unwrap().additional_type_mask().unwrap(), 0);
        assert_eq!(catalog.module("repeated").unwrap().additional_type_mask().unwrap(), 0x400);
        for (text, expected) in [("-1", u64::MAX), ("18446744073709551615", u64::MAX),
            ("18446744073709551616", u64::MAX), ("-18446744073709551616", u64::MAX),
            ("+32tail", 32), ("  8", 8), ("nonnumeric", 0), ("0x80", 0)] {
            assert_eq!(decimal_category_mask(text), expected, "{text}");
        }
    }

    #[test]
    fn unexpanded_generations_are_not_proven_missing_candidates_or_complete_profiles() {
        let catalog = catalog("equipments = { hull = { is_archetype = yes type = armor default_modules = { engine = missing } } } duplicate_archetypes = { generated = { archetype = hull type = fighter for_each = { hardness = { set = 0.5 } } } }");
        let refusal = |token: &str| EquipmentDefinitionError::UnresolvedGeneration {
            token: token.into(), sources: vec!["common/units/equipment/a.txt".into()],
        };
        assert_eq!(catalog.definition("generated").unwrap_err(), refusal("generated"));
        assert_eq!(catalog.definition("not_observed").unwrap_err(), refusal("not_observed"));
        assert_eq!(catalog.hierarchy_profile("hull").unwrap_err(), refusal("hull"));
        assert_eq!(catalog.model_profile("hull").unwrap_err(), refusal("hull"));
        assert_eq!(catalog.definition("hull").unwrap().type_mask().unwrap(), 4);
        assert_eq!(catalog.module("missing").unwrap_err(), EquipmentDefinitionError::MissingDefinition("missing".into()));
    }

    #[test]
    fn empty_generation_root_allows_absence_but_unparsed_root_does_not() {
        let empty = catalog("equipments = { hull = { type = armor } } duplicate_archetypes = {}");
        assert_eq!(empty.definition("missing").unwrap_err(), EquipmentDefinitionError::MissingDefinition("missing".into()));
        assert_eq!(empty.model_profile("hull").unwrap().normalized_type_mask, 4);
        let unparsed = catalog("duplicate_archetypes = malformed");
        assert!(matches!(unparsed.definition("missing"), Err(EquipmentDefinitionError::UnresolvedGeneration { .. })));
        let mut disabled = catalog(r#"equipments = { limit = { has_dlc = "Pack" } hull = {} } duplicate_archetypes = { hull = { archetype = source } }"#);
        disabled.active_dlcs = Some(BTreeSet::new());
        assert!(matches!(disabled.definition("hull"), Err(EquipmentDefinitionError::UnresolvedGeneration { .. })));
    }


    #[test]
    fn ordered_generation_remaps_hierarchy_copies_profile_and_retains_provenance() {
        let catalog = catalog(r#"
            equipments = {
                hull = { is_archetype = yes type = armor visual_tech_level_addition = 2 }
                hull_2 = { parent = hull_1 visual_tech_level_addition = -1 visual_level = 7 }
                hull_1 = { archetype = hull parent = hull visual_tech_level_addition = 3 default_modules = { engine = engine } }
            }
            equipment_modules = { engine = { add_equipment_type = heavy_fighter } }
            duplicate_archetypes = { aircraft = { archetype = hull type = fighter } }
        "#).resolve_generated().unwrap();
        let child = catalog.model_profile("aircraft_2").unwrap();
        assert_eq!(child.hierarchy.parent.token, "aircraft_1");
        assert_eq!(child.hierarchy.archetype.as_ref().unwrap().token, "aircraft");
        assert_eq!(child.hierarchy.parent_chain.iter().map(|x| x.token.as_str()).collect::<Vec<_>>(), ["aircraft_2", "aircraft_1", "aircraft"]);
        assert_eq!(child.hierarchy.effective_model_priority, 4);
        assert_eq!(child.hierarchy.effective_visual_level, 7);
        assert_eq!(child.own_type_mask, 0x400);
        assert_eq!(child.archetype_type_mask, Some(0x400));
        assert_eq!(catalog.model_profile("aircraft_1").unwrap().additional_type_mask, 0x100000000);
        let definition = catalog.definition("aircraft_2").unwrap();
        let provenance = definition.generation.as_ref().unwrap();
        assert_eq!(provenance.original.token, "hull_2");
        assert_eq!(provenance.recipe.token, "aircraft");
        assert_eq!(definition.field("parent").unwrap().as_scalar(), Some("hull_1"));
        assert_eq!(definition.scalar("parent").unwrap(), Some("aircraft_1"));
        assert_eq!(catalog.definition("absent").unwrap_err(), EquipmentDefinitionError::MissingDefinition("absent".into()));
    }

    #[test]
    fn generation_observes_block_order_and_excludes_existing_duplicates() {
        let text = r#"
            equipments = { root = { is_archetype = yes type = armor } root_1 = { archetype = root parent = root visual_level = 2 } }
            duplicate_archetypes = { first = { archetype = root type = fighter } }
            equipments = { root_1 = { archetype = root parent = root visual_level = 8 } }
            duplicate_archetypes = { second = { archetype = root type = cas } third = { archetype = first type = naval_bomber } }
        "#;
        let catalog = catalog(text).resolve_generated().unwrap();
        assert_eq!(catalog.model_profile("first_1").unwrap().hierarchy.effective_visual_level, 2);
        assert_eq!(catalog.model_profile("second_1").unwrap().hierarchy.effective_visual_level, 8);
        assert!(matches!(catalog.definition("third"), Err(EquipmentDefinitionError::MissingDefinition(_))));
        let before = super::tests::catalog("duplicate_archetypes = { new = { archetype = later } } equipments = { later = { is_archetype = yes } }");
        assert!(matches!(before.resolve_generated(), Err(EquipmentDefinitionError::MissingDefinition(token)) if token == "later"));
    }

    #[test]
    fn generation_only_archetype_clears_modules_and_repeated_recipe_fields_merge() {
        let catalog = catalog(r#"
            equipment_modules = { wing = { add_equipment_type = heavy_fighter } }
            equipments = { base = { is_archetype = yes type = armor default_modules = { engine = wing } } base_1 = { archetype = base parent = base } }
            duplicate_archetypes = { new = { archetype = base type = cas only_duplicate_archetype = yes } new = { module_slots = none type = fighter } }
        "#).resolve_generated().unwrap();
        let model = catalog.model_profile("new").unwrap();
        assert_eq!(model.own_type_mask, 0x400);
        assert_eq!(model.additional_type_mask, 0);
        assert!(model.default_modules.is_empty());
        assert_eq!(model.hierarchy.parent.token, "new");
        assert_eq!(model.hierarchy.archetype.unwrap().token, "new");
        assert!(matches!(catalog.definition("new_1"), Err(EquipmentDefinitionError::MissingDefinition(_))));
        assert_eq!(catalog.model_profile("base").unwrap().additional_type_mask, 0x100000000);
    }

    #[test]
    fn for_each_last_operation_and_first_substring_then_explicit_name_override() {
        let catalog = catalog(r#"
            equipments = { chassis = { is_archetype = yes type = armor air_superiority = 2 } chassis_chassis = { archetype = chassis parent = chassis air_superiority = 3 } chassis_2 = { archetype = chassis parent = chassis } }
            duplicate_archetypes = { gun_chassis = {
                archetype = chassis type = artillery
                for_each = { air_superiority = { set = 9 } variant_name = { find_and_replace = { "chassis" "equipment" } } }
                for_each = { air_superiority = { multiply = -0.500009 } }
                variant_name = { gun_chassis_2 = named_variant }
            } }
        "#).resolve_generated().unwrap();
        assert_eq!(catalog.definition("gun_chassis").unwrap().scalar("air_superiority").unwrap(), Some("-1.00000"));
        assert_eq!(catalog.definition("gun_chassis_chassis").unwrap().scalar("air_superiority").unwrap(), Some("-1.50000"));
        assert_eq!(catalog.definition("gun_chassis_chassis").unwrap().scalar("variant_name").unwrap(), Some("gun_equipment_chassis"));
        assert_eq!(catalog.definition("gun_chassis_2").unwrap().scalar("variant_name").unwrap(), Some("named_variant"));
        assert_eq!(catalog.definition("gun_chassis").unwrap().scalar("variant_name").unwrap(), None);
        assert_eq!(catalog.definition("chassis_chassis").unwrap().scalar("air_superiority").unwrap(), Some("3.00000"));
    }

    #[test]
    fn generated_closure_keeps_unknowns_cycles_collisions_and_dlc_rebinding_explicit() {
        for (text, field) in [
            ("equipments = { base = { is_archetype = yes } } duplicate_archetypes = { new = { archetype = base for_each = { priority = { set = 9 } } } }", "for_each.priority.lexical_stat"),
            ("equipments = { base = { is_archetype = yes } new = {} } duplicate_archetypes = { new = { archetype = base } }", "duplicate_archetypes.existing_definition_mode"),
        ] {
            assert!(matches!(catalog(text).resolve_generated(), Err(EquipmentDefinitionError::UnsupportedAuthored { field: actual, .. }) if actual == field));
        }
        assert!(matches!(catalog("equipments = { base = { is_archetype = yes parent = child } child = { parent = base archetype = base } } duplicate_archetypes = { new = { archetype = base } }").resolve_generated(), Err(EquipmentDefinitionError::RelationshipCycle(_))));
        let text = r#"equipments = { limit = { has_dlc = "Pack" } base = { is_archetype = yes type = fighter } } equipments = { limit = { NOT = { has_dlc = "Pack" } } base = { is_archetype = yes type = armor } } duplicate_archetypes = { new = { archetype = base } }"#;
        assert!(matches!(catalog(text).resolve_generated(), Err(EquipmentDefinitionError::MissingActiveDlcProfile)));
        let mut content = catalog(text);
        content.active_dlcs = Some(BTreeSet::new());
        let mut content = content.resolve_generated().unwrap();
        assert_eq!(content.model_profile("base").unwrap().own_type_mask, 4);
        content.active_dlcs = Some(BTreeSet::from(["Pack".into()]));
        assert_eq!(content.definition("base").unwrap_err(), EquipmentDefinitionError::ContentConditionProfileChanged);
        let content = content.resolve_generated().unwrap();
        assert_eq!(content.model_profile("base").unwrap().own_type_mask, 0x400);
    }

    #[test]
    fn designer_families_preserve_archetype_and_module_categories_without_exclusivity() {
        let catalog = catalog("equipments = { root = { is_archetype = yes type = armor } child = { archetype = root type = fighter default_modules = { engine = capital } } empty = { is_archetype = yes } } equipment_modules = { capital = { add_equipment_type = capital_ship } }");
        assert_eq!(catalog.designer_classification("root").unwrap(), EquipmentDesignerClassification { tank:true, ship:false, plane:false });
        assert_eq!(catalog.designer_classification("child").unwrap(), EquipmentDesignerClassification { tank:true, ship:true, plane:true });
        assert_eq!(catalog.designer_classification("empty").unwrap(), EquipmentDesignerClassification::default());
        assert!(matches!(catalog.designer_classification("missing"), Err(EquipmentDefinitionError::MissingDefinition(_))));
    }

    #[test]
    fn designer_classification_requires_generated_closure_and_respects_generated_masks() {
        let authored = catalog("equipments = { root = { is_archetype = yes type = armor } child = { archetype = root } } duplicate_archetypes = { copied = { archetype = root type = fighter } }");
        assert!(matches!(authored.designer_classification("root"), Err(EquipmentDefinitionError::UnresolvedGeneration { .. })));
        let resolved = authored.resolve_generated().unwrap();
        assert_eq!(resolved.designer_classification("copied").unwrap(), EquipmentDesignerClassification { plane:true, ship:false, tank:false });
        assert_eq!(resolved.designer_classification("copiedchild").unwrap(), EquipmentDesignerClassification { plane:true, ship:false, tank:false });
        let cyclic = catalog("equipments = { a = { is_archetype = yes archetype = b } b = { is_archetype = yes archetype = a } }");
        assert!(matches!(cyclic.designer_classification("a"), Err(EquipmentDefinitionError::RelationshipCycle(_))));
    }

}
