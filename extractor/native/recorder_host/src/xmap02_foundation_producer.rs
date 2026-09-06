//! XMAP02 content producer: emits the resolved-foundation-content record for
//! the five map visual content families from the finalized portable content
//! manifest.
//!
//! The QJ re-judge rows `content.borders`, `content.lighting_shadow`,
//! `content.political_fow`, `content.rivers_straits` and `content.water_ice`
//! were PRODUCER_missing: nothing capture-side emitted them, so a viewer
//! resolved every family's inputs from its own admission (product authority,
//! nothing recorded). This module closes that: at attach time it derives each
//! family's canonical-closure classification against the effective-VFS winners
//! actually admitted for the recording and seals the record.
//!
//! The derivation is SHARED CODE in the portable crate
//! (`derive_foundation_family_classification`), so consumers re-derive from
//! their own copy of the same manifest and verify the seal - nothing derivable
//! rides the wire (smallest pre-compression size). Winner selection is
//! CONSUMED from the manifest's own required-content rows; the proof that
//! overlapping mods win resolution is lane RC's binding interface, never
//! re-derived here.

use portable_content_manifest::foundation_family::{
    derive_family_missing_paths, canonical_closure, derive_foundation_family_classification,
    FoundationFamilyContentSet, FOUNDATION_FAMILY_CONTENT_SET_SCHEMA,
};
use portable_content_manifest::ContentManifest;

pub const XMAP02_FOUNDATION_CONTENT_FIELD: &str = "xmap02_foundation_content";
pub const XMAP02_FOUNDATION_CONTENT_GAP_SCHEMA: &str =
    "hoi4.replay.xmap02-foundation-content-gap.v1";
pub const XMAP02_FOUNDATION_PRODUCER_CONSUMER: &str = "xmap02_foundation_content_producer";

/// The durable named-gap marker a capture carries when production did not
/// run to a sealed record: the recording still seals, and the file itself
/// names why (never an indistinguishable silent absence).
pub fn foundation_content_gap_document(reason: &str) -> serde_json::Value {
    serde_json::json!({
        "schema": XMAP02_FOUNDATION_CONTENT_GAP_SCHEMA,
        "reason": reason,
    })
}

