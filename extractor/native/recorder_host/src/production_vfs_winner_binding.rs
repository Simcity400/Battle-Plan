//! Stable source-inventory v3 to ephemeral render-authority winner binding.
//!
//! Absolute roots never enter this seam. The Python resolver records exact
//! loose-file or ZIP member physical ranges; this module independently admits
//! the document, applies its ordered override/replace_path plan, and returns
//! the path-redacted binding sent through authenticated H4CT.

use portable_content_manifest::{
    normalize_game_relative_path, ContentLayerKind, LayerStorageKind, RecordedContentLayer,
};
use production_render_submit_live_lane::{
    RawEffectiveVfsWinnerIndexBinding, RawEffectiveVfsWinnerRow, RawVfsContentIdentity,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const SOURCE_INVENTORY_SCHEMA: &str = "extractor.live_binding_source_inventory.v3";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceInventoryV3 {
    schema: String,
    identity: SourceInventoryIdentity,
    namespaces: Vec<SourceNamespaceV3>,
    rows: Vec<SourceRowV3>,
    namespace_sha256: String,
    rows_sha256: String,
    inventory_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceInventoryIdentity {
    playset_id: String,
    profile_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceNamespaceV3 {
    ordinal: u32,
    kind: String,
    binding: String,
    id: String,
    label: String,
    replace_paths: Vec<String>,
    storage_kind: String,
    source_container_sha256: Option<String>,
    source_container_bytes: Option<u64>,
    file_count: u64,
    content_sha256: String,
    binding_content_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceRowV3 {
    namespace_ordinal: u32,
    layer_kind: String,
    layer_id: String,
    relpath: String,
    bytes: u64,
    sha256: String,
    storage_kind: String,
    source_container_sha256: Option<String>,
    source_byte_offset: u64,
    source_byte_count: u64,
    source_range_sha256: String,
}

/// Builds the exact effective winner index from an already finalized v3
/// inventory. The file is read twice and must remain byte-identical.
pub fn build_effective_vfs_winner_binding_from_inventory(
    source_inventory: &Path,
    ordered_layers: &[RecordedContentLayer],
    process_start_nonce: u64,
    authority_manifest_sha256: &str,
) -> Result<RawEffectiveVfsWinnerIndexBinding, String> {
    if process_start_nonce == 0 || !valid_sha(authority_manifest_sha256) {
        return Err("winner binding process nonce or authority digest is invalid".into());
    }
    let first = fs::read(source_inventory)
        .map_err(|error| format!("read source inventory failed: {error}"))?;
    let second = fs::read(source_inventory)
        .map_err(|error| format!("re-read source inventory failed: {error}"))?;
    if first != second {
        return Err("source inventory changed while building winner binding".into());
    }
    let inventory: SourceInventoryV3 = serde_json::from_slice(&first)
        .map_err(|error| format!("decode source inventory v3 failed: {error}"))?;
    validate_inventory(&inventory)?;

    let rows_by_namespace = inventory.rows.iter().fold(
        BTreeMap::<u32, Vec<&SourceRowV3>>::new(),
        |mut grouped, row| {
            grouped.entry(row.namespace_ordinal).or_default().push(row);
            grouped
        },
    );
    let mut effective = BTreeMap::<String, RawEffectiveVfsWinnerRow>::new();
    let mut selected_layer_ordinal = 0usize;
    for namespace in &inventory.namespaces {
        if namespace.binding == "asset_only" {
            continue;
        }
        let layer = ordered_layers
            .get(selected_layer_ordinal)
            .ok_or("source inventory has more selected namespaces than ordered layers")?;
        validate_namespace_layer(namespace, layer, selected_layer_ordinal)?;
        selected_layer_ordinal += 1;
        let replace_paths = namespace
            .replace_paths
            .iter()
            .map(|path| normalize_game_relative_path(path))
            .collect::<Result<BTreeSet<_>, _>>()?;
        effective.retain(|path, _| !replace_paths.contains(logical_parent(path)));
        for source in rows_by_namespace
            .get(&namespace.ordinal)
            .into_iter()
            .flatten()
        {
            let logical_path = normalize_game_relative_path(&source.relpath)?;
            let layer_kind = match layer.layer_kind {
                ContentLayerKind::Base => "base",
                ContentLayerKind::IntegratedDlc => "integrated_dlc",
                ContentLayerKind::Dlc => "dlc",
                ContentLayerKind::Mod => "mod",
            };
            let source_sha256 = source
                .source_container_sha256
                .clone()
                .unwrap_or_else(|| source.sha256.clone());
            let winner = RawEffectiveVfsWinnerRow {
                content: RawVfsContentIdentity {
                    logical_path: logical_path.clone(),
                    winning_layer_ordinal: layer.ordinal,
                    layer_kind: layer_kind.into(),
                    layer_id: layer.layer_id.clone(),
                    source_sha256,
                    payload_sha256: source.sha256.clone(),
                    payload_bytes: source.bytes,
                },
                source_container_sha256: source.source_container_sha256.clone(),
                source_byte_offset: source.source_byte_offset,
                source_byte_count: source.source_byte_count,
                source_range_sha256: source.source_range_sha256.clone(),
            };
            winner.validate()?;
            effective.insert(logical_path, winner);
        }
    }
    if selected_layer_ordinal != ordered_layers.len() {
        return Err("ordered layers contain namespaces absent from source inventory".into());
    }
    let ordered_rows = effective.into_values().collect::<Vec<_>>();
    let binding = RawEffectiveVfsWinnerIndexBinding {
        process_start_nonce,
        authority_manifest_sha256: authority_manifest_sha256.into(),
        declared_row_count: u64::try_from(ordered_rows.len())
            .map_err(|_| "effective winner count overflow")?,
        ordered_rows_sha256: RawEffectiveVfsWinnerIndexBinding::canonical_rows_sha256(
            &ordered_rows,
        )?,
        ordered_rows,
    };
    binding.validate()?;
    Ok(binding)
}

fn validate_namespace_layer(
    namespace: &SourceNamespaceV3,
    layer: &RecordedContentLayer,
    expected_ordinal: usize,
) -> Result<(), String> {
    let expected_kind = match namespace.kind.as_str() {
        "vanilla" => ContentLayerKind::Base,
        "integrated_dlc" => ContentLayerKind::IntegratedDlc,
        "dlc" => ContentLayerKind::Dlc,
        "mod" => ContentLayerKind::Mod,
        _ => return Err("source inventory namespace kind is unsupported".into()),
    };
    let expected_storage = match namespace.storage_kind.as_str() {
        "directory" => LayerStorageKind::Directory,
        "zip_archive" => LayerStorageKind::ZipArchive,
        _ => return Err("source inventory namespace storage kind is unsupported".into()),
    };
    let mut namespace_replace_paths = namespace
        .replace_paths
        .iter()
        .map(|path| normalize_game_relative_path(path))
        .collect::<Result<Vec<_>, _>>()?;
    let mut layer_replace_paths = layer
        .replace_paths
        .iter()
        .map(|path| normalize_game_relative_path(path))
        .collect::<Result<Vec<_>, _>>()?;
    namespace_replace_paths.sort();
    layer_replace_paths.sort();
    if layer.ordinal as usize != expected_ordinal
        || layer.layer_kind != expected_kind
        || layer.storage_kind != expected_storage
        || layer.source_container_sha256 != namespace.source_container_sha256
        || namespace_replace_paths != layer_replace_paths
        || expected_kind == ContentLayerKind::Base && layer.layer_id != "base"
        || expected_kind == ContentLayerKind::Mod && layer.layer_id != namespace.id
    {
        return Err("source inventory namespace disagrees with ordered content layer".into());
    }
    Ok(())
}

fn validate_inventory(inventory: &SourceInventoryV3) -> Result<(), String> {
    if inventory.schema != SOURCE_INVENTORY_SCHEMA
        || inventory.identity.playset_id.trim().is_empty()
        || inventory.identity.profile_id.trim().is_empty()
        || inventory.namespaces.is_empty()
        || inventory.rows.is_empty()
        || !valid_sha(&inventory.namespace_sha256)
        || !valid_sha(&inventory.rows_sha256)
        || !valid_sha(&inventory.inventory_sha256)
    {
        return Err("source inventory v3 header is incomplete".into());
    }
    let namespace_sha = sha_json(&inventory.namespaces)?;
    let rows_sha = sha_json(&inventory.rows)?;
    let core = json!({
        "schema": inventory.schema,
        "identity": inventory.identity,
        "namespaces": inventory.namespaces,
        "rows": inventory.rows,
        "namespace_sha256": namespace_sha,
        "rows_sha256": rows_sha,
    });
    if inventory.namespace_sha256 != namespace_sha
        || inventory.rows_sha256 != rows_sha
        || inventory.inventory_sha256 != sha_json(&core)?
    {
        return Err("source inventory v3 digest mismatch".into());
    }

    let mut seen_labels = BTreeSet::new();
    let mut seen_rows = BTreeSet::new();
    for (position, namespace) in inventory.namespaces.iter().enumerate() {
        if namespace.ordinal as usize != position
            || namespace.id.trim().is_empty()
            || namespace.label.trim().is_empty()
            || !seen_labels.insert(namespace.label.clone())
            || !matches!(
                namespace.binding.as_str(),
                "base" | "integrated" | "active_product" | "asset_only" | "active_mod"
            )
            || !valid_sha(&namespace.content_sha256)
            || !valid_sha(&namespace.binding_content_sha256)
        {
            return Err("source inventory v3 namespace is malformed".into());
        }
        match namespace.storage_kind.as_str() {
            "directory"
                if namespace.source_container_sha256.is_none()
                    && namespace.source_container_bytes.is_none() => {}
            "zip_archive"
                if namespace
                    .source_container_sha256
                    .as_deref()
                    .is_some_and(valid_sha)
                    && namespace
                        .source_container_bytes
                        .is_some_and(|bytes| bytes > 0) => {}
            _ => return Err("source inventory v3 namespace storage identity is malformed".into()),
        }
    }
    for row in &inventory.rows {
        let namespace = inventory
            .namespaces
            .get(row.namespace_ordinal as usize)
            .ok_or("source inventory row names an absent namespace")?;
        let logical = normalize_game_relative_path(&row.relpath)?;
        if row.layer_kind != namespace.kind
            || row.layer_id != namespace.id
            || row.storage_kind != namespace.storage_kind
            || !valid_sha(&row.sha256)
            || !valid_sha(&row.source_range_sha256)
            || !seen_rows.insert((row.namespace_ordinal, logical))
        {
            return Err("source inventory v3 row is malformed or repeated".into());
        }
        match row.storage_kind.as_str() {
            "directory"
                if row.source_container_sha256.is_none()
                    && row.source_byte_offset == 0
                    && row.source_byte_count == row.bytes
                    && row.source_range_sha256 == row.sha256 => {}
            "zip_archive"
                if row.source_container_sha256 == namespace.source_container_sha256
                    && row
                        .source_byte_offset
                        .checked_add(row.source_byte_count)
                        .is_some_and(|end| {
                            end <= namespace.source_container_bytes.unwrap_or(0)
                        }) => {}
            _ => return Err("source inventory v3 row physical range is malformed".into()),
        }
    }
    for namespace in &inventory.namespaces {
        let rows = inventory
            .rows
            .iter()
            .filter(|row| row.namespace_ordinal == namespace.ordinal)
            .collect::<Vec<_>>();
        if rows.len() as u64 != namespace.file_count {
            return Err("source inventory v3 namespace file count differs".into());
        }
        let content_rows = rows
            .into_iter()
            .map(|row| {
                json!({
                    "relpath": row.relpath,
                    "bytes": row.bytes,
                    "sha256": row.sha256,
                })
            })
            .collect::<Vec<_>>();
        if sha_json(&content_rows)? != namespace.content_sha256 {
            return Err("source inventory v3 namespace content digest differs".into());
        }
    }
    Ok(())
}

fn sha_json(value: &impl Serialize) -> Result<String, String> {
    let value = serde_json::to_value(value)
        .map_err(|error| format!("canonicalize source inventory JSON failed: {error}"))?;
    Ok(hex_sha256(&canonical_json(&value)?))
}

fn canonical_json(value: &Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec(value)
        .map_err(|error| format!("encode canonical source inventory JSON failed: {error}"))
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn logical_parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestRoot(std::path::PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "vfs-winner-binding-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos()
            ));
            fs::create_dir_all(&path).expect("create test root");
            Self(path)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn payload_sha(bytes: &[u8]) -> String {
        hex_sha256(bytes)
    }

    fn inventory() -> SourceInventoryV3 {
        let base_common = b"base";
        let base_gui = b"gui";
        let mod_common = b"override";
        let archive_hash = "c".repeat(64);
        let mut rows = vec![
            SourceRowV3 {
                namespace_ordinal: 0,
                layer_kind: "vanilla".into(),
                layer_id: "vanilla".into(),
                relpath: "common/base.txt".into(),
                bytes: base_common.len() as u64,
                sha256: payload_sha(base_common),
                storage_kind: "directory".into(),
                source_container_sha256: None,
                source_byte_offset: 0,
                source_byte_count: base_common.len() as u64,
                source_range_sha256: payload_sha(base_common),
            },
            SourceRowV3 {
                namespace_ordinal: 0,
                layer_kind: "vanilla".into(),
                layer_id: "vanilla".into(),
                relpath: "interface/base.gui".into(),
                bytes: base_gui.len() as u64,
                sha256: payload_sha(base_gui),
                storage_kind: "directory".into(),
                source_container_sha256: None,
                source_byte_offset: 0,
                source_byte_count: base_gui.len() as u64,
                source_range_sha256: payload_sha(base_gui),
            },
            SourceRowV3 {
                namespace_ordinal: 1,
                layer_kind: "mod".into(),
                layer_id: "mod/override.mod".into(),
                relpath: "common/mod.txt".into(),
                bytes: mod_common.len() as u64,
                sha256: payload_sha(mod_common),
                storage_kind: "zip_archive".into(),
                source_container_sha256: Some(archive_hash.clone()),
                source_byte_offset: 41,
                source_byte_count: 6,
                source_range_sha256: "d".repeat(64),
            },
        ];
        let mut namespaces = vec![
            SourceNamespaceV3 {
                ordinal: 0,
                kind: "vanilla".into(),
                binding: "base".into(),
                id: "vanilla".into(),
                label: "vanilla".into(),
                replace_paths: Vec::new(),
                storage_kind: "directory".into(),
                source_container_sha256: None,
                source_container_bytes: None,
                file_count: 2,
                content_sha256: String::new(),
                binding_content_sha256: "1".repeat(64),
            },
            SourceNamespaceV3 {
                ordinal: 1,
                kind: "mod".into(),
                binding: "active_mod".into(),
                id: "mod/override.mod".into(),
                label: "Override".into(),
                replace_paths: vec!["common".into()],
                storage_kind: "zip_archive".into(),
                source_container_sha256: Some(archive_hash),
                source_container_bytes: Some(100),
                file_count: 1,
                content_sha256: String::new(),
                binding_content_sha256: "2".repeat(64),
            },
        ];
        for namespace in &mut namespaces {
            let content_rows = rows
                .iter()
                .filter(|row| row.namespace_ordinal == namespace.ordinal)
                .map(|row| {
                    json!({
                        "relpath": row.relpath,
                        "bytes": row.bytes,
                        "sha256": row.sha256,
                    })
                })
                .collect::<Vec<_>>();
            namespace.content_sha256 = sha_json(&content_rows).unwrap();
        }
        let namespace_sha256 = sha_json(&namespaces).unwrap();
        let rows_sha256 = sha_json(&rows).unwrap();
        let mut value = SourceInventoryV3 {
            schema: SOURCE_INVENTORY_SCHEMA.into(),
            identity: SourceInventoryIdentity {
                playset_id: "fixture-playset".into(),
                profile_id: "fixture-profile".into(),
            },
            namespaces,
            rows: std::mem::take(&mut rows),
            namespace_sha256,
            rows_sha256,
            inventory_sha256: String::new(),
        };
        value.inventory_sha256 = sha_json(&json!({
            "schema": value.schema,
            "identity": value.identity,
            "namespaces": value.namespaces,
            "rows": value.rows,
            "namespace_sha256": value.namespace_sha256,
            "rows_sha256": value.rows_sha256,
        }))
        .unwrap();
        value
    }

    fn write_inventory(path: &Path, inventory: &SourceInventoryV3) {
        fs::write(path, serde_json::to_vec(inventory).unwrap()).unwrap();
    }

    fn ordered_layers() -> Vec<RecordedContentLayer> {
        vec![
            RecordedContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                display_name: "Base".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: false,
            },
            RecordedContentLayer {
                ordinal: 1,
                layer_kind: ContentLayerKind::Mod,
                layer_id: "mod/override.mod".into(),
                display_name: "Override".into(),
                descriptor_sha256: Some("4".repeat(64)),
                source_container_sha256: Some("c".repeat(64)),
                storage_kind: LayerStorageKind::ZipArchive,
                replace_paths: vec!["common".into()],
                required_for_playback: false,
            },
        ]
    }

    #[test]
    fn builder_applies_archive_mod_replace_path_and_is_relocation_invariant() {
        let first = TestRoot::new("a");
        let second = TestRoot::new("b");
        let first_path = first.0.join("source_inventory.json");
        let second_path = second.0.join("source_inventory.json");
        let inventory = inventory();
        write_inventory(&first_path, &inventory);
        write_inventory(&second_path, &inventory);
        let a = build_effective_vfs_winner_binding_from_inventory(
            &first_path,
            &ordered_layers(),
            7,
            &"a".repeat(64),
        )
        .unwrap();
        let b = build_effective_vfs_winner_binding_from_inventory(
            &second_path,
            &ordered_layers(),
            7,
            &"a".repeat(64),
        )
        .unwrap();
        assert_eq!(a, b);
        assert_eq!(a.declared_row_count, 2);
        assert_eq!(a.ordered_rows[0].content.logical_path, "common/mod.txt");
        assert_eq!(a.ordered_rows[0].source_byte_offset, 41);
        assert_eq!(a.ordered_rows[0].source_byte_count, 6);
        assert_ne!(
            a.ordered_rows[0].source_byte_count,
            a.ordered_rows[0].content.payload_bytes
        );
        assert_eq!(a.ordered_rows[1].content.logical_path, "interface/base.gui");
    }

    #[test]
    fn builder_rejects_archive_range_and_inventory_hash_drift() {
        let root = TestRoot::new("tamper");
        let path = root.0.join("source_inventory.json");
        let mut malformed = inventory();
        malformed.rows[2].source_byte_offset = 99;
        malformed.rows[2].source_byte_count = 2;
        malformed.rows_sha256 = sha_json(&malformed.rows).unwrap();
        malformed.inventory_sha256 = sha_json(&json!({
            "schema": malformed.schema,
            "identity": malformed.identity,
            "namespaces": malformed.namespaces,
            "rows": malformed.rows,
            "namespace_sha256": malformed.namespace_sha256,
            "rows_sha256": malformed.rows_sha256,
        }))
        .unwrap();
        write_inventory(&path, &malformed);
        assert!(build_effective_vfs_winner_binding_from_inventory(
            &path,
            &ordered_layers(),
            7,
            &"a".repeat(64)
        )
        .is_err());

        let mut hash_drift = inventory();
        hash_drift.rows_sha256 = "0".repeat(64);
        write_inventory(&path, &hash_drift);
        assert!(build_effective_vfs_winner_binding_from_inventory(
            &path,
            &ordered_layers(),
            7,
            &"a".repeat(64)
        )
        .is_err());
    }
}
