//! Playback-installation compatibility diagnostics.
//!
//! Validation laws live in `portable_content_manifest`; this module is only a
//! product-facing formatter. It never guesses, substitutes, or weakens an
//! incompatibility reported by the shared portable-file contract.

use map_viewer::{GameFileResolver, RecordedContentSource};
use portable_content_manifest::{ContentIssueCode, ContentManifest, ContentValidationReport};

use crate::install_paths::ProductPaths;
use crate::timeline::OwnerProvider;

/// Admit the selected installation/profile before any renderer or asset
/// consumer is constructed. The returned resolver embeds the capability, so
/// every subsequent file/directory lookup rejects paths that the portable
/// replay did not record and exact identities that no longer match.
pub fn bind_provider_content(
    provider: &dyn OwnerProvider,
    paths: &ProductPaths,
) -> Result<(GameFileResolver, replay_core::ValidatedContentAdmission), String> {
    let manifest = provider.content_manifest().ok_or_else(|| {
        missing_manifest_summary(&replay_core::ContentValidationReport {
            compatible: false,
            verified_required_content: 0,
            issues: vec![replay_core::ContentIssue {
                code: replay_core::ContentIssueCode::InvalidManifest,
                subject: "manifest.content_manifest".into(),
                message: "recording has no portable used-content manifest".into(),
            }],
        })
    })?;
    // Mount only replay-required layers, in the recorded order. Extra locally
    // enabled but unused DLC/mods must neither be required nor override a used
    // path during playback.
    let resolver = resolver_for_manifest(paths, manifest)?;
    let environment = crate::install_paths::playback_environment_from_paths(
        paths,
        &resolver,
        manifest.required_settings.keys().cloned(),
    )
    .map_err(|error| format!("CONTENT INCOMPATIBLE: {error}"))?;
    let inventory = resolver
        .playback_inventory(manifest, environment)
        .map_err(|error| format!("CONTENT INCOMPATIBLE: {error}"))?;
    let admission = provider
        .admit_playback_content(&inventory)
        .map_err(|report| compatibility_summary(manifest, &report))?;
    let resolver = resolver
        .with_content_admission(&admission)
        .map_err(|error| format!("CONTENT INCOMPATIBLE: {error}"))?;
    Ok((resolver, admission))
}

fn resolver_for_manifest(
    paths: &ProductPaths,
    manifest: &ContentManifest,
) -> Result<GameFileResolver, String> {
    let recorded_layers = manifest
        .ordered_layers
        .iter()
        .filter(|layer| {
            layer.required_for_playback && layer.layer_kind != replay_core::ContentLayerKind::Base
        })
        .map(|layer| RecordedContentSource {
            ordinal: layer.ordinal,
            source_id: layer.layer_id.clone(),
        })
        .collect::<Vec<_>>();
    GameFileResolver::from_recorded_content_layers(
        paths.game_install.clone(),
        paths.user_data.clone(),
        &recorded_layers,
    )
    .map_err(|error| {
        format!("CONTENT INCOMPATIBLE: cannot reconstruct replay-required content order: {error}")
    })
}

/// Single product gate for every renderer consumer after a timeline is open.
/// Callers must not resolve or upload assets before this succeeds.
pub fn admit_selected_content(
    timeline: &replay_core::WorldTimeline,
    inventory: &replay_core::PlaybackContentInventory,
) -> Result<replay_core::ValidatedContentAdmission, String> {
    timeline
        .admit_playback_content(inventory)
        .map_err(|report| match timeline.content_manifest() {
            Some(manifest) => compatibility_summary(manifest, &report),
            None => missing_manifest_summary(&report),
        })
}