/// Derive and seal the XMAP02 record against `manifest`. Fails on an
/// unfinalized or invalid manifest; the caller turns that into the durable
/// typed gap marker so partial data still seals with the gap named.
/// Incompletely-admitted families DO seal - their unbound fixed inputs are
/// witnessed in the classification and their NAMES surface on the summary
/// line here, so an incomplete capture is never indistinguishable from a
/// complete one.
pub fn produce_foundation_family_content_set(
    manifest: &ContentManifest,
) -> Result<FoundationFamilyContentSet, String> {
    let classification = derive_foundation_family_classification(manifest)?;
    let required_paths: std::collections::BTreeSet<&str> = manifest
        .required_content
        .iter()
        .map(|row| row.content.normalized_game_relative_path.as_str())
        .collect();
    for closure in portable_content_manifest::foundation_family::FOUNDATION_FAMILY_CLOSURES
        .iter()
    {
        let missing =
            derive_family_missing_paths(closure, |path| required_paths.contains(path));
        if !missing.is_empty() {
            eprintln!(
                "recorder summary: XMAP02 {} seals with {} unbound fixed closure inputs: {}",
                closure.family_id,
                missing.len(),
                missing.join(",")
            );
        }
    }
    Ok(FoundationFamilyContentSet {
        schema: FOUNDATION_FAMILY_CONTENT_SET_SCHEMA.into(),
        manifest_digest_sha256: classification.manifest_digest_sha256,
        classification_sha256: classification.classification_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::{
        ContentLayerKind, ContentManifest, LayerStorageKind, RecordedContentLayer,
        RequiredContent, RequiredContentKind, SourceGameIdentity, SourceProfileIdentity,
    };
    use std::collections::BTreeMap;

    fn sha(value: char) -> String {
        std::iter::repeat(value).take(64).collect()
    }

    /// Minimal valid manifest whose required content covers exactly `paths`.
    fn fixture_manifest(paths: &[&str]) -> ContentManifest {
        let vfs = sha('a');
        let layers = vec![RecordedContentLayer {
            ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            display_name: "Base game".into(),
            descriptor_sha256: None,
            source_container_sha256: None,
            storage_kind: LayerStorageKind::Directory,
            replace_paths: Vec::new(),
            required_for_playback: true,
        }];
        let mut required_content: Vec<RequiredContent> = paths
            .iter()
            .map(|path| RequiredContent {
                content_key: normalize(path),
                kind: RequiredContentKind::Other,
                content: portable_content_manifest::UsedContentKey {
                    effective_vfs_manifest_sha256: vfs.clone(),
                    stored_game_relative_path: normalize(path),
                    normalized_game_relative_path: normalize(path),
                    winning_layer_ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    source_sha256: sha('b'),
                    payload_sha256: sha('c'),
                    payload_bytes: 64,
                },
                observation_count: 1,
                consumers: vec![XMAP02_FOUNDATION_PRODUCER_CONSUMER.to_string()],
            })
            .collect();
        required_content.sort_by(|left, right| left.content_key.cmp(&right.content_key));
        let mut manifest = ContentManifest {
            witnessed_content_namespaces: Vec::new(),
            witnessed_absent_content: Vec::new(),
            schema: portable_content_manifest::SCHEMA.into(),
            product_modset_sha256: sha('e'),
            source_game: SourceGameIdentity {
                version: "1.19.2.0".into(),
                build: "a729".into(),
                executable_sha256: sha('9'),
                build_profile: "hoi4-1.19.2.0-a729".into(),
            },
            source_profile: SourceProfileIdentity {
                profile_id: "profile:fixture".into(),
                playset_id: "playset:fixture".into(),
                selection_sha256: String::new(),
            },
            effective_vfs_manifest_sha256: vfs,
            ordered_layers: layers,
            dlc_selection: Vec::new(),
            localisation_language: "english".into(),
            required_settings: BTreeMap::new(),
            recorder_local_settings: BTreeMap::new(),
            required_content,
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 =
            manifest.computed_selection_sha256().expect("selection");
        manifest.with_computed_digest().expect("sealed fixture")
    }

    fn normalize(path: &str) -> String {
        portable_content_manifest::normalize_game_relative_path(path).expect("path")
    }

    #[test]
    fn produces_sealed_record_bound_to_the_finalized_manifest() {
        let manifest = fixture_manifest(&["map/provinces.bmp", "gfx/fx/border.shader"]);
        let set = produce_foundation_family_content_set(&manifest).expect("produce");
        set.validate().expect("valid");
        let derived =
            portable_content_manifest::foundation_family::derive_foundation_family_classification(
                &manifest,
            )
            .expect("derive");
        assert_eq!(derived.families.len(), 5);
        set.verify_against(&derived).expect("binds its manifest");
    }

    #[test]
    fn incomplete_capture_seals_with_a_different_named_gap_witness() {
        // kills: R4-1 - a capture whose fixed closure inputs were not fully
        // admitted must not seal identically to a complete one, and the names
        // surface on the summary line instead of vanishing.
        use portable_content_manifest::foundation_family::derive_foundation_family_classification;
        let complete = produce_foundation_family_content_set(&fixture_manifest(&[
            "gfx/fx/border.shader",
            "map/provinces.bmp",
            "map/default.map",
            "common/defines/00_graphics.lua",
        ]))
        .expect("complete");
        let partial = produce_foundation_family_content_set(&fixture_manifest(&[
            "gfx/fx/border.shader",
        ]))
        .expect("partial still seals - named gap, not abort");
        assert_ne!(complete.classification_sha256, partial.classification_sha256);
    }

    #[test]
    fn namespace_member_changes_the_sealed_record() {
        // kills: content.borders / content.political_fow reopen condition -
        // history/states belongs to both families; a mod adding state files
        // must change what was recorded for them.
        let without =
            produce_foundation_family_content_set(&fixture_manifest(&["map/provinces.bmp"]))
                .expect("without");
        let with_mod = produce_foundation_family_content_set(&fixture_manifest(&[
            "history/states/1-AAA.txt",
            "map/provinces.bmp",
        ]))
        .expect("with mod");
        assert_ne!(without.classification_sha256, with_mod.classification_sha256);
    }

    #[test]
    fn consumer_can_rederive_and_verify_the_producer_seal() {
        // kills: content.* PRODUCER_missing - playback must be able to
        // reconstruct the classification from the recorded manifest alone and
        // verify it equals what the producer sealed; that equality is the
        // whole consumer join.
        let manifest = fixture_manifest(&["gfx/fx/border.shader"]);
        let sealed = produce_foundation_family_content_set(&manifest).expect("produce");
        let derived =
            portable_content_manifest::foundation_family::derive_foundation_family_classification(
                &manifest,
            )
            .expect("rederive");
        sealed.verify_against(&derived).expect("seal verifies");
    }

    #[test]
    fn production_is_deterministic_for_one_modset() {
        let manifest = fixture_manifest(&["map/provinces.bmp"]);
        let first = produce_foundation_family_content_set(&manifest).expect("first");
        let second = produce_foundation_family_content_set(&manifest).expect("second");
        assert_eq!(first, second);
    }

    #[test]
    fn refuses_unfinalized_manifest() {
        let mut manifest = fixture_manifest(&["map/provinces.bmp"]);
        manifest.manifest_digest_sha256 = None;
        let error =
            produce_foundation_family_content_set(&manifest).err().expect("refused");
        assert!(error.contains("finalized"), "{error}");
    }
}
