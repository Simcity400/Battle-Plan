
use crate::source::PackagedStaticArtifact;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const FOCUS_DEFS_STATIC_ARTIFACT: &str = "focus_defs_static";
const FOCUS_DEFS_STATIC_SCHEMA: &str = "focus_defs_static.static_artifact.v1";
const ARTIFACT_KIND: &str = "deterministic_static_replay_enrichment";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticFocusDefsError {
    pub reason: String,
}

impl StaticFocusDefsError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for StaticFocusDefsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{FOCUS_DEFS_STATIC_ARTIFACT} rejected: {}", self.reason)
    }
}

impl std::error::Error for StaticFocusDefsError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticFocusDefinitionKind {
    Local,
    Shared,
    Joint,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct StaticFocusBackgroundStyle {
    pub name: String,
    pub default: bool,
    pub unavailable: String,
    pub available: String,
    pub current: String,
    pub completed: String,
    pub source_file: String,
    pub source_layer: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StaticFocusDefinition {
    pub id: String,
    pub kind: StaticFocusDefinitionKind,
    pub icon_sprite: Option<String>,
    pub overlay_sprite: Option<String>,
    pub text_icon_style: Option<String>,
    pub title_loc_key: String,
    pub description_loc_key: String,
    pub authored_x: Option<f64>,
    pub authored_y: Option<f64>,
    pub relative_position_id: Option<String>,
    pub prerequisites: Vec<Vec<String>>,
    pub mutually_exclusive: Vec<String>,
    pub scripted_position_offsets: u32,
    pub allow_branch_blocks: u32,
    pub background_style_name: String,
    pub background_style: StaticFocusBackgroundStyle,
    pub source_file: String,
    pub source_layer: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticFocusTree {
    pub id: String,
    pub local_focus_ids: BTreeSet<String>,
    pub shared_focus_refs: BTreeSet<String>,
    pub source_file: String,
    pub source_layer: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedStaticFocusDefinitions {
    content_manifest_digest_sha256: String,
    artifact_sha256: String,
    embedded_artifact_sha256: String,
    trees: BTreeMap<String, StaticFocusTree>,
    focuses: BTreeMap<String, StaticFocusDefinition>,
    imported_shared_by_tree: BTreeMap<String, BTreeSet<String>>,
}

impl AdmittedStaticFocusDefinitions {
    pub fn content_manifest_digest_sha256(&self) -> &str {
        &self.content_manifest_digest_sha256
    }

    pub fn artifact_sha256(&self) -> &str {
        &self.artifact_sha256
    }

    pub fn embedded_artifact_sha256(&self) -> &str {
        &self.embedded_artifact_sha256
    }

    pub fn tree(&self, id: &str) -> Option<&StaticFocusTree> {
        self.trees.get(id)
    }

    pub fn definition(&self, id: &str) -> Option<&StaticFocusDefinition> {
        self.focuses.get(id)
    }

    pub fn definitions(
        &self,
    ) -> impl ExactSizeIterator<Item = (&str, &StaticFocusDefinition)> {
        self.focuses
            .iter()
            .map(|(id, definition)| (id.as_str(), definition))
    }

    pub fn imported_shared_node_ids(
        &self,
        tree_id: &str,
    ) -> Option<&BTreeSet<String>> {
        self.imported_shared_by_tree.get(tree_id)
    }
}

#[derive(Deserialize)]
struct Root {
    schema: String,
    artifact_kind: String,
    artifact_sha256: String,
    meta: Meta,
    trees: BTreeMap<String, TreeRow>,
    focuses: BTreeMap<String, FocusRow>,
    focus_background_styles: BTreeMap<String, StaticFocusBackgroundStyle>,
    source_files: Vec<SourceFileRow>,
}

#[derive(Deserialize)]
struct Meta {
    tree_count: u64,
    focus_count: u64,
    shared_focus_count: u64,
    joint_focus_count: u64,
    focus_background_style_count: u64,
    default_focus_background_styles: Vec<String>,
    unresolved_focus_background_styles: Vec<Value>,
    id_collisions_last_wins: Vec<Value>,
    files_parsed: u64,
}

#[derive(Deserialize)]
struct TreeRow {
    id: String,
    focus_count: u64,
    #[serde(default)]
    focuses: Vec<String>,
    #[serde(default)]
    shared_focus_refs: Vec<String>,
    source_file: String,
    source_layer: String,
}

#[derive(Deserialize)]
struct FocusRow {
    id: String,
    kind: String,
    title_loc_key: String,
    description_loc_key: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    overlay: Option<String>,
    #[serde(default)]
    text_icon: Option<String>,
    #[serde(default)]
    x: Option<f64>,
    #[serde(default)]
    y: Option<f64>,
    #[serde(default)]
    relative_position_id: Option<String>,
    #[serde(default)]
    prerequisites: Vec<Vec<String>>,
    mutually_exclusive: Vec<String>,
    scripted_position_offsets: u32,
    allow_branch_blocks: u32,
    background_style_name: String,
    background_style: StaticFocusBackgroundStyle,
    source_file: String,
    source_layer: String,
}

#[derive(Deserialize)]
struct SourceFileRow {
    relpath: String,
    bytes: u64,
    sha256: String,
}

pub fn admit_static_focus_definitions(
    artifact: &PackagedStaticArtifact,
) -> Result<AdmittedStaticFocusDefinitions, StaticFocusDefsError> {
    validate_packaged_identity(artifact)?;
    let mut semantic: Value = serde_json::from_slice(artifact.bytes())
        .map_err(|error| StaticFocusDefsError::new(format!("invalid semantic JSON: {error}")))?;
    let claimed_self_hash = semantic
        .get("artifact_sha256")
        .and_then(Value::as_str)
        .filter(|value| lowercase_sha256(value))
        .ok_or_else(|| StaticFocusDefsError::new("artifact_sha256 is not canonical"))?
        .to_string();
    semantic
        .as_object_mut()
        .ok_or_else(|| StaticFocusDefsError::new("semantic root is not an object"))?
        .remove("artifact_sha256");
    let canonical = canonical_json_value(&semantic);
    let canonical_bytes = serde_json::to_vec(&canonical)
        .map_err(|error| StaticFocusDefsError::new(format!("cannot canonicalize JSON: {error}")))?;
    let actual_self_hash = format!("{:x}", Sha256::digest(&canonical_bytes));
    if actual_self_hash != claimed_self_hash {
        return Err(StaticFocusDefsError::new(
            "embedded artifact_sha256 disagrees with canonical semantic bytes",
        ));
    }

    let root: Root = serde_json::from_slice(artifact.bytes())
        .map_err(|error| StaticFocusDefsError::new(format!("invalid typed catalog: {error}")))?;
    if root.schema != FOCUS_DEFS_STATIC_SCHEMA || root.artifact_kind != ARTIFACT_KIND {
        return Err(StaticFocusDefsError::new(
            "schema or artifact_kind is unsupported",
        ));
    }
    if root.artifact_sha256 != claimed_self_hash {
        return Err(StaticFocusDefsError::new(
            "typed artifact_sha256 disagrees with semantic root",
        ));
    }
    count_matches("meta.tree_count", root.meta.tree_count, root.trees.len())?;
    count_matches(
        "meta.focus_count",
        root.meta.focus_count,
        root.focuses.len(),
    )?;
    count_matches(
        "meta.focus_background_style_count",
        root.meta.focus_background_style_count,
        root.focus_background_styles.len(),
    )?;
    count_matches(
        "meta.files_parsed",
        root.meta.files_parsed,
        root.source_files.len(),
    )?;
    if root.meta.files_parsed == 0
        || root.trees.is_empty()
        || root.focuses.is_empty()
        || root.focus_background_styles.is_empty()
        || !root.meta.id_collisions_last_wins.is_empty()
        || !root.meta.unresolved_focus_background_styles.is_empty()
        || root.meta.default_focus_background_styles.len() != 1
    {
        return Err(StaticFocusDefsError::new(
            "catalog is empty, collided, unresolved, or lacks one exact default style",
        ));
    }
    let mut source_relpaths = BTreeSet::new();
    for source in &root.source_files {
        if source.relpath.trim().is_empty()
            || source.relpath.contains('\\')
            || source.relpath.starts_with('/')
            || source.relpath.split('/').any(|part| matches!(part, "" | "." | ".."))
            || !lowercase_sha256(&source.sha256)
            || !source_relpaths.insert(source.relpath.as_str())
        {
            return Err(StaticFocusDefsError::new(
                "source_files has unsafe, duplicate, or unauthenticated rows",
            ));
        }
        let _ = source.bytes;
    }
    let default_style = &root.meta.default_focus_background_styles[0];
    for (style_id, style) in &root.focus_background_styles {
        if style_id != &style.name
            || !portable_id(style_id)
            || style.source_file.trim().is_empty()
            || style.source_layer.trim().is_empty()
            || !source_relpaths.contains(style.source_file.as_str())
            || [
                &style.unavailable,
                &style.available,
                &style.current,
                &style.completed,
            ]
            .iter()
            .any(|value| value.trim().is_empty())
        {
            return Err(StaticFocusDefsError::new(format!(
                "background style {style_id:?} has inconsistent identity/provenance/sprites"
            )));
        }
    }
    if !root
        .focus_background_styles
        .get(default_style)
        .is_some_and(|style| style.default)
        || root
            .focus_background_styles
            .values()
            .filter(|style| style.default)
            .count()
            != 1
    {
        return Err(StaticFocusDefsError::new(
            "default background style declaration disagrees with style rows",
        ));
    }

    let mut focuses = BTreeMap::new();
    let mut shared_count = 0usize;
    let mut joint_count = 0usize;
    for (focus_id, row) in root.focuses {
        let kind = match row.kind.as_str() {
            "focus" => StaticFocusDefinitionKind::Local,
            "shared_focus" => {
                shared_count += 1;
                StaticFocusDefinitionKind::Shared
            }
            "joint_focus" => {
                joint_count += 1;
                StaticFocusDefinitionKind::Joint
            }
            _ => {
                return Err(StaticFocusDefsError::new(format!(
                    "focus {focus_id:?} has unknown definition kind"
                )));
            }
        };
        if row.id != focus_id
            || !portable_id(&focus_id)
            || row.title_loc_key != focus_id
            || row.description_loc_key != format!("{focus_id}_desc")
            || row.source_file.trim().is_empty()
            || row.source_layer.trim().is_empty()
            || !source_relpaths.contains(row.source_file.as_str())
            || row.x.is_some_and(|value| !value.is_finite())
            || row.y.is_some_and(|value| !value.is_finite())
            || row
                .relative_position_id
                .as_deref()
                .is_some_and(|id| !portable_id(id))
            || row.prerequisites.iter().any(|group| {
                group.iter().any(|id| !portable_id(id))
                    || group.iter().collect::<BTreeSet<_>>().len() != group.len()
            })
            || row.mutually_exclusive.iter().any(|id| !portable_id(id))
            || row
                .mutually_exclusive
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != row.mutually_exclusive.len()
            || row.mutually_exclusive.contains(&focus_id)
        {
            return Err(StaticFocusDefsError::new(format!(
                "focus {focus_id:?} has malformed identity, geometry, or dependency rows"
            )));
        }
        let registry_style = root
            .focus_background_styles
            .get(&row.background_style_name)
            .ok_or_else(|| {
                StaticFocusDefsError::new(format!(
                    "focus {focus_id:?} references absent background style"
                ))
            })?;
        if registry_style != &row.background_style
            || (kind == StaticFocusDefinitionKind::Joint
                && row.text_icon.as_deref() != Some(row.background_style_name.as_str()))
        {
            return Err(StaticFocusDefsError::new(format!(
                "focus {focus_id:?} background family is inconsistent"
            )));
        }
        focuses.insert(
            focus_id.clone(),
            StaticFocusDefinition {
                id: focus_id,
                kind,
                icon_sprite: row.icon,
                overlay_sprite: row.overlay,
                text_icon_style: row.text_icon,
                title_loc_key: row.title_loc_key,
                description_loc_key: row.description_loc_key,
                authored_x: row.x,
                authored_y: row.y,
                relative_position_id: row.relative_position_id,
                prerequisites: row.prerequisites,
                mutually_exclusive: row.mutually_exclusive,
                scripted_position_offsets: row.scripted_position_offsets,
                allow_branch_blocks: row.allow_branch_blocks,
                background_style_name: row.background_style_name,
                background_style: row.background_style,
                source_file: row.source_file,
                source_layer: row.source_layer,
            },
        );
    }
    count_matches(
        "meta.shared_focus_count",
        root.meta.shared_focus_count,
        shared_count,
    )?;
    count_matches(
        "meta.joint_focus_count",
        root.meta.joint_focus_count,
        joint_count,
    )?;

    let mut trees = BTreeMap::new();
    let mut local_owners = BTreeMap::new();
    for (tree_id, row) in root.trees {
        if row.id != tree_id
            || !portable_id(&tree_id)
            || row.source_file.trim().is_empty()
            || row.source_layer.trim().is_empty()
            || !source_relpaths.contains(row.source_file.as_str())
        {
            return Err(StaticFocusDefsError::new(format!(
                "focus tree {tree_id:?} has malformed identity/provenance"
            )));
        }
        let local_focus_ids = unique_ids(&row.focuses, &format!("tree {tree_id} focuses"))?;
        let shared_focus_refs =
            unique_ids(&row.shared_focus_refs, &format!("tree {tree_id} shared refs"))?;
        count_matches(
            "tree.focus_count",
            row.focus_count,
            local_focus_ids.len(),
        )?;
        for focus_id in &local_focus_ids {
            if focuses.get(focus_id).map(|row| row.kind)
                != Some(StaticFocusDefinitionKind::Local)
            {
                return Err(StaticFocusDefsError::new(format!(
                    "tree {tree_id:?} references absent/non-local node {focus_id:?}"
                )));
            }
            if local_owners.insert(focus_id.clone(), tree_id.clone()).is_some() {
                return Err(StaticFocusDefsError::new(format!(
                    "local focus {focus_id:?} belongs to more than one tree"
                )));
            }
        }
        for focus_id in &shared_focus_refs {
            if !matches!(
                focuses.get(focus_id).map(|row| row.kind),
                Some(StaticFocusDefinitionKind::Shared | StaticFocusDefinitionKind::Joint)
            ) {
                return Err(StaticFocusDefsError::new(format!(
                    "tree {tree_id:?} references absent/non-shared root {focus_id:?}"
                )));
            }
        }
        trees.insert(
            tree_id.clone(),
            StaticFocusTree {
                id: tree_id,
                local_focus_ids,
                shared_focus_refs,
                source_file: row.source_file,
                source_layer: row.source_layer,
            },
        );
    }
    let all_local: BTreeSet<&str> = focuses
        .values()
        .filter(|row| row.kind == StaticFocusDefinitionKind::Local)
        .map(|row| row.id.as_str())
        .collect();
    let owned_local: BTreeSet<&str> = local_owners.keys().map(String::as_str).collect();
    if all_local != owned_local {
        return Err(StaticFocusDefsError::new(
            "tree topology does not own every local focus definition exactly once",
        ));
    }

    let imported_shared_by_tree = trees
        .iter()
        .map(|(tree_id, tree)| {
            (
                tree_id.clone(),
                imported_shared_component(tree, &focuses),
            )
        })
        .collect();
    Ok(AdmittedStaticFocusDefinitions {
        content_manifest_digest_sha256: artifact
            .content_manifest_digest_sha256()
            .to_string(),
        artifact_sha256: artifact.sha256().to_string(),
        embedded_artifact_sha256: claimed_self_hash,
        trees,
        focuses,
        imported_shared_by_tree,
    })
}

fn imported_shared_component(
    tree: &StaticFocusTree,
    focuses: &BTreeMap<String, StaticFocusDefinition>,
) -> BTreeSet<String> {
    let mut admitted: BTreeSet<String> = tree
        .shared_focus_refs
        .iter()
        .filter(|id| {
            focuses.get(*id).map(|row| row.kind) == Some(StaticFocusDefinitionKind::Shared)
        })
        .cloned()
        .collect();
    loop {
        let before = admitted.len();
        for (id, row) in focuses {
            if row.kind != StaticFocusDefinitionKind::Shared || admitted.contains(id) {
                continue;
            }
            let linked = row
                .relative_position_id
                .as_ref()
                .is_some_and(|parent| admitted.contains(parent))
                || row
                    .prerequisites
                    .iter()
                    .flatten()
                    .any(|parent| admitted.contains(parent));
            if linked {
                admitted.insert(id.clone());
            }
        }
        if admitted.len() == before {
            return admitted;
        }
    }
}

fn validate_packaged_identity(
    artifact: &PackagedStaticArtifact,
) -> Result<(), StaticFocusDefsError> {
    if artifact.name() != FOCUS_DEFS_STATIC_ARTIFACT
        || !lowercase_sha256(artifact.sha256())
        || !lowercase_sha256(artifact.content_manifest_digest_sha256())
        || artifact.bytes().is_empty()
    {
        return Err(StaticFocusDefsError::new(
            "packaged role, hashes, or semantic bytes are invalid",
        ));
    }
    Ok(())
}

fn count_matches(
    field: &str,
    declared: u64,
    actual: usize,
) -> Result<(), StaticFocusDefsError> {
    if usize::try_from(declared).ok() != Some(actual) {
        return Err(StaticFocusDefsError::new(format!(
            "{field} {declared} disagrees with {actual} parsed rows"
        )));
    }
    Ok(())
}

fn unique_ids(
    values: &[String],
    field: &str,
) -> Result<BTreeSet<String>, StaticFocusDefsError> {
    let set: BTreeSet<String> = values.iter().cloned().collect();
    if set.len() != values.len() || values.iter().any(|id| !portable_id(id)) {
        return Err(StaticFocusDefsError::new(format!(
            "{field} contains empty, duplicate, or non-portable ids"
        )));
    }
    Ok(set)
}

fn portable_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && (bytes[0].is_ascii_alphanumeric() || bytes[0] == b'_')
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-' | b':'))
}

fn lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
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
