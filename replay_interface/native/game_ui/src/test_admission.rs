//! An admitted resolver over a synthetic vanilla root, built through the same
//! manifest -> inventory -> admission chain the product runs. A reader that
//! bypasses the admission is caught by tampering with the admission's own
//! snapshot: an admitted read refuses, a raw read returns the tampered bytes.

use map_viewer::{GameFileResolver, PlaybackEnvironment};
use replay_core::{
    ContentLayerKind, ContentManifest, LayerStorageKind, RecordedContentLayer, RequiredContent,
    RequiredContentKind, SourceGameIdentity, SourceProfileIdentity, UsedContentKey,
    ValidatedContentAdmission,
};
use std::collections::BTreeMap;
use std::path::Path;

/// `label` separates one test's admission identity from another's: the
/// identity names a directory in the machine-wide admission cache.
pub(crate) fn admit_vanilla(
    game: &Path,
    language: &str,
    paths: &[&str],
    label: &str,
) -> (GameFileResolver, ValidatedContentAdmission) {
    let resolver = GameFileResolver::vanilla(game).unwrap();
    let resolved = resolver
        .resolve_used_content(paths.iter().map(|path| path.to_string()))
        .unwrap();
    let vfs = "e".repeat(64);
    let required_content = resolved
        .iter()
        .map(|row| RequiredContent {
            content_key: row.logical_path.clone(),
            kind: RequiredContentKind::Other,
            observation_count: 1,
            consumers: vec![format!("{label}/{}", row.sha256)],
            content: UsedContentKey {
                effective_vfs_manifest_sha256: vfs.clone(),
                stored_game_relative_path: row.source_relative_path.clone(),
                normalized_game_relative_path: row.logical_path.clone(),
                winning_layer_ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                source_sha256: row.sha256.clone(),
                payload_sha256: row.sha256.clone(),
                payload_bytes: row.byte_len,
            },
        })
        .collect::<Vec<_>>();
    let source_game = SourceGameIdentity {
        version: "1.19.2".into(),
        build: "a729".into(),
        executable_sha256: "8".repeat(64),
        build_profile: label.into(),
    };
    let mut manifest = ContentManifest {
        schema: replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
        product_modset_sha256: "9".repeat(64),
        source_game: source_game.clone(),
        source_profile: SourceProfileIdentity {
            profile_id: format!("{label}-profile"),
            playset_id: format!("{label}-playset"),
            selection_sha256: "0".repeat(64),
        },
        effective_vfs_manifest_sha256: vfs,
        ordered_layers: vec![RecordedContentLayer {
            ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            display_name: "Base".into(),
            layer_id: "base".into(),
            descriptor_sha256: None,
            source_container_sha256: None,
            storage_kind: LayerStorageKind::Directory,
            replace_paths: Vec::new(),
            required_for_playback: true,
        }],
        dlc_selection: Vec::new(),
        localisation_language: language.into(),
        required_settings: BTreeMap::new(),
        recorder_local_settings: BTreeMap::new(),
        required_content,
        manifest_digest_sha256: None,
    };
    manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
    manifest = manifest.with_computed_digest().unwrap();
    let inventory = resolver
        .playback_inventory(
            &manifest,
            PlaybackEnvironment {
                source_game,
                source_profile: manifest.source_profile.clone(),
                dlc_selection: Vec::new(),
                localisation_language: language.into(),
                settings: BTreeMap::new(),
            },
        )
        .unwrap();
    let admission = manifest.admit_playback(&inventory).unwrap();
    let resolver = resolver.with_content_admission(&admission).unwrap();
    (resolver, admission)
}

/// Admission snapshots are written read-only; a tamper has to clear that first.
pub(crate) fn overwrite_readonly(path: &Path, bytes: &[u8]) {
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(path, permissions).unwrap();
    std::fs::write(path, bytes).unwrap();
}
