//! Semantic admission for the recorder-packaged research/focus catalog set.
//!
//! `source` authenticates each artifact's container role, byte hash, and
//! content-manifest epoch.  This module is the next boundary: it parses those
//! exact bytes and proves the three catalogs agree semantically.  Consumers
//! never reopen `game_defs/out` or substitute the playback machine's files.

use crate::source::PackagedStaticArtifact;
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const LOCALISATION_JOIN_ARTIFACT: &str = "localisation_join";
pub const IDEA_TECH_LOC_XREF_ARTIFACT: &str = "idea_tech_loc_xref_static";
pub const TECHNOLOGY_SHARING_ARTIFACT: &str = "technology_sharing_static";

const LOCALISATION_SCHEMA: &str = "static_localisation_join_r3.static_artifact.v1";
const IDEA_TECH_SCHEMA: &str = "idea_tech_loc_xref_static.static_artifact.v1";
const TECHNOLOGY_SHARING_SCHEMA: &str = "technology_sharing_static.static_artifact.v1";
const ARTIFACT_KIND: &str = "deterministic_static_replay_enrichment";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticResearchCatalogError {
    pub artifact: &'static str,
    pub reason: String,
}

impl StaticResearchCatalogError {
    fn new(artifact: &'static str, reason: impl Into<String>) -> Self {
        Self {
            artifact,
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for StaticResearchCatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} rejected: {}", self.artifact, self.reason)
    }
}