pub fn validate_timeline_content(
    timeline: &replay_core::WorldTimeline,
    paths: &ProductPaths,
) -> Result<replay_core::ValidatedContentAdmission, String> {
    let manifest = timeline.content_manifest().ok_or_else(|| {
        missing_manifest_summary(&ContentValidationReport {
            compatible: false,
            verified_required_content: 0,
            issues: vec![replay_core::ContentIssue {
                code: replay_core::ContentIssueCode::InvalidManifest,
                subject: "manifest.content_manifest".into(),
                message: "recording has no portable used-content manifest".into(),
            }],
        })
    })?;
    let resolver = resolver_for_manifest(paths, manifest)?;
    let environment = crate::install_paths::playback_environment_from_paths(
        paths,
        &resolver,
        manifest.required_settings.keys().cloned(),
    )
    .map_err(|error| format!("CONTENT INCOMPATIBLE: {error}"))?;
    let inventory = resolver
        .playback_inventory(manifest, environment)
        .map_err(|error| format!("CONTENT INCOMPATIBLE: {error}"))?;
    let admission = admit_selected_content(timeline, &inventory)?;
    println!(
        "{}",
        compatibility_summary(
            manifest,
            &ContentValidationReport {
                compatible: true,
                verified_required_content: admission.required_paths().count(),
                issues: Vec::new(),
            }
        )
    );
    Ok(admission)
}

pub fn compatibility_summary(
    manifest: &ContentManifest,
    report: &ContentValidationReport,
) -> String {
    let required_dlcs = manifest
        .dlc_selection
        .iter()
        .filter(|row| row.required_for_playback)
        .count();
    let optional_dlcs = manifest
        .dlc_selection
        .iter()
        .filter(|row| !row.required_for_playback)
        .count();
    if report.compatible {
        return format!(
            "CONTENT COMPATIBLE: verified {} required content item(s) and {} required DLC selection(s). {} selected but unused/optional DLC item(s) are recorded for provenance and are not required.",
            report.verified_required_content, required_dlcs, optional_dlcs
        );
    }

    let mut text = format!(
        "CONTENT INCOMPATIBLE: selected playback installation/profile cannot reproduce this replay ({} issue(s)). No missing DLC, mod, file, or setting will be silently substituted.\n",
        report.issues.len()
    );
    for issue in &report.issues {
        text.push_str("  - ");
        text.push_str(issue_label(issue.code.clone()));
        text.push_str(": ");
        text.push_str(&issue.message);
        if !issue.subject.trim().is_empty() {
            text.push_str(" [");
            text.push_str(&issue.subject);
            text.push(']');
        }
        text.push('\n');
    }
    text.push_str(&format!(
        "Required DLC: {required_dlcs}; selected but unused/optional DLC: {optional_dlcs} (optional items do not block playback).\nChange the playback profile with replay_app --configure, then reopen the replay."
    ));
    text
}

fn issue_label(code: ContentIssueCode) -> &'static str {
    match code {
        ContentIssueCode::InvalidManifest => "invalid replay content manifest",
        ContentIssueCode::GameVersionMismatch => "wrong game version",
        ContentIssueCode::ExecutableMismatch => "wrong game executable/build hash",
        ContentIssueCode::BuildProfileMismatch => "wrong game build profile",
        ContentIssueCode::ProfileMismatch => "wrong launcher profile/playset",
        ContentIssueCode::MissingLayer => "missing required mod/DLC layer",
        ContentIssueCode::LayerIdentityMismatch => "wrong mod/DLC identity",
        ContentIssueCode::LayerOrderMismatch => "wrong mod/DLC load order",
        ContentIssueCode::ReplacePathMismatch => "replace_path outcome mismatch",
        ContentIssueCode::RequiredDlcMismatch => "missing or disabled required DLC",
        ContentIssueCode::LanguageMismatch => "material language mismatch",
        ContentIssueCode::SettingMismatch => "material setting mismatch",
        ContentIssueCode::MissingContent => "missing required content",
        ContentIssueCode::WinningLayerMismatch => "wrong VFS override winner",
        ContentIssueCode::SourceHashMismatch => "source container/file hash mismatch",
        ContentIssueCode::PayloadHashMismatch => "resolved payload hash mismatch",
        ContentIssueCode::PayloadLengthMismatch => "resolved payload length mismatch",
    }
}

