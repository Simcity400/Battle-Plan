//! An admitted resolver over a synthetic vanilla root, built through the same
//! manifest -> inventory -> admission chain the product runs. A reader that
//! bypasses the admission is caught by tampering with the admission's own
//! snapshot: an admitted read refuses, a raw read returns the tampered bytes.

use map_viewer::{GameFileResolver, PlaybackEnvironment};
use replay_core::{
    ContentLayerKind, ContentManifest, LayerStorageKind, RecordedContentLayer, RequiredContent,
    RequiredContentKind, SourceGameIdentity, SourceProfileIdentity,
    ValidatedContentAdmission,
};
use portable_content_manifest::{
    content_namespace::{ContentNamespaceScope, SealedContentNamespace},
    effective_vfs::{EffectiveVfs, EffectiveVfsLayerMount},
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
    admit_vanilla_scope(game, language, paths, label, &ContentNamespaceScope::root())
}

pub(crate) fn admit_vanilla_scope(game: &Path, language: &str, paths: &[&str], label: &str,
    scope: &ContentNamespaceScope,
) -> (GameFileResolver, ValidatedContentAdmission) {
    let resolver = GameFileResolver::vanilla(game).unwrap();
    let vfs = "e".repeat(64);
    let sealed = seal_vanilla_namespace_scope(game, &vfs, scope);
    for path in paths {
        let path = portable_content_manifest::normalize_game_relative_path(path).unwrap();
        assert!(sealed.winners().iter().any(|row| row.normalized_game_relative_path == path),
            "required fixture path was not observed: {path}");
    }
    let required_content = sealed.winners().iter().map(|row| RequiredContent {
        content_key: row.normalized_game_relative_path.clone(),
        kind: RequiredContentKind::Other,
        observation_count: 1,
        consumers: vec![format!("{label}/{}", row.payload_sha256)],
        content: row.clone(),
    }).collect();
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
        witnessed_content_namespaces: Vec::new(),
        witnessed_absent_content: Vec::new(),
        manifest_digest_sha256: None,
    };
    manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
    manifest.bind_observed_namespace(sealed).unwrap();
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

pub(crate) fn seal_vanilla_namespace(game: &Path, authority: &str) -> SealedContentNamespace {
    seal_vanilla_namespace_scope(game, authority, &ContentNamespaceScope::root())
}

fn seal_vanilla_namespace_scope(game: &Path, authority: &str, scope: &ContentNamespaceScope) -> SealedContentNamespace {
    let vfs = EffectiveVfs::new(authority, [EffectiveVfsLayerMount {
        ordinal: 0,
        layer_kind: ContentLayerKind::Base,
        layer_id: "base".into(),
        storage_kind: LayerStorageKind::Directory,
        local_source: game.to_path_buf(),
        source_container_sha256: None,
        replace_paths: Vec::new(),
    }]).unwrap();
    vfs.observe_namespace(scope).unwrap().seal(&vfs).unwrap()
}

/// Admission snapshots are written read-only; a tamper has to clear that first.
pub(crate) fn overwrite_readonly(path: &Path, bytes: &[u8]) {
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(path, permissions).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn vanilla_admission_seals_complete_fixture_not_requested_subset() {
    let game = std::env::temp_dir().join(format!("view2-ui-namespace-{}-{}",
        std::process::id(), std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(game.join("interface")).unwrap();
    std::fs::write(game.join("interface/Selected.gui"), b"selected").unwrap();
    std::fs::write(game.join("interface/Other.gui"), b"other").unwrap();
    let (_, admission) = admit_vanilla(&game, "l_english", &["interface/Selected.gui"],
        "view2-complete-namespace");
    let rows = admission.require_complete_namespace(ContentNamespaceScope::root()).unwrap();
    assert_eq!(rows.iter().map(|row| row.normalized_game_relative_path.as_str()).collect::<Vec<_>>(),
        ["interface/other.gui", "interface/selected.gui"]);
    assert_eq!(rows[0].stored_game_relative_path, "interface/Other.gui");
    assert_eq!(rows[0].payload_bytes, 5);
    assert_eq!(rows[0].winning_layer_ordinal, 0);
    assert_eq!(rows[0].layer_kind, ContentLayerKind::Base);
    assert_eq!(rows[0].layer_id, "base");
    assert_eq!(admission.require_used_content("interface/other.gui").unwrap(), &rows[0]);
    std::fs::write(game.join("interface/Later.gui"), b"later").unwrap();
    assert_eq!(admission.require_complete_namespace(ContentNamespaceScope::root()).unwrap(), rows);
    assert!(admission.require_used_content("interface/later.gui").is_err());
    std::fs::remove_dir_all(game).unwrap();
}
