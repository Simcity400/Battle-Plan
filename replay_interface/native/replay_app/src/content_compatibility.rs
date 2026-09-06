
use map_viewer::{GameFileResolver, RecordedContentSource};
use portable_content_manifest::{
    normalize_game_relative_path, ContentIssueCode, ContentManifest, ContentValidationReport,
    UsedContentKey,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use crate::install_paths::ProductPaths;
use crate::timeline::OwnerProvider;

pub const BUNDLE_CONTENT_AUTHORITY_SCHEMA: &str = "hoi4.replay.xmap02-bundle-content-authority.v1";

/// THE XMAP02 CONSUMER JOIN at foundation start: the recorded record must be
/// exactly the classification derived from this recording's own manifest with
/// the shared closures, and every path that classification binds must be
/// carried by THIS machine's admitted playback content. Winner identity rides
/// once in the manifest's required-content rows; the portable admission gate
/// already refuses when the local install resolves any of those rows to a
/// different layer, so a mod that overlapped vanilla at capture must win here
/// too or playback never reaches this point. This join binds the RECORD itself
/// to the local admission on routes where no container-level join ran (e.g.
/// directory-route opens). Recordings without the record (made before the
/// XMAP02 producer existed) join trivially: that state is a named gap, not an
/// error.
pub(crate) fn join_recorded_xmap02_foundation_families(
    recorded: Option<&portable_content_manifest::foundation_family::FoundationFamilyContentSet>,
    admission: &replay_core::ValidatedContentAdmission,
    manifest: &replay_core::ContentManifest,
) -> Result<(), String> {
    use portable_content_manifest::foundation_family::{
        bound_paths_sha256, derive_family_bound_paths, derive_family_missing_paths,
        derive_foundation_family_classification, missing_paths_sha256, FOUNDATION_FAMILY_CLOSURES,
    };
    let Some(set) = recorded else {
        return Ok(());
    };
    set.validate().map_err(|error| {
        format!("CONTENT INCOMPATIBLE: recorded xmap02 foundation content is invalid: {error}")
    })?;
    let declared = manifest.manifest_digest_sha256.as_deref().ok_or(
        "CONTENT INCOMPATIBLE: recording carries xmap02_foundation_content but its manifest          has no finalized digest to join against",
    )?;
    if set.manifest_digest_sha256 != declared {
        return Err(
            "CONTENT INCOMPATIBLE: recording carries xmap02_foundation_content bound to a              different content authority than its own manifest"
                .into(),
        );
    }
    let derived = derive_foundation_family_classification(manifest).map_err(|error| {
        format!(
            "CONTENT INCOMPATIBLE: cannot re-derive recorded xmap02 foundation content: {error}"
        )
    })?;
    set.verify_against(&derived).map_err(|error| {
        format!(
            "CONTENT INCOMPATIBLE: recording's xmap02_foundation_content rejected: {error}"
        )
    })?;
    // Local leg: re-derive each family against THIS machine's admitted
    // playback content; any witness divergence from the record means the
    // capture requires inputs this install does not carry (or admits inputs
    // the capture could not bind).
    let admitted_paths: std::collections::BTreeSet<&str> =
        admission.required_paths().collect();
    for closure in FOUNDATION_FAMILY_CLOSURES.iter() {
        let present = |path: &str| admitted_paths.contains(path);
        let local_bound = derive_family_bound_paths(closure, present, |prefix| {
            let dir = format!("{}/", prefix.trim_end_matches('/'));
            admitted_paths
                .range(dir.as_str()..)
                .take_while(|path| path.starts_with(dir.as_str()))
                .map(|path| (*path).to_string())
                .collect::<Vec<String>>()
        });
        let local_missing = derive_family_missing_paths(closure, present);
        let row = derived.family(closure.family_id).ok_or_else(|| {
            format!(
                "CONTENT INCOMPATIBLE: xmap02 record lost family {}",
                closure.family_id
            )
        })?;
        if bound_paths_sha256(&local_bound) != row.bound_paths_sha256
            || missing_paths_sha256(&local_missing) != row.missing_paths_sha256
        {
            return Err(format!(
                "CONTENT INCOMPATIBLE: {} requires inputs this machine's admitted playback                  content does not carry (or admits inputs the capture could not bind)",
                closure.family_id
            ));
        }
    }
    Ok(())
}

/// Mechanical agreement between a generated map bundle and the exact content
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleContentAuthority {
    pub schema: String,
    pub manifest_digest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub bundle_modset_id: String,
    pub bundle_mod_count: usize,
    pub ordered_inputs: Vec<UsedContentKey>,
    pub authority_sha256: String,
}

/// external generated `MapBundle` is never a prerequisite for this gate.
pub fn derive_bundle_content_authority(
    manifest: &ContentManifest,
    resolver: &GameFileResolver,
    admission: &replay_core::ValidatedContentAdmission,
) -> Result<BundleContentAuthority, String> {
    manifest
        .validate()
        .map_err(|error| format!("CONTENT INCOMPATIBLE: invalid manifest: {error}"))?;
    let manifest_digest = manifest
        .computed_digest()
        .map_err(|error| format!("CONTENT INCOMPATIBLE: manifest digest: {error}"))?;
    if admission.manifest_digest_sha256() != manifest_digest
        || admission.product_modset_sha256() != manifest.product_modset_sha256
    {
        return Err(
            "CONTENT INCOMPATIBLE: content admission authority tuple disagrees with manifest"
                .into(),
        );
    }
    let recorded_layer_ids = manifest
        .ordered_layers
        .iter()
        .filter(|layer| {
            layer.required_for_playback && layer.layer_kind != replay_core::ContentLayerKind::Base
        })
        .map(|layer| layer.layer_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mounted_layer_ids = resolver
        .active_sources()
        .into_iter()
        .filter(|source| source.kind != "vanilla" && source.kind != "base")
        .map(|source| source.source_id)
        .collect::<std::collections::BTreeSet<_>>();
    if mounted_layer_ids != recorded_layer_ids {
        let missing = recorded_layer_ids
            .difference(&mounted_layer_ids)
            .cloned()
            .collect::<Vec<_>>();
        let extra = mounted_layer_ids
            .difference(&recorded_layer_ids)
            .cloned()
            .collect::<Vec<_>>();
        return Err(format!(
            "CONTENT INCOMPATIBLE: mounted resolver {} and recording do not name one exact content roster:              {} mounted vs {} required; not mounted here: {:?}; mounted but not recorded: {:?}",
            resolver.bundle_modset_id(),
            mounted_layer_ids.len(),
            recorded_layer_ids.len(),
            missing,
            extra
        ));
    }
    let mut ordered_inputs = Vec::new();
    let mut effective_vfs = None;
    for path in admission.required_paths() {
        let key = admission
            .require_used_content(path)
            .map_err(|error| format!("CONTENT INCOMPATIBLE: {error}"))?
            .clone();
        resolver
            .read_admitted_used_content(&key)
            .map_err(|error| format!("CONTENT INCOMPATIBLE: {error}"))?;
        match &effective_vfs {
            Some(identity) if identity != &key.effective_vfs_manifest_sha256 => {
                return Err(
                    "CONTENT INCOMPATIBLE: bundle inputs span different effective VFS identities"
                        .into(),
                )
            }
            None => effective_vfs = Some(key.effective_vfs_manifest_sha256.clone()),
            _ => {}
        }
        ordered_inputs.push(key);
    }
    let effective_vfs_manifest_sha256 = effective_vfs
        .ok_or_else(|| "CONTENT INCOMPATIBLE: bundle has no admitted source inputs".to_string())?;
    let mut authority = BundleContentAuthority {
        schema: BUNDLE_CONTENT_AUTHORITY_SCHEMA.into(),
        manifest_digest_sha256: manifest_digest,
        product_modset_sha256: manifest.product_modset_sha256.clone(),
        effective_vfs_manifest_sha256,
        bundle_modset_id: resolver.bundle_modset_id(),
        bundle_mod_count: resolver.bundle_mod_count(),
        ordered_inputs,
        authority_sha256: String::new(),
    };
    authority.authority_sha256 = bundle_authority_sha256(&authority)?;
    Ok(authority)
}

/// Optional evidence/debug comparison for a supplied generated map bundle.
pub fn bind_bundle_provenance(
    provenance: &crate::bundle::BundleProvenance,
    manifest: &ContentManifest,
    resolver: &GameFileResolver,
    admission: &replay_core::ValidatedContentAdmission,
) -> Result<BundleContentAuthority, String> {
    let authority = derive_bundle_content_authority(manifest, resolver, admission)?;
    if provenance.mod_count != authority.bundle_mod_count
        || provenance.modset_id != authority.bundle_modset_id
    {
        return Err(format!(
            "CONTENT INCOMPATIBLE: supplied bundle modset {} ({} mods) disagrees with canonical recording authority {} ({} mods)",
            provenance.modset_id,
            provenance.mod_count,
            authority.bundle_modset_id,
            authority.bundle_mod_count
        ));
    }
    let mut expected_fnv = BTreeMap::new();
    expected_fnv.insert(
        normalize_game_relative_path("map/definition.csv")?,
        provenance.definition_csv_fnv1a64.clone(),
    );
    expected_fnv.insert(
        normalize_game_relative_path("map/provinces.bmp")?,
        provenance.provinces_bmp_fnv1a64.clone(),
    );
    for (path, hash) in &provenance.visual_input_fnv1a64 {
        let path = normalize_game_relative_path(path)?;
        if let Some(previous) = expected_fnv.insert(path.clone(), hash.clone()) {
            if previous != *hash {
                return Err(format!(
                    "CONTENT INCOMPATIBLE: bundle gives conflicting identities for {path}"
                ));
            }
        }
    }
    for (path, expected_hash) in expected_fnv {
        let key = admission
            .require_used_content(&path)
            .map_err(|error| format!("CONTENT INCOMPATIBLE: {error}"))?;
        let bytes = resolver
            .read_admitted_used_content(key)
            .map_err(|error| format!("CONTENT INCOMPATIBLE: {error}"))?;
        let actual_hash = fnv1a64_hex(&bytes);
        if actual_hash != expected_hash {
            return Err(format!(
                "CONTENT INCOMPATIBLE: supplied map bundle input {path} was generated from {expected_hash}, but the admitted effective VFS contains {actual_hash}"
            ));
        }
    }
    Ok(authority)
}

fn bundle_authority_sha256(authority: &BundleContentAuthority) -> Result<String, String> {
    let mut unsigned = authority.clone();
    unsigned.authority_sha256.clear();
    let bytes = serde_json::to_vec(&unsigned)
        .map_err(|error| format!("encode bundle content authority: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn fnv1a64_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

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
            excluded_engine_written_sinks: Vec::new(),
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
    let admission = provider
        .admit_playback_content(&inventory)
        .map_err(|report| compatibility_summary(manifest, &report))?;
    let resolver = resolver
        .with_content_admission(&admission)
        .map_err(|error| format!("CONTENT INCOMPATIBLE: {error}"))?;
    Ok((resolver, admission))
}

pub(super) fn resolver_for_manifest(
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
            excluded_engine_written_sinks: Vec::new(),
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
    let (_terrain_options, render_settings_pin) =
        crate::install_paths::playback_render_settings(paths, &manifest.required_settings)
            .map_err(|error| error.to_string())?;
    println!(
        "{}",
        crate::game_file_ledger::open_statement(
            &admission,
            timeline.product_admission(),
            &render_settings_pin,
            &crate::install_paths::playback_map_sidecar_probe(paths, &resolver.mounted_layer_roots()),
            &manifest.localisation_language,
        )
    );
    println!(
        "{}",
        compatibility_summary(
            manifest,
            &ContentValidationReport {
                compatible: true,
                verified_required_content: admission.required_paths().count(),
                issues: Vec::new(),
                // From the ADMISSION, never an empty placeholder: the success
                // path has no validation report of its own (a compatible one is
                // consumed by `admit_playback`), and synthesizing one with an
                // empty list silently un-named every exclusion in the only case
                // where playback actually proceeds.
                excluded_engine_written_sinks: admission
                    .excluded_engine_written_sinks()
                    .to_vec(),
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
        let mut text = format!(
            "CONTENT COMPATIBLE: verified {} required content item(s) and {} required DLC selection(s). {} selected but unused/optional DLC item(s) are recorded for provenance and are not required.",
            report.verified_required_content, required_dlcs, optional_dlcs
        );
        // "Compatible" must never quietly mean "compatible except for some
        // files nobody mentioned". These rows were recorded by an older
        // capture, before the content census stopped collecting the engine's
        // own scratch files; the reader is told which ones went uncompared and
        // why, rather than the difference disappearing into the verified count.
        if !report.excluded_engine_written_sinks.is_empty() {
            text.push_str(&format!(
                "\n{} recorded item(s) were NOT compared because the game writes them itself and never reads them as content ({}). They are not part of this replay's content identity.",
                report.excluded_engine_written_sinks.len(),
                report.excluded_engine_written_sinks.join(", ")
            ));
        }
        return text;
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
        "Required DLC: {required_dlcs}; selected but unused/optional DLC: {optional_dlcs} (optional items do not block playback)."
    ));
    if report
        .issues
        .iter()
        .any(|issue| issue.code == ContentIssueCode::InvalidPlaybackInventory)
    {
        text.push_str(
            "\nThe local installation/profile inventory is itself malformed (not the replay): repair or reselect the installation, then reopen the replay.",
        );
    }
    let only_invalid_manifest = !report.issues.is_empty()
        && report
            .issues
            .iter()
            .all(|issue| issue.code == ContentIssueCode::InvalidManifest);
    if only_invalid_manifest {
        text.push_str(
            "\nThe replay's own content manifest is malformed: no local installation or profile can play it back, and reconfiguring will not help. Obtain an intact recording.",
        );
        return text;
    }
    if report
        .issues
        .iter()
        .any(|issue| issue.code == ContentIssueCode::InvalidManifest)
    {
        text.push_str(
            "\nThe replay's own content manifest is malformed: that part cannot be fixed by any local installation or profile.",
        );
    }
    text.push_str(
        "\nChange the playback profile with replay_app --configure, then reopen the replay.",
    );
    text
}

fn issue_label(code: ContentIssueCode) -> &'static str {
    match code {
        ContentIssueCode::InvalidManifest => "invalid replay content manifest",
        ContentIssueCode::InvalidPlaybackInventory => "invalid local content inventory",
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
        ContentIssueCode::StoredPathMismatch => "stored content path spelling mismatch",
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
        RequiredContent, RequiredContentKind, SourceGameIdentity, SourceProfileIdentity,
        UsedContentKey, SCHEMA,
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
            recorder_local_settings: BTreeMap::new(),
            required_content: Vec::new(),
            witnessed_absent_content: Vec::new(),
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
    fn bundle_content_authority_hash_is_canonical_and_mutation_sensitive() {
        assert_eq!(fnv1a64_hex(b"hello"), "a430d84680aabd0b");
        let mut authority = BundleContentAuthority {
            schema: BUNDLE_CONTENT_AUTHORITY_SCHEMA.into(),
            manifest_digest_sha256: "a".repeat(64),
            product_modset_sha256: "b".repeat(64),
            effective_vfs_manifest_sha256: "c".repeat(64),
            bundle_modset_id: "vanilla".into(),
            bundle_mod_count: 0,
            ordered_inputs: Vec::new(),
            authority_sha256: String::new(),
        };
        let first = bundle_authority_sha256(&authority).unwrap();
        assert_eq!(first, bundle_authority_sha256(&authority).unwrap());
        authority.bundle_mod_count = 1;
        assert_ne!(first, bundle_authority_sha256(&authority).unwrap());
    }

    #[test]
    fn compatible_summary_distinguishes_unused_dlc() {
        let text = compatibility_summary(
            &manifest(),
            &ContentValidationReport {
                compatible: true,
                verified_required_content: 0,
                issues: Vec::new(),
                excluded_engine_written_sinks: Vec::new(),
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
            ContentIssueCode::InvalidManifest,
            ContentIssueCode::InvalidPlaybackInventory,
            ContentIssueCode::ExecutableMismatch,
            ContentIssueCode::BuildProfileMismatch,
            ContentIssueCode::ProfileMismatch,
            ContentIssueCode::LayerIdentityMismatch,
            ContentIssueCode::SourceHashMismatch,
            ContentIssueCode::PayloadLengthMismatch,
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
            excluded_engine_written_sinks: Vec::new(),
        };
        let text = compatibility_summary(&manifest(), &report);
        for needle in [
            "invalid replay content manifest",
            "invalid local content inventory",
            "The local installation/profile inventory is itself malformed",
            "The replay's own content manifest is malformed",
            "wrong game executable/build hash",
            "wrong game build profile",
            "wrong launcher profile/playset",
            "wrong mod/DLC identity",
            "source container/file hash mismatch",
            "resolved payload length mismatch",
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
    fn a_manifest_only_failure_does_not_advise_reconfiguring_the_profile() {
        let report = ContentValidationReport {
            compatible: false,
            verified_required_content: 0,
            issues: vec![ContentIssue {
                code: ContentIssueCode::InvalidManifest,
                subject: "manifest".into(),
                message: "content manifest digest mismatch".into(),
            }],
            excluded_engine_written_sinks: Vec::new(),
        };
        let text = compatibility_summary(&manifest(), &report);
        assert!(text.contains("invalid replay content manifest"), "{text}");
        assert!(text.contains("reconfiguring will not help"), "{text}");
        assert!(
            !text.contains("--configure"),
            "the profile remediation contradicts the sentence above it: {text}"
        );

        let mixed = ContentValidationReport {
            compatible: false,
            verified_required_content: 0,
            issues: vec![
                ContentIssue {
                    code: ContentIssueCode::InvalidManifest,
                    subject: "manifest".into(),
                    message: "content manifest digest mismatch".into(),
                },
                ContentIssue {
                    code: ContentIssueCode::RequiredDlcMismatch,
                    subject: "dlc:used".into(),
                    message: "required enabled state is true".into(),
                },
            ],
            excluded_engine_written_sinks: Vec::new(),
        };
        let text = compatibility_summary(&manifest(), &mixed);
        assert!(text.contains("--configure"), "{text}");
    }

    /// A recording made before the content census stopped collecting the
    /// engine's own write sinks still carries rows for them, and the validator
    /// no longer compares those rows — otherwise the game touching its own
    /// `console_history.txt` refuses the replay over a file nothing reads.
    ///
    /// The person opening the replay is told. "CONTENT COMPATIBLE" must not
    /// quietly mean "compatible except for two files nobody mentioned", so the
    /// summary names the uncompared items and says why.
    ///
    /// kills: printing the compatible summary from `verified_required_content`
    /// alone and dropping `excluded_engine_written_sinks` on the floor.
    #[test]
    fn a_compatible_summary_names_the_content_it_did_not_compare() {
        let clean = ContentValidationReport {
            compatible: true,
            verified_required_content: 7,
            issues: Vec::new(),
            excluded_engine_written_sinks: Vec::new(),
        };
        let text = compatibility_summary(&manifest(), &clean);
        assert!(text.contains("CONTENT COMPATIBLE"), "{text}");
        assert!(
            !text.contains("NOT compared"),
            "a recording with nothing excluded must not claim an exclusion: {text}"
        );

        let with_sinks = ContentValidationReport {
            compatible: true,
            verified_required_content: 7,
            issues: Vec::new(),
            excluded_engine_written_sinks: vec![
                "console_history.txt".into(),
                "imgui.ini".into(),
            ],
        };
        let text = compatibility_summary(&manifest(), &with_sinks);
        assert!(text.contains("CONTENT COMPATIBLE"), "{text}");
        assert!(text.contains("2 recorded item(s) were NOT compared"), "{text}");
        assert!(text.contains("console_history.txt"), "{text}");
        assert!(text.contains("imgui.ini"), "{text}");
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
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unrelated_enabled_dlc_does_not_substitute_for_disabled_required_dlc() {
        let (root, paths) = launcher_fixture(true);
        let error = resolver_for_manifest(&paths, &required_dlc_manifest()).unwrap_err();
        assert!(error.contains("dlc/dlc002/dlc002.dlc"), "{error}");
        assert!(error.contains("not active locally"), "{error}");
        portable_test_fs_retry::remove_dir_all(root).unwrap();
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
            recorder_local_settings: BTreeMap::new(),
            required_content: vec![replay_core::RequiredContent {
                content_key: "non-ui-definition".into(),
                kind: replay_core::RequiredContentKind::Definition,
                content: replay_core::UsedContentKey {
                    effective_vfs_manifest_sha256: vfs_sha,
                    stored_game_relative_path: "common/replay/non_ui.txt".into(),
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
            witnessed_absent_content: Vec::new(),
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
        let authority =
            derive_bundle_content_authority(&provider.manifest, &resolver, &admission).unwrap();
        assert_eq!(authority.bundle_modset_id, "vanilla");
        assert_eq!(authority.bundle_mod_count, 0);
        assert_eq!(authority.ordered_inputs.len(), 1);
        assert_eq!(
            authority.ordered_inputs[0].normalized_game_relative_path,
            "common/replay/non_ui.txt"
        );
        assert_eq!(authority.authority_sha256.len(), 64);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    /// XMAP02 join fixture: a finalized manifest whose required content
    /// carries exactly `paths`, plus its admission and the sealed record the
    /// producer would emit for it.
    fn xmap02_join_fixture(
        paths: &[&str],
    ) -> (
        replay_core::ValidatedContentAdmission,
        ContentManifest,
        portable_content_manifest::foundation_family::FoundationFamilyContentSet,
    ) {
        use portable_content_manifest::foundation_family::FOUNDATION_FAMILY_CLOSURES;
        let mut base = manifest();
        base.required_content = paths
            .iter()
            .map(|path| {
                let normalized = portable_content_manifest::normalize_game_relative_path(path)
                    .unwrap();
                RequiredContent {
                    content_key: normalized.clone(),
                    kind: RequiredContentKind::ShaderSource,
                    content: UsedContentKey {
                        effective_vfs_manifest_sha256: "c".repeat(64),
                        stored_game_relative_path: normalized.clone(),
                        normalized_game_relative_path: normalized,
                        winning_layer_ordinal: 0,
                        layer_kind: ContentLayerKind::Base,
                        layer_id: "base".into(),
                        source_sha256: "d".repeat(64),
                        payload_sha256: "e".repeat(64),
                        payload_bytes: 32,
                    },
                    observation_count: 1,
                    consumers: vec!["xmap02_join_fixture".into()],
                }
            })
            .collect();
        base.source_profile.selection_sha256 = base.computed_selection_sha256().unwrap();
        let manifest = base.with_computed_digest().unwrap();
        let inventory = replay_core::PlaybackContentInventory {
            source_game: manifest.source_game.clone(),
            source_profile: manifest.source_profile.clone(),
            layers: manifest
                .ordered_layers
                .iter()
                .filter(|layer| layer.required_for_playback)
                .map(|layer| replay_core::PlaybackContentLayer {
                    recorded_ordinal: layer.ordinal,
                    layer_kind: layer.layer_kind,
                    layer_id: layer.layer_id.clone(),
                    descriptor_sha256: layer.descriptor_sha256.clone(),
                    source_container_sha256: layer.source_container_sha256.clone(),
                    storage_kind: layer.storage_kind,
                    replace_paths: Vec::new(),
                })
                .collect(),
            dlc_selection: manifest.dlc_selection.clone(),
            localisation_language: manifest.localisation_language.clone(),
            settings: BTreeMap::new(),
            resolved_content: manifest
                .required_content
                .iter()
                .map(|row| portable_content_manifest::ResolvedPlaybackContent {
                    stored_game_relative_path: row.content.stored_game_relative_path.clone(),
                    normalized_game_relative_path: row
                        .content
                        .normalized_game_relative_path
                        .clone(),
                    winning_layer_ordinal: row.content.winning_layer_ordinal,
                    layer_kind: row.content.layer_kind,
                    layer_id: row.content.layer_id.clone(),
                    source_sha256: row.content.source_sha256.clone(),
                    payload_sha256: row.content.payload_sha256.clone(),
                    payload_bytes: row.content.payload_bytes,
                })
                .collect(),
        };
        let admission = manifest.admit_playback(&inventory).unwrap();
        let derived =
            portable_content_manifest::foundation_family::derive_foundation_family_classification(
                &manifest,
            )
            .unwrap();
        assert_eq!(derived.families.len(), FOUNDATION_FAMILY_CLOSURES.len());
        let set = portable_content_manifest::foundation_family::FoundationFamilyContentSet {
            schema:
                portable_content_manifest::foundation_family::FOUNDATION_FAMILY_CONTENT_SET_SCHEMA
                    .into(),
            manifest_digest_sha256: derived.manifest_digest_sha256.clone(),
            classification_sha256: derived.classification_sha256,
        };
        set.validate().unwrap();
        (admission, manifest, set)
    }

    #[test]
    fn recorded_xmap02_seal_matching_this_manifest_joins() {
        let (admission, manifest, set) = xmap02_join_fixture(&[
            "gfx/fx/border.shader",
            "map/provinces.bmp",
            "history/states/1.txt",
        ]);
        join_recorded_xmap02_foundation_families(Some(&set), &admission, &manifest)
            .expect("matching seal joins");
    }

    #[test]
    fn recorded_xmap02_set_bound_to_another_manifest_refuses() {
        // kills: content.* reopen condition - a record sealed against one
        // content authority must not ride a recording whose manifest is
        // another; playback re-derives from its own manifest and refuses on
        // any divergence.
        let (_own_admission, own_manifest, set) =
            xmap02_join_fixture(&["gfx/fx/border.shader"]);
        let _ = own_manifest;
        let (foreign_admission, foreign_manifest, _foreign_set) = xmap02_join_fixture(&[
            "gfx/fx/border.shader",
            "history/states/9.txt",
        ]);
        let error = join_recorded_xmap02_foundation_families(
            Some(&set),
            &foreign_admission,
            &foreign_manifest,
        )
        .err()
        .expect("foreign authority refused");
        assert!(
            error.contains("different content authority")
                || error.contains("does not match"),
            "{error}"
        );
    }

    #[test]
    fn tampered_xmap02_record_refuses_before_any_join() {
        // kills: R2-2 - even with deny-unknown decode, a mutated digest row or
        // stale seal must refuse inside the join itself.
        let (admission, manifest, mut set) =
            xmap02_join_fixture(&["gfx/fx/border.shader"]);
        set.classification_sha256 = "f".repeat(64);
        let error = join_recorded_xmap02_foundation_families(Some(&set), &admission, &manifest)
            .err()
            .expect("tampered record refused");
        assert!(
            error.contains("rejected") || error.contains("invalid"),
            "{error}"
        );
    }

    #[test]
    fn absent_xmap02_record_is_a_named_gap_not_an_error() {
        let (admission, manifest, _set) = xmap02_join_fixture(&["gfx/fx/border.shader"]);
        join_recorded_xmap02_foundation_families(None, &admission, &manifest)
            .expect("pre-producer recordings open");
    }
}