fn missing_manifest_summary(report: &ContentValidationReport) -> String {
    let mut text = String::from(
        "CONTENT INCOMPATIBLE: replay has no valid portable content manifest. No local content will be guessed or substituted.\n",
    );
    for issue in &report.issues {
        text.push_str("  - ");
        text.push_str(issue_label(issue.code.clone()));
        text.push_str(": ");
        text.push_str(&issue.message);
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::{
        ContentIssue, ContentLayerKind, DlcSelection, LayerStorageKind, RecordedContentLayer,
        SourceGameIdentity, SourceProfileIdentity, SCHEMA,
    };
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    fn manifest() -> ContentManifest {
        ContentManifest {
            schema: SCHEMA.into(),
            product_modset_sha256: "9".repeat(64),
            source_game: SourceGameIdentity {
                version: "1.19.2".into(),
                build: "a729".into(),
                executable_sha256: "a".repeat(64),
                build_profile: "release".into(),
            },
            source_profile: SourceProfileIdentity {
                profile_id: "profile".into(),
                playset_id: "playset".into(),
                selection_sha256: "b".repeat(64),
            },
            effective_vfs_manifest_sha256: "c".repeat(64),
            ordered_layers: vec![RecordedContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                display_name: "the game".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: true,
            }],
            dlc_selection: vec![
                DlcSelection {
                    dlc_id: "used".into(),
                    enabled: true,
                    required_for_playback: true,
                },
                DlcSelection {
                    dlc_id: "unused".into(),
                    enabled: true,
                    required_for_playback: false,
                },
            ],
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::new(),
            required_content: Vec::new(),
            manifest_digest_sha256: None,
        }
    }

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "replay-app-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn launcher_fixture(disable_required_dlc: bool) -> (PathBuf, ProductPaths) {
        let root = temp_root("content-profile");
        let game = root.join("game");
        let userdata = root.join("userdata");
        write(&game, "map/base.txt", "base");
        write(
            &game,
            "dlc/dlc001/dlc001.dlc",
            "name=\"unused corrupt DLC\"\npath=\"missing\"\narchive=\"missing.zip\"",
        );
        write(
            &game,
            "dlc/dlc002/dlc002.dlc",
            "name=\"required replay DLC\"",
        );
        write(&game, "dlc/dlc002/map/winner.txt", "required-dlc");
        write(
            &userdata,
            "mod/extra.mod",
            "name=\"enabled but unused and corrupt\"\npath=\"missing\"\narchive=\"missing.zip\"",
        );
        let disabled = if disable_required_dlc {
            "[\"dlc/dlc002/dlc002.dlc\"]"
        } else {
            "[]"
        };
        write(
            &userdata,
            "dlc_load.json",
            &format!("{{\"enabled_mods\":[\"mod/extra.mod\"],\"disabled_dlcs\":{disabled}}}"),
        );
        (
            root,
            ProductPaths {
                game_install: game,
                user_data: userdata,
            },
        )
    }

    fn required_dlc_manifest() -> ContentManifest {
        let mut value = manifest();
        value.ordered_layers.extend([
            RecordedContentLayer {
                ordinal: 1,
                layer_kind: ContentLayerKind::Dlc,
                layer_id: "dlc/dlc001/dlc001.dlc".into(),
                display_name: "unused local DLC".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: false,
            },
            RecordedContentLayer {
                ordinal: 2,
                layer_kind: ContentLayerKind::Dlc,
                layer_id: "dlc/dlc002/dlc002.dlc".into(),
                display_name: "required replay DLC".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: true,
            },
            RecordedContentLayer {
                ordinal: 4,
                layer_kind: ContentLayerKind::Mod,
                layer_id: "mod/extra.mod".into(),
                display_name: "enabled but unused".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: false,
            },
        ]);
        value
    }

    struct ManifestProvider {
        manifest: ContentManifest,
        hours: Vec<i64>,
    }

    impl OwnerProvider for ManifestProvider {
        fn hours(&self) -> &[i64] {
            &self.hours
        }

        fn owners_at(&self, _hour: i64) -> crate::Result<Arc<BTreeMap<u32, String>>> {
            Ok(Arc::new(BTreeMap::new()))
        }

        fn controllers_at(&self, _hour: i64) -> crate::Result<Arc<BTreeMap<u32, String>>> {
            Ok(Arc::new(BTreeMap::new()))
        }

        fn content_manifest(&self) -> Option<&ContentManifest> {
            Some(&self.manifest)
        }

        fn admit_playback_content(
            &self,
            inventory: &replay_core::PlaybackContentInventory,
        ) -> std::result::Result<
            replay_core::ValidatedContentAdmission,
            replay_core::ContentValidationReport,
        > {
            self.manifest.admit_playback(inventory)
        }
    }

    #[test]
    fn compatible_summary_distinguishes_unused_dlc() {
        let text = compatibility_summary(
            &manifest(),
            &ContentValidationReport {
                compatible: true,
                verified_required_content: 0,
                issues: Vec::new(),
            },
        );
        assert!(text.contains("1 required DLC"), "{text}");
        assert!(
            text.contains("1 selected but unused/optional DLC"),
            "{text}"
        );
        assert!(text.contains("are not required"), "{text}");
    }

    #[test]
    fn incompatible_summary_names_each_fail_closed_class() {
        let codes = [
            ContentIssueCode::GameVersionMismatch,
            ContentIssueCode::MissingLayer,
            ContentIssueCode::RequiredDlcMismatch,
            ContentIssueCode::LayerOrderMismatch,
            ContentIssueCode::ReplacePathMismatch,
            ContentIssueCode::LanguageMismatch,
            ContentIssueCode::SettingMismatch,
            ContentIssueCode::MissingContent,
            ContentIssueCode::WinningLayerMismatch,
            ContentIssueCode::PayloadHashMismatch,
        ];
        let report = ContentValidationReport {
            compatible: false,
            verified_required_content: 0,
            issues: codes
                .into_iter()
                .map(|code| ContentIssue {
                    code,
                    subject: "subject".into(),
                    message: "exact shared-validator detail".into(),
                })
                .collect(),
        };
        let text = compatibility_summary(&manifest(), &report);
        for needle in [
            "wrong game version",
            "missing required mod/DLC layer",
            "missing or disabled required DLC",
            "wrong mod/DLC load order",
            "replace_path outcome mismatch",
            "material language mismatch",
            "material setting mismatch",
            "missing required content",
            "wrong VFS override winner",
            "resolved payload hash mismatch",
        ] {
            assert!(text.contains(needle), "missing {needle:?}: {text}");
        }
        assert!(text.contains("No missing DLC, mod, file, or setting will be silently substituted"));
        assert!(text.contains("--configure"));
    }

    #[test]
    fn playback_resolver_omits_enabled_unused_content_and_preserves_sparse_ordinal() {
        let (root, paths) = launcher_fixture(false);
        let resolver = resolver_for_manifest(&paths, &required_dlc_manifest()).unwrap();
        let sources = resolver.active_sources();
        assert_eq!(sources.len(), 2, "base plus only one replay-required layer");
        assert_eq!(sources[0].source_id, "base");
        assert_eq!(sources[1].source_id, "dlc/dlc002/dlc002.dlc");
        assert_eq!(
            sources[1].ordinal, 2,
            "recorded ordinal must not be renumbered"
        );
        assert_eq!(
            fs::read_to_string(
                resolver
                    .resolve_required("map/winner.txt", "winner")
                    .unwrap()
            )
            .unwrap(),
            "required-dlc",
            "unused enabled mod and DLC must be invisible"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unrelated_enabled_dlc_does_not_substitute_for_disabled_required_dlc() {
        let (root, paths) = launcher_fixture(true);
        let error = resolver_for_manifest(&paths, &required_dlc_manifest()).unwrap_err();
        assert!(error.contains("dlc/dlc002/dlc002.dlc"), "{error}");
        assert!(error.contains("not active locally"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn non_ui_only_manifest_binds_without_ui_catalog_or_unused_ui_content() {
        use sha2::{Digest, Sha256};

        let root = temp_root("non-ui-only");
        let game = root.join("game");
        let userdata = root.join("userdata");
        let payload = b"engine-derived non-ui definition";
        write(
            &game,
            "common/replay/non_ui.txt",
            std::str::from_utf8(payload).unwrap(),
        );
        fs::create_dir_all(game.join("gfx")).unwrap();
        fs::create_dir_all(game.join("map")).unwrap();
        // Deliberately corrupt UI content exists locally but is neither opened
        // nor required by this non-UI replay.
        write(
            &game,
            "interface/unused.gui",
            "containerWindowType = { broken",
        );
        write(
            &game,
            "launcher-settings.json",
            r#"{"gameVersion":"1.19.2.0 (a729)","rawVersion":"1.19.2.0","exePath":"hoi4.exe"}"#,
        );
        fs::write(game.join("hoi4.exe"), b"test executable").unwrap();
        write(
            &userdata,
            "dlc_load.json",
            r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        );
        write(
            &userdata,
            "settings.txt",
            r#"language="l_english"
graphics={
 size={ x=1920 y=1080 }
 min_gui={ x=1280 y=720 }
 gui_scale=1.0
 refreshRate=60
 max_refresh_rate=60
 fullScreen=no
 borderless=yes
}"#,
        );
        let payload_sha = format!("{:x}", Sha256::digest(payload));
        let executable_sha = format!("{:x}", Sha256::digest(b"test executable"));
        let vfs_sha = "4a".repeat(32);
        let mut manifest = ContentManifest {
            schema: SCHEMA.into(),
            product_modset_sha256: "9".repeat(64),
            source_game: SourceGameIdentity {
                version: "1.19.2.0".into(),
                build: "a729".into(),
                executable_sha256: executable_sha,
                build_profile: "hoi4-1.19.2.0-a729".into(),
            },
            source_profile: SourceProfileIdentity {
                profile_id: "recording-provenance".into(),
                playset_id: "recording-provenance".into(),
                selection_sha256: "00".repeat(32),
            },
            effective_vfs_manifest_sha256: vfs_sha.clone(),
            ordered_layers: vec![RecordedContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                display_name: "selected installation".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: true,
            }],
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::new(),
            required_content: vec![replay_core::RequiredContent {
                content_key: "non-ui-definition".into(),
                kind: replay_core::RequiredContentKind::Definition,
                content: replay_core::UsedContentKey {
                    effective_vfs_manifest_sha256: vfs_sha,
                    normalized_game_relative_path: "common/replay/non_ui.txt".into(),
                    winning_layer_ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    source_sha256: payload_sha.clone(),
                    payload_sha256: payload_sha,
                    payload_bytes: payload.len() as u64,
                },
                observation_count: 1,
                consumers: vec!["production-non-ui".into()],
            }],
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
        manifest = manifest.with_computed_digest().unwrap();
        let provider = ManifestProvider {
            manifest,
            hours: vec![0],
        };
        let paths = ProductPaths {
            game_install: game,
            user_data: userdata,
        };
        let (resolver, admission) = bind_provider_content(&provider, &paths).unwrap();
        assert_eq!(admission.required_paths().count(), 1);
        assert!(resolver
            .resolve("common/replay/non_ui.txt")
            .unwrap()
            .is_some());
        assert!(resolver
            .resolve("interface/unused.gui")
            .unwrap_err()
            .to_string()
            .contains("rejects unrecorded/default lookup"));
        fs::remove_dir_all(root).unwrap();
    }
}
