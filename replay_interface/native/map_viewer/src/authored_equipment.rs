use crate::{ActiveDlcPresentation, GameFileResolver};
use portable_content_manifest::paradox_text::{self, Value};
use replay_core::UsedContentKey;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EquipmentDefinitionError {
    ContentAdmission(String),
    MalformedContent { source: String, detail: String },
    MissingDefinition(String),
    AmbiguousDefinition { token: String, sources: Vec<String> },
    MalformedField { token: String, field: String },
    MissingField { token: String, field: String },
    VariableExpansion { source: String, detail: String },
    UnsupportedAuthored { token: String, field: String },
    RelationshipCycle(Vec<String>),
    NotArchetype(String),
    MissingActiveDlcProfile,
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
    expanded: Value,
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

    fn malformed(&self, field: &str) -> EquipmentDefinitionError {
        EquipmentDefinitionError::MalformedField { token: self.token.clone(), field: field.into() }
    }
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
}

impl AuthoredEquipmentCatalog {
    pub fn load(resolver: &GameFileResolver) -> EquipmentResult<Self> {
        let sources = resolver.require_complete_admitted_dir(
            "common/units/equipment", Some("txt"), true, "equipment definitions",
        ).map_err(|error| EquipmentDefinitionError::ContentAdmission(error.to_string()))?;
        let mut catalog = Self { definitions: BTreeMap::new(), sources: Vec::new(), modules: BTreeMap::new(), active_dlcs: None };
        for source in sources {
            let bytes = resolver.read_admitted_used_content(&source)
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
        self.admitted_definition(&self.definitions, token)
    }

    pub fn module(&self, token: &str) -> EquipmentResult<&AuthoredEquipmentDefinition> {
        self.admitted_definition(&self.modules, token)
    }

    fn admitted_definition<'a>(&self, definitions: &'a BTreeMap<String, Vec<AuthoredEquipmentDefinition>>,
        token: &str) -> EquipmentResult<&'a AuthoredEquipmentDefinition>
    {
        match definitions.get(token).map(Vec::as_slice) {
            Some([definition]) => {
                if let Some(limit) = &definition.container_limit {
                    if !self.content_limit(token, limit)? {
                        return Err(EquipmentDefinitionError::DisabledByContentCondition(token.into()));
                    }
                }
                Ok(definition)
            },
            Some(definitions) => Err(EquipmentDefinitionError::AmbiguousDefinition {
                token: token.into(),
                sources: definitions.iter().map(|value| value.source.normalized_game_relative_path.clone()).collect(),
            }),
            None => Err(EquipmentDefinitionError::MissingDefinition(token.into())),
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
                    expanded: expanded_definition.value.clone(), container_limit: limit.clone(),
                });
            }
        }
        self.sources.push(AuthoredEquipmentSource { source, authored: root });
        Ok(())
    }
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

    fn catalog(text: &str) -> AuthoredEquipmentCatalog {
        let mut catalog = AuthoredEquipmentCatalog { definitions: BTreeMap::new(), sources: Vec::new(), modules: BTreeMap::new(), active_dlcs: None };
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
        let mut catalog = AuthoredEquipmentCatalog { definitions: BTreeMap::new(), sources: Vec::new(), modules: BTreeMap::new(), active_dlcs: None };
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
        let mut catalog = AuthoredEquipmentCatalog { definitions: BTreeMap::new(), sources: Vec::new(), modules: BTreeMap::new(), active_dlcs: None };
        assert!(matches!(catalog.insert_source(source("common/units/equipment/a.txt"),
            b"equipments = { hull = { value = @absent } }"), Err(EquipmentDefinitionError::VariableExpansion { .. })));
    }

    #[test]
    fn retains_generated_roots_and_does_not_admit_unevaluated_limits() {
        let catalog = catalog(r#"equipments = { limit = { has_dlc = "DLC" } hull = { type = armor } }
            duplicate_archetypes = { derived = { archetype = hull for_each = { x = { set = 2 } } } }"#);
        assert_eq!(catalog.definition("hull").unwrap_err(), EquipmentDefinitionError::MissingActiveDlcProfile);
        assert!(catalog.sources()[0].authored.first_value_ci("duplicate_archetypes").is_some());
        assert!(matches!(catalog.definition("limit"), Err(EquipmentDefinitionError::MissingDefinition(_))));
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

}