impl std::error::Error for StaticResearchCatalogError {}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct LocalisationCatalogEntry {
    pub name: String,
    pub raw: String,
    pub has_interpolation: bool,
    pub source_file: String,
    pub source_layer: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalisationJoinCatalog {
    keys: BTreeMap<String, LocalisationCatalogEntry>,
}

impl LocalisationJoinCatalog {
    pub fn entry(&self, key: &str) -> Option<&LocalisationCatalogEntry> {
        self.keys.get(key)
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        self.entry(key).map(|entry| entry.name.as_str())
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct AuthoredTechnologyFolder {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub x_raw: Option<String>,
    #[serde(default)]
    pub y_raw: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct AuthoredTechnologyLeadEdge {
    pub to: String,
    #[serde(default)]
    pub research_cost_coeff: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct IdeaTechnologyDefinition {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub picture: Option<String>,
    #[serde(default)]
    pub start_year: Option<f64>,
    #[serde(default)]
    pub doctrine: bool,
    #[serde(default)]
    pub force_use_small_tech_layout: bool,
    #[serde(default)]
    pub authored_folders: Vec<AuthoredTechnologyFolder>,
    #[serde(default)]
    pub sub_tech_index: Option<i64>,
    #[serde(default)]
    pub sub_technologies: Vec<String>,
    #[serde(default)]
    pub leads_to: Vec<String>,
    #[serde(default)]
    pub leads_to_edges: Vec<AuthoredTechnologyLeadEdge>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, Value>,
    #[serde(default)]
    pub xor: Vec<String>,
    #[serde(default)]
    pub literal_name: Option<String>,
    #[serde(default)]
    pub loc_key: Option<String>,
    #[serde(default)]
    pub localised_name: Option<String>,
    pub localisation_status: String,
    #[serde(default)]
    pub resolved_loc_key: Option<String>,
    #[serde(default)]
    pub resolution_rule: Option<String>,
    pub source_file: String,
    pub source_layer: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IdeaTechnologyLocalisationCatalog {
    ideas: BTreeMap<String, IdeaTechnologyDefinition>,
    technologies: BTreeMap<String, IdeaTechnologyDefinition>,
}

impl IdeaTechnologyLocalisationCatalog {
    pub fn idea(&self, id: &str) -> Option<&IdeaTechnologyDefinition> {
        self.ideas.get(id)
    }

    pub fn technology(&self, id: &str) -> Option<&IdeaTechnologyDefinition> {
        self.technologies.get(id)
    }

    /// Iterate effective recorder-packaged technology definitions in stable
    /// identifier order for observer-owned replay composition.
    pub fn technologies(
        &self,
    ) -> impl Iterator<Item = (&str, &IdeaTechnologyDefinition)> {
        self.technologies
            .iter()
            .map(|(id, definition)| (id.as_str(), definition))
    }

    pub fn idea_count(&self) -> usize {
        self.ideas.len()
    }

    pub fn technology_count(&self) -> usize {
        self.technologies.len()
    }

    /// Stable effective-VFS technology identities in canonical key order.
    pub fn technology_ids(&self) -> impl Iterator<Item = &str> {
        self.technologies.keys().map(String::as_str)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct TechnologySharingDefinition {
    pub id: String,
    pub base_research_sharing_per_country_bonus: f64,
    pub base_research_sharing_per_country_bonus_raw: String,
    #[serde(default)]
    pub categories: Vec<String>,
    pub categories_present: bool,
    pub category_scope: String,
    pub description_loc_key: String,
    pub description_localisation_status: String,
    pub localised_description: String,
    pub localised_name: String,
    pub name_loc_key: String,
    pub name_localisation_status: String,
    pub picture: String,
    #[serde(default)]
    pub runtime_hash_candidate: Option<String>,
    pub runtime_join_status: String,
    #[serde(default)]
    pub runtime_token_candidate: Option<String>,
    pub source_file: String,
    pub source_layer: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TechnologySharingCatalog {
    definitions: Vec<TechnologySharingDefinition>,
    definitions_by_id: BTreeMap<String, TechnologySharingDefinition>,
}

impl TechnologySharingCatalog {
    pub fn definitions(&self) -> &[TechnologySharingDefinition] {
        &self.definitions
    }

    pub fn definition(&self, id: &str) -> Option<&TechnologySharingDefinition> {
        self.definitions_by_id.get(id)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedStaticResearchCatalogs {
    content_manifest_digest_sha256: String,
    artifact_sha256: BTreeMap<&'static str, String>,
    localisation: LocalisationJoinCatalog,
    idea_technology: IdeaTechnologyLocalisationCatalog,
    technology_sharing: TechnologySharingCatalog,
}

impl AdmittedStaticResearchCatalogs {
    pub fn content_manifest_digest_sha256(&self) -> &str {
        &self.content_manifest_digest_sha256
    }

    pub fn artifact_sha256(&self, name: &str) -> Option<&str> {
        self.artifact_sha256.get(name).map(String::as_str)
    }

    pub fn localisation(&self) -> &LocalisationJoinCatalog {
        &self.localisation
    }

    pub fn idea_technology(&self) -> &IdeaTechnologyLocalisationCatalog {
        &self.idea_technology
    }

    pub fn technology_sharing(&self) -> &TechnologySharingCatalog {
        &self.technology_sharing
    }

    /// Focus definitions/topology are not present in this artifact set.  This
    /// lookup consumes the exact packaged localisation row only; callers must
    /// obtain node geometry/icon/kind from a separately authenticated owner.
    pub fn focus_display_name(&self, focus_id: &str) -> Option<&str> {
        self.localisation.text(focus_id)
    }

    pub fn technology_display_name(&self, technology_id: &str) -> Option<&str> {
        self.idea_technology
            .technology(technology_id)
            .and_then(|definition| definition.localised_name.as_deref())
    }
}

#[derive(Deserialize)]
struct ArtifactHeader {
    schema: String,
    artifact_kind: String,
    artifact_sha256: String,
}

#[derive(Deserialize)]
struct LocalisationRoot {
    #[serde(flatten)]
    header: ArtifactHeader,
    request: String,
    keys: BTreeMap<String, LocalisationCatalogEntry>,
    meta: LocalisationMeta,
}

#[derive(Deserialize)]
struct LocalisationMeta {
    key_count: u64,
}

#[derive(Deserialize)]
struct IdeaTechnologyRoot {
    #[serde(flatten)]
    header: ArtifactHeader,
    tokens: IdeaTechnologyTokens,
    meta: IdeaTechnologyMeta,
}

#[derive(Deserialize)]
struct IdeaTechnologyTokens {
    ideas: BTreeMap<String, IdeaTechnologyDefinition>,
    technologies: BTreeMap<String, IdeaTechnologyDefinition>,
}

#[derive(Deserialize)]
struct IdeaTechnologyMeta {
    idea_count: u64,
    tech_count: u64,
}

#[derive(Deserialize)]
struct TechnologySharingRoot {
    #[serde(flatten)]
    header: ArtifactHeader,
    definitions: Vec<TechnologySharingDefinition>,
    definitions_by_id: BTreeMap<String, TechnologySharingDefinition>,
    join_keys: TechnologySharingJoinKeys,
    meta: TechnologySharingMeta,
}

#[derive(Deserialize)]
struct TechnologySharingJoinKeys {
    compatibility_status: String,
    #[serde(default)]
    runtime_hash_candidate: Option<String>,
}

#[derive(Deserialize)]
struct TechnologySharingMeta {
    definition_count: u64,
    runtime_token_proof: String,
    runtime_hash_proof: String,
}

pub fn admit_static_research_catalogs(
    localisation: &PackagedStaticArtifact,
    idea_technology: &PackagedStaticArtifact,
    technology_sharing: &PackagedStaticArtifact,
) -> Result<AdmittedStaticResearchCatalogs, StaticResearchCatalogError> {
    validate_artifact_identity(localisation, LOCALISATION_JOIN_ARTIFACT)?;
    validate_artifact_identity(idea_technology, IDEA_TECH_LOC_XREF_ARTIFACT)?;
    validate_artifact_identity(technology_sharing, TECHNOLOGY_SHARING_ARTIFACT)?;
    validate_embedded_self_hash(localisation.bytes(), LOCALISATION_JOIN_ARTIFACT)?;
    validate_embedded_self_hash(idea_technology.bytes(), IDEA_TECH_LOC_XREF_ARTIFACT)?;
    validate_embedded_self_hash(
        technology_sharing.bytes(),
        TECHNOLOGY_SHARING_ARTIFACT,
    )?;
    let content_digest = localisation.content_manifest_digest_sha256();
    for (name, artifact) in [
        (IDEA_TECH_LOC_XREF_ARTIFACT, idea_technology),
        (TECHNOLOGY_SHARING_ARTIFACT, technology_sharing),
    ] {
        if artifact.content_manifest_digest_sha256() != content_digest {
            return Err(StaticResearchCatalogError::new(
                name,
                "content-manifest epoch disagrees with localisation_join",
            ));
        }
    }

    let localisation_root: LocalisationRoot =
        parse_root(localisation.bytes(), LOCALISATION_JOIN_ARTIFACT)?;
    validate_header(
        &localisation_root.header,
        LOCALISATION_SCHEMA,
        LOCALISATION_JOIN_ARTIFACT,
    )?;
    if localisation_root.request != "R3" {
        return Err(StaticResearchCatalogError::new(
            LOCALISATION_JOIN_ARTIFACT,
            "request must be R3",
        ));
    }
    count_matches(
        LOCALISATION_JOIN_ARTIFACT,
        "meta.key_count",
        localisation_root.meta.key_count,
        localisation_root.keys.len(),
    )?;
    validate_localisation_entries(&localisation_root.keys)?;
    let localisation_catalog = LocalisationJoinCatalog {
        keys: localisation_root.keys,
    };

    let idea_root: IdeaTechnologyRoot =
        parse_root(idea_technology.bytes(), IDEA_TECH_LOC_XREF_ARTIFACT)?;
    validate_header(
        &idea_root.header,
        IDEA_TECH_SCHEMA,
        IDEA_TECH_LOC_XREF_ARTIFACT,
    )?;
    count_matches(
        IDEA_TECH_LOC_XREF_ARTIFACT,
        "meta.idea_count",
        idea_root.meta.idea_count,
        idea_root.tokens.ideas.len(),
    )?;
    count_matches(
        IDEA_TECH_LOC_XREF_ARTIFACT,
        "meta.tech_count",
        idea_root.meta.tech_count,
        idea_root.tokens.technologies.len(),
    )?;
    validate_definition_map(
        IDEA_TECH_LOC_XREF_ARTIFACT,
        "idea",
        &idea_root.tokens.ideas,
        &localisation_catalog,
    )?;
    validate_definition_map(
        IDEA_TECH_LOC_XREF_ARTIFACT,
        "tech",
        &idea_root.tokens.technologies,
        &localisation_catalog,
    )?;
    let idea_technology_catalog = IdeaTechnologyLocalisationCatalog {
        ideas: idea_root.tokens.ideas,
        technologies: idea_root.tokens.technologies,
    };

    let sharing_root: TechnologySharingRoot =
        parse_root(technology_sharing.bytes(), TECHNOLOGY_SHARING_ARTIFACT)?;
    validate_header(
        &sharing_root.header,
        TECHNOLOGY_SHARING_SCHEMA,
        TECHNOLOGY_SHARING_ARTIFACT,
    )?;
    count_matches(
        TECHNOLOGY_SHARING_ARTIFACT,
        "meta.definition_count",
        sharing_root.meta.definition_count,
        sharing_root.definitions.len(),
    )?;
    validate_technology_sharing(&sharing_root, &localisation_catalog)?;
    let artifact_sha256 = BTreeMap::from([
        (LOCALISATION_JOIN_ARTIFACT, localisation.sha256().to_string()),
        (
            IDEA_TECH_LOC_XREF_ARTIFACT,
            idea_technology.sha256().to_string(),
        ),
        (
            TECHNOLOGY_SHARING_ARTIFACT,
            technology_sharing.sha256().to_string(),
        ),
    ]);
    Ok(AdmittedStaticResearchCatalogs {
        content_manifest_digest_sha256: content_digest.to_string(),
        artifact_sha256,
        localisation: localisation_catalog,
        idea_technology: idea_technology_catalog,
        technology_sharing: TechnologySharingCatalog {
            definitions: sharing_root.definitions,
            definitions_by_id: sharing_root.definitions_by_id,
        },
    })
}

fn parse_root<T: DeserializeOwned>(
    bytes: &[u8],
    artifact: &'static str,
) -> Result<T, StaticResearchCatalogError> {
    let value = snapshot::parse_json_strict(bytes).map_err(|error| {
        StaticResearchCatalogError::new(
            artifact,
            format!("strict semantic JSON rejected: {error}"),
        )
    })?;
    serde_json::from_value(value).map_err(|error| {
        StaticResearchCatalogError::new(artifact, format!("invalid typed catalog: {error}"))
    })
}

fn validate_embedded_self_hash(
    bytes: &[u8],
    artifact: &'static str,
) -> Result<(), StaticResearchCatalogError> {
    let mut semantic: Value = snapshot::parse_json_strict(bytes).map_err(|error| {
        StaticResearchCatalogError::new(
            artifact,
            format!("strict semantic JSON rejected: {error}"),
        )
    })?;
    let root = semantic.as_object_mut().ok_or_else(|| {
        StaticResearchCatalogError::new(artifact, "semantic root is not an object")
    })?;
    let claimed = root
        .remove("artifact_sha256")
        .and_then(|value| value.as_str().map(str::to_owned))
        .filter(|value| is_lowercase_sha256(value))
        .ok_or_else(|| {
            StaticResearchCatalogError::new(
                artifact,
                "embedded artifact_sha256 is not canonical",
            )
        })?;
    let canonical = canonical_json_value(&semantic);
    let canonical_bytes = serde_json::to_vec(&canonical).map_err(|error| {
        StaticResearchCatalogError::new(
            artifact,
            format!("cannot canonicalize semantic JSON: {error}"),
        )
    })?;
    let actual = format!("{:x}", Sha256::digest(canonical_bytes));
    if actual != claimed {
        return Err(StaticResearchCatalogError::new(
            artifact,
            "embedded artifact_sha256 disagrees with canonical semantic bytes",
        ));
    }
    Ok(())
}

fn canonical_json_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonical_json_value).collect()),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut ordered = serde_json::Map::new();
            for key in keys {
                ordered.insert(key.clone(), canonical_json_value(&object[key]));
            }
            Value::Object(ordered)
        }
        other => other.clone(),
    }
}

fn validate_artifact_identity(
    artifact: &PackagedStaticArtifact,
    expected: &'static str,
) -> Result<(), StaticResearchCatalogError> {
    if artifact.name() != expected {
        return Err(StaticResearchCatalogError::new(
            expected,
            format!(
                "packaged role returned artifact name {:?}",
                artifact.name()
            ),
        ));
    }
    for (field, value) in [
        ("container sha256", artifact.sha256()),
        (
            "content manifest digest",
            artifact.content_manifest_digest_sha256(),
        ),
    ] {
        if !is_lowercase_sha256(value) {
            return Err(StaticResearchCatalogError::new(
                expected,
                format!("{field} is not a canonical SHA-256"),
            ));
        }
    }
    if artifact.bytes().is_empty() {
        return Err(StaticResearchCatalogError::new(
            expected,
            "packaged semantic bytes are empty",
        ));
    }
    Ok(())
}

fn validate_header(
    header: &ArtifactHeader,
    schema: &str,
    artifact: &'static str,
) -> Result<(), StaticResearchCatalogError> {
    if header.schema != schema {
        return Err(StaticResearchCatalogError::new(
            artifact,
            format!("schema {:?} expected {schema:?}", header.schema),
        ));
    }
    if header.artifact_kind != ARTIFACT_KIND {
        return Err(StaticResearchCatalogError::new(
            artifact,
            format!(
                "artifact_kind {:?} expected {ARTIFACT_KIND:?}",
                header.artifact_kind
            ),
        ));
    }
    if !is_lowercase_sha256(&header.artifact_sha256) {
        return Err(StaticResearchCatalogError::new(
            artifact,
            "embedded artifact_sha256 is not canonical",
        ));
    }
    Ok(())
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn count_matches(
    artifact: &'static str,
    field: &str,
    declared: u64,
    actual: usize,
) -> Result<(), StaticResearchCatalogError> {
    if usize::try_from(declared).ok() != Some(actual) {
        return Err(StaticResearchCatalogError::new(
            artifact,
            format!("{field} {declared} disagrees with {actual} parsed rows"),
        ));
    }
    Ok(())
}

fn validate_localisation_entries(
    keys: &BTreeMap<String, LocalisationCatalogEntry>,
) -> Result<(), StaticResearchCatalogError> {
    if keys.is_empty() {
        return Err(StaticResearchCatalogError::new(
            LOCALISATION_JOIN_ARTIFACT,
            "keys is empty",
        ));
    }
    for (key, entry) in keys {
        if key.trim().is_empty()
            || entry.source_file.trim().is_empty()
            || entry.source_layer.trim().is_empty()
        {
            return Err(StaticResearchCatalogError::new(
                LOCALISATION_JOIN_ARTIFACT,
                format!("{key:?} has empty identity/provenance"),
            ));
        }
    }
    Ok(())
}

fn validate_definition_map(
    artifact: &'static str,
    expected_kind: &str,
    definitions: &BTreeMap<String, IdeaTechnologyDefinition>,
    localisation: &LocalisationJoinCatalog,
) -> Result<(), StaticResearchCatalogError> {
    for (key, definition) in definitions {
        if key.trim().is_empty()
            || definition.id.as_str() != key.as_str()
            || definition.kind != expected_kind
            || definition.source_file.trim().is_empty()
            || definition.source_layer.trim().is_empty()
        {
            return Err(StaticResearchCatalogError::new(
                artifact,
                format!("{expected_kind} row {key:?} has inconsistent identity/provenance"),
            ));
        }
        let unique_nonempty = |values: &[String]| {
            let set: BTreeSet<&str> = values
                .iter()
                .filter(|value| !value.trim().is_empty())
                .map(String::as_str)
                .collect();
            set.len() == values.len()
        };
        if !unique_nonempty(&definition.categories) || !unique_nonempty(&definition.folders) {
            return Err(StaticResearchCatalogError::new(
                artifact,
                format!("{expected_kind} row {key:?} has empty/duplicate category or folder"),
            ));
        }
        if expected_kind == "technology" {
            let authored_folder_names = definition
                .authored_folders
                .iter()
                .filter_map(|folder| folder.name.as_deref())
                .collect::<BTreeSet<_>>();
            let edge_targets = definition
                .leads_to_edges
                .iter()
                .map(|edge| edge.to.as_str())
                .collect::<Vec<_>>();
            if !unique_nonempty(&definition.sub_technologies)
                || !unique_nonempty(&definition.leads_to)
                || !unique_nonempty(&definition.xor)
                || authored_folder_names.len() != definition.authored_folders.len()
                || edge_targets
                    != definition
                        .leads_to
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                || definition.sub_tech_index.is_some_and(|index| index < 0)
                || definition.authored_folders.iter().any(|folder| {
                    folder.name.as_deref().is_none_or(str::is_empty)
                        || folder.x.is_some() != folder.y.is_some()
                        || folder.x.is_some_and(|value| !value.is_finite())
                        || folder.y.is_some_and(|value| !value.is_finite())
                })
                || definition.leads_to_edges.iter().any(|edge| {
                    edge.to.trim().is_empty()
                        || edge
                            .research_cost_coeff
                            .is_some_and(|value| !value.is_finite())
                })
            {
                return Err(StaticResearchCatalogError::new(
                    artifact,
                    format!("technology row {key:?} has invalid authored topology"),
                ));
            }
        }
        match definition.localisation_status.as_str() {
            "resolved" => {
                let resolved_key = definition.resolved_loc_key.as_deref().ok_or_else(|| {
                    StaticResearchCatalogError::new(
                        artifact,
                        format!("{expected_kind} row {key:?} lacks resolved_loc_key"),
                    )
                })?;
                let expected_name = localisation.text(resolved_key).ok_or_else(|| {
                    StaticResearchCatalogError::new(
                        artifact,
                        format!(
                            "{expected_kind} row {key:?} references absent localisation {resolved_key:?}"
                        ),
                    )
                })?;
                if definition.localised_name.as_deref() != Some(expected_name) {
                    return Err(StaticResearchCatalogError::new(
                        artifact,
                        format!(
                            "{expected_kind} row {key:?} disagrees with localisation_join"
                        ),
                    ));
                }
            }
            "resolved_literal" => {
                if definition.literal_name.as_deref() != definition.localised_name.as_deref()
                    || definition
                        .localised_name
                        .as_deref()
                        .is_none_or(str::is_empty)
                {
                    return Err(StaticResearchCatalogError::new(
                        artifact,
                        format!("{expected_kind} row {key:?} has invalid literal resolution"),
                    ));
                }
            }
            "missing_localisation_key" => {
                if definition.localised_name.is_some() || definition.resolved_loc_key.is_some() {
                    return Err(StaticResearchCatalogError::new(
                        artifact,
                        format!(
                            "{expected_kind} row {key:?} fabricates a missing localisation"
                        ),
                    ));
                }
            }
            status => {
                return Err(StaticResearchCatalogError::new(
                    artifact,
                    format!("{expected_kind} row {key:?} has unknown status {status:?}"),
                ));
            }
        }
    }
    if expected_kind == "technology" {
        let mut subtechnology_owners = BTreeMap::<&str, &str>::new();
        for (source, definition) in definitions {
            for target in definition
                .sub_technologies
                .iter()
                .chain(&definition.leads_to)
                .chain(&definition.xor)
            {
                if !definitions.contains_key(target) {
                    return Err(StaticResearchCatalogError::new(
                        artifact,
                        format!(
                            "technology row {source:?} references absent technology {target:?}"
                        ),
                    ));
                }
            }
            for child in &definition.sub_technologies {
                if let Some(prior) = subtechnology_owners.insert(child, source) {
                    return Err(StaticResearchCatalogError::new(
                        artifact,
                        format!(
                            "subtechnology {child:?} is owned by both {prior:?} and {source:?}"
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_technology_sharing(
    root: &TechnologySharingRoot,
    localisation: &LocalisationJoinCatalog,
) -> Result<(), StaticResearchCatalogError> {
    if root.definitions.len() != root.definitions_by_id.len() {
        return Err(StaticResearchCatalogError::new(
            TECHNOLOGY_SHARING_ARTIFACT,
            "definitions and definitions_by_id cardinalities disagree",
        ));
    }
    if root.join_keys.compatibility_status.trim().is_empty() {
        return Err(StaticResearchCatalogError::new(
            TECHNOLOGY_SHARING_ARTIFACT,
            "join_keys.compatibility_status is empty",
        ));
    }
    if root.join_keys.runtime_hash_candidate.is_some()
        || root.meta.runtime_token_proof != "engine_runtime_intern_ordinal_not_string_hash"
        || root.meta.runtime_hash_proof != "fnv1a_only_indexes_intern_table_not_runtime_token"
    {
        return Err(StaticResearchCatalogError::new(
            TECHNOLOGY_SHARING_ARTIFACT,
            "static runtime-token/hash authority disagrees with the exact intern-ordinal RE law",
        ));
    }
    let mut seen = BTreeSet::new();
    for definition in &root.definitions {
        if definition.id.trim().is_empty()
            || !seen.insert(definition.id.as_str())
            || definition.source_file.trim().is_empty()
            || definition.source_layer.trim().is_empty()
            || definition.picture.trim().is_empty()
            || !definition.base_research_sharing_per_country_bonus.is_finite()
            || definition.runtime_hash_candidate.is_some()
            || definition.runtime_token_candidate.as_deref() != Some(definition.id.as_str())
            || definition.runtime_join_status
                != "definition_id_candidate_only_runtime_token_is_intern_ordinal_mapping_unproven"
        {
            return Err(StaticResearchCatalogError::new(
                TECHNOLOGY_SHARING_ARTIFACT,
                format!(
                    "definition {:?} has invalid identity/provenance/value",
                    definition.id
                ),
            ));
        }
        if root.definitions_by_id.get(&definition.id) != Some(definition) {
            return Err(StaticResearchCatalogError::new(
                TECHNOLOGY_SHARING_ARTIFACT,
                format!(
                    "definitions_by_id row {:?} is absent or differs from definitions[]",
                    definition.id
                ),
            ));
        }
        let raw_bonus = definition
            .base_research_sharing_per_country_bonus_raw
            .parse::<f64>()
            .map_err(|error| {
                StaticResearchCatalogError::new(
                    TECHNOLOGY_SHARING_ARTIFACT,
                    format!(
                        "definition {:?} has invalid authored bonus literal: {error}",
                        definition.id
                    ),
                )
            })?;
        if raw_bonus.to_bits()
            != definition
                .base_research_sharing_per_country_bonus
                .to_bits()
        {
            return Err(StaticResearchCatalogError::new(
                TECHNOLOGY_SHARING_ARTIFACT,
                format!(
                    "definition {:?} parsed bonus disagrees with its authored literal",
                    definition.id
                ),
            ));
        }
        let category_shape_valid = match definition.category_scope.as_str() {
            "unrestricted_by_absent_categories" => {
                !definition.categories_present && definition.categories.is_empty()
            }
            "explicit_category_list" => {
                definition.categories_present && !definition.categories.is_empty()
            }
            "explicit_empty_categories_semantics_unproven" => {
                definition.categories_present && definition.categories.is_empty()
            }
            _ => false,
        };
        let unique_nonempty_categories = definition.categories.iter().all(|category| {
            !category.trim().is_empty()
                && definition
                    .categories
                    .iter()
                    .filter(|candidate| *candidate == category)
                    .count()
                    == 1
        });
        if !category_shape_valid || !unique_nonempty_categories {
            return Err(StaticResearchCatalogError::new(
                TECHNOLOGY_SHARING_ARTIFACT,
                format!(
                    "definition {:?} has inconsistent category declaration",
                    definition.id
                ),
            ));
        }
        for (status, loc_key, text, role) in [
            (
                definition.name_localisation_status.as_str(),
                definition.name_loc_key.as_str(),
                definition.localised_name.as_str(),
                "name",
            ),
            (
                definition.description_localisation_status.as_str(),
                definition.description_loc_key.as_str(),
                definition.localised_description.as_str(),
                "description",
            ),
        ] {
            if status != "resolved"
                || loc_key.trim().is_empty()
                || localisation.text(loc_key) != Some(text)
            {
                return Err(StaticResearchCatalogError::new(
                    TECHNOLOGY_SHARING_ARTIFACT,
                    format!(
                        "definition {:?} {role} disagrees with localisation_join",
                        definition.id
                    ),
                ));
            }
        }
    }
    Ok(())
}
