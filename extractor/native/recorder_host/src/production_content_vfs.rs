//! Recorder-owned physical mounting for the neutral portable effective VFS.
//!
//! Local launch roots are admitted only long enough to build exact mounts.
//! They never enter the portable replay or any neutral schema.

use crate::portable_content_manifest::{
    descriptor_replace_paths, descriptor_scalar, latin1_bytes_to_string,
    normalize_stable_layer_id, semantic_descriptor_sha256, PortableLaunchBinding,
    RuntimeContentRoots,
};
use crate::modset::ModsetReport;
use portable_content_manifest::effective_vfs::{EffectiveVfs, EffectiveVfsLayerMount};
use portable_content_manifest::{ContentLayerKind, LayerStorageKind, RecordedContentLayer};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const A729_EXE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
const D49_STRATEGIC_REGION_PREFIX: &str = "map/strategicregions";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct D49StrategicRegionFileAuthority {
    pub logical_path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct D49BaseOnlyStrategicRegionAuthority {
    pub schema: &'static str,
    pub source: &'static str,
    pub exact_exe_sha256: String,
    pub install_root: PathBuf,
    pub inventory_sha256: String,
    pub files: Vec<D49StrategicRegionFileAuthority>,
}

/// Admit the base strategic-region directory for an ordinary attach only when
/// the launch binding proves zero mods and every enabled/integrated DLC is
/// independently proven not to replace or contribute that namespace. This is
/// intentionally narrower than the full H4CT effective-VFS path: any non-base
/// contribution fails closed instead of guessing DLC priority.
pub fn d49_base_only_authority_from_resolved_zero_mod_launch(
    report: &ModsetReport,
) -> Result<D49BaseOnlyStrategicRegionAuthority, String> {
    if !report.resolved || !report.enabled_mods.is_empty() {
        return Err("D49 base-only authority requires a resolved zero-mod launch".into());
    }
    let exact_exe_sha256 = report
        .exe_sha256
        .as_deref()
        .ok_or("D49 base-only authority omitted executable identity")?;
    if exact_exe_sha256 != A729_EXE_SHA256 {
        return Err("D49 base-only authority requires the exact a729 executable".into());
    }
    let roots = RuntimeContentRoots::from_modset(report)?;
    let launch = PortableLaunchBinding::from_modset(report)?;
    let base = RecordedContentLayer {
        ordinal: 0,
        layer_kind: ContentLayerKind::Base,
        layer_id: "base".into(),
        display_name: "base".into(),
        descriptor_sha256: None,
        source_container_sha256: None,
        storage_kind: LayerStorageKind::Directory,
        replace_paths: Vec::new(),
        required_for_playback: false,
    };

    for bound in launch.integrated_dlc_layers.values() {
        prove_layer_does_not_affect_d49(
            &roots,
            &base,
            ContentLayerKind::IntegratedDlc,
            &bound.layer_id,
            &bound.descriptor_sha256,
            bound.source_container_sha256.clone(),
            bound.storage_kind,
            exact_exe_sha256,
        )?;
    }
    for bound in launch.enabled_dlc_layers.values() {
        prove_layer_does_not_affect_d49(
            &roots,
            &base,
            ContentLayerKind::Dlc,
            &bound.layer_id,
            &bound.descriptor_sha256,
            bound.source_container_sha256.clone(),
            bound.storage_kind,
            exact_exe_sha256,
        )?;
    }

    let base_vfs = effective_vfs_from_launch(&roots, std::slice::from_ref(&base), exact_exe_sha256)?;
    let mut resolved = base_vfs.list_recursive(D49_STRATEGIC_REGION_PREFIX, Some("txt"))?;
    resolved.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    if resolved.is_empty() || resolved.len() > 16_384 {
        return Err("D49 base strategic-region inventory is empty or unbounded".into());
    }
    let files = resolved
        .into_iter()
        .map(|file| D49StrategicRegionFileAuthority {
            logical_path: file.logical_path,
            bytes: file.bytes.len() as u64,
            sha256: file.content.payload_sha256,
        })
        .collect::<Vec<_>>();
    let inventory_bytes = serde_json::to_vec(&files)
        .map_err(|error| format!("serialize D49 base-only inventory failed: {error}"))?;
    let inventory_sha256 = format!("{:x}", Sha256::digest(&inventory_bytes));
    Ok(D49BaseOnlyStrategicRegionAuthority {
        schema: "hoi4.d49-base-only-strategic-regions.v1",
        source: "resolved_zero_mod_base_only_effective_vfs",
        exact_exe_sha256: exact_exe_sha256.into(),
        install_root: roots.install_root,
        inventory_sha256,
        files,
    })
}

#[allow(clippy::too_many_arguments)]
fn prove_layer_does_not_affect_d49(
    roots: &RuntimeContentRoots,
    base: &RecordedContentLayer,
    layer_kind: ContentLayerKind,
    layer_id: &str,
    descriptor_sha256: &str,
    source_container_sha256: Option<String>,
    storage_kind: LayerStorageKind,
    authority_sha256: &str,
) -> Result<(), String> {
    let descriptor = stable_relative_join(&roots.install_root, layer_id)?;
    let provisional = RecordedContentLayer {
        ordinal: 1,
        layer_kind,
        layer_id: layer_id.into(),
        display_name: layer_id.into(),
        descriptor_sha256: Some(descriptor_sha256.into()),
        source_container_sha256,
        storage_kind,
        replace_paths: Vec::new(),
        required_for_playback: false,
    };
    let text = verify_descriptor(&provisional, &descriptor)?;
    let replace_paths = descriptor_replace_paths(&text);
    if replace_paths
        .iter()
        .any(|path| path.eq_ignore_ascii_case(D49_STRATEGIC_REGION_PREFIX))
    {
        return Err(format!(
            "D49 base-only authority rejected {} because it replaces {}",
            layer_id, D49_STRATEGIC_REGION_PREFIX
        ));
    }
    let local_source = match layer_kind {
        ContentLayerKind::IntegratedDlc => descriptor
            .parent()
            .ok_or("integrated DLC descriptor has no containing directory")?
            .to_path_buf(),
        ContentLayerKind::Dlc => descriptor_content_source(
            &roots.install_root,
            &descriptor,
            &text,
            storage_kind,
            "DLC",
        )?,
        _ => return Err("D49 non-base proof received an unsupported layer kind".into()),
    };
    let layer = RecordedContentLayer {
        replace_paths,
        ..provisional
    };
    let mounts = [base.clone(), layer.clone()];
    let vfs = effective_vfs_from_launch(roots, &mounts, authority_sha256)?;
    let rows = vfs.list_recursive(D49_STRATEGIC_REGION_PREFIX, Some("txt"))?;
    if rows
        .iter()
        .any(|row| row.content.winning_layer_ordinal == layer.ordinal)
    {
        return Err(format!(
            "D49 base-only authority rejected {} because it contributes {}",
            layer_id, D49_STRATEGIC_REGION_PREFIX
        ));
    }
    // `effective_vfs_from_launch` derives the same path, but retaining this
    // value here ensures the layer's authored source was resolved and checked
    // even when the target namespace is empty.
    let _ = local_source;
    Ok(())
}

pub fn effective_vfs_from_launch(
    roots: &RuntimeContentRoots,
    layers: &[RecordedContentLayer],
    authority_manifest_sha256: &str,
) -> Result<EffectiveVfs, String> {
    if layers.is_empty() {
        return Err("production effective VFS has no recorded content layers".into());
    }
    let mut mounts = Vec::with_capacity(layers.len());
    for (position, layer) in layers.iter().enumerate() {
        if layer.ordinal as usize != position {
            return Err("production effective VFS requires contiguous recorded ordinals".into());
        }
        let local_source = match layer.layer_kind {
            ContentLayerKind::Base => {
                if layer.ordinal != 0
                    || layer.layer_id != "base"
                    || layer.descriptor_sha256.is_some()
                    || layer.storage_kind != LayerStorageKind::Directory
                {
                    return Err("production effective VFS base identity is invalid".into());
                }
                roots.install_root.clone()
            }
            ContentLayerKind::IntegratedDlc => {
                let descriptor = stable_relative_join(&roots.install_root, &layer.layer_id)?;
                verify_descriptor(layer, &descriptor)?;
                if layer.storage_kind != LayerStorageKind::Directory
                    || layer.source_container_sha256.is_some()
                {
                    return Err(
                        "integrated DLC must mount its physical descriptor directory".into(),
                    );
                }
                descriptor
                    .parent()
                    .ok_or("integrated DLC descriptor has no containing directory")?
                    .to_path_buf()
            }
            ContentLayerKind::Dlc => {
                let descriptor = stable_relative_join(&roots.install_root, &layer.layer_id)?;
                let text = verify_descriptor(layer, &descriptor)?;
                descriptor_content_source(
                    &roots.install_root,
                    &descriptor,
                    &text,
                    layer.storage_kind,
                    "DLC",
                )?
            }
            ContentLayerKind::Mod => {
                let descriptor = stable_relative_join(&roots.user_dir, &layer.layer_id)?;
                let text = verify_descriptor(layer, &descriptor)?;
                descriptor_content_source(
                    &roots.user_dir,
                    &descriptor,
                    &text,
                    layer.storage_kind,
                    "mod",
                )?
            }
        };
        mounts.push(EffectiveVfsLayerMount {
            ordinal: layer.ordinal,
            layer_kind: layer.layer_kind,
            layer_id: layer.layer_id.clone(),
            storage_kind: layer.storage_kind,
            local_source,
            source_container_sha256: layer.source_container_sha256.clone(),
            replace_paths: layer.replace_paths.clone(),
        });
    }
    EffectiveVfs::new(authority_manifest_sha256, mounts)
}

fn verify_descriptor(layer: &RecordedContentLayer, descriptor: &Path) -> Result<String, String> {
    let expected = layer.descriptor_sha256.as_deref().ok_or_else(|| {
        format!(
            "recorded layer {} omitted descriptor identity",
            layer.layer_id
        )
    })?;
    let first = fs::read(descriptor).map_err(|error| {
        format!(
            "read recorded layer descriptor {} failed: {error}",
            descriptor.display()
        )
    })?;
    let second = fs::read(descriptor).map_err(|error| {
        format!(
            "re-read recorded layer descriptor {} failed: {error}",
            descriptor.display()
        )
    })?;
    if first != second {
        return Err(format!(
            "recorded layer descriptor changed while mounting: {}",
            descriptor.display()
        ));
    }
    let text = latin1_bytes_to_string(&first);
    if semantic_descriptor_sha256(&text) != expected {
        return Err(format!(
            "recorded layer descriptor semantic identity drifted: {}",
            descriptor.display()
        ));
    }
    Ok(text)
}

fn descriptor_content_source(
    relative_root: &Path,
    descriptor: &Path,
    text: &str,
    expected_storage: LayerStorageKind,
    label: &str,
) -> Result<PathBuf, String> {
    let archive = descriptor_scalar(text, "archive");
    let path = descriptor_scalar(text, "path");
    if archive.is_some() && path.is_some() {
        return Err(format!("{label} descriptor has both path and archive"));
    }
    match expected_storage {
        LayerStorageKind::ZipArchive => {
            let authored = archive.ok_or_else(|| {
                format!("recorded archive {label} descriptor has no archive field")
            })?;
            resolve_authored_source(relative_root, descriptor, &authored, true)
        }
        LayerStorageKind::Directory => {
            if archive.is_some() {
                return Err(format!(
                    "recorded directory {label} descriptor names an archive"
                ));
            }
            match path {
                Some(authored) => {
                    resolve_authored_source(relative_root, descriptor, &authored, false)
                }
                None if label == "DLC" => descriptor
                    .parent()
                    .map(Path::to_path_buf)
                    .ok_or_else(|| "DLC descriptor has no containing directory".into()),
                None => Err("recorded mod descriptor has no path or archive field".into()),
            }
        }
    }
}

fn resolve_authored_source(
    relative_root: &Path,
    descriptor: &Path,
    authored: &str,
    require_file: bool,
) -> Result<PathBuf, String> {
    let authored = authored.replace('\\', "/");
    let candidate = PathBuf::from(&authored);
    let resolved = if candidate.is_absolute() {
        candidate
    } else {
        // Launcher descriptors resolve portable paths from the install/profile
        // root. A descriptor-local path is accepted only when the root-relative
        // interpretation does not exist, matching launcher mod descriptors.
        let from_root = relative_root.join(authored.replace('/', std::path::MAIN_SEPARATOR_STR));
        if from_root.exists() {
            from_root
        } else {
            descriptor
                .parent()
                .unwrap_or(relative_root)
                .join(authored.replace('/', std::path::MAIN_SEPARATOR_STR))
        }
    };
    let metadata = fs::metadata(&resolved).map_err(|error| {
        format!(
            "recorded descriptor content source {} failed: {error}",
            resolved.display()
        )
    })?;
    if require_file != metadata.is_file() || (!require_file && !metadata.is_dir()) {
        return Err(format!(
            "recorded descriptor content source has wrong storage kind: {}",
            resolved.display()
        ));
    }
    Ok(resolved)
}

fn stable_relative_join(root: &Path, layer_id: &str) -> Result<PathBuf, String> {
    let normalized = normalize_stable_layer_id(layer_id)?;
    Ok(root.join(normalized.replace('/', std::path::MAIN_SEPARATOR_STR)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp() -> PathBuf {
        std::env::temp_dir().join(format!(
            "recorder_production_content_vfs_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn directory_mod_override_is_relocation_stable() {
        let mut results = Vec::new();
        for machine in ["first", "second"] {
            let root = temp().join(machine);
            let game = root.join("game");
            let user = root.join("profile");
            fs::create_dir_all(game.join("interface")).unwrap();
            fs::create_dir_all(user.join("mod/content/interface")).unwrap();
            fs::create_dir_all(user.join("mod")).unwrap();
            fs::write(game.join("interface/topbar.gui"), b"base").unwrap();
            fs::write(user.join("mod/content/interface/topbar.gui"), b"mod").unwrap();
            let descriptor_text = "name=\"Portable\"\npath=\"mod/content\"\n";
            fs::write(user.join("mod/portable.mod"), descriptor_text).unwrap();
            let layers = vec![
                RecordedContentLayer {
                    ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    display_name: "base".into(),
                    descriptor_sha256: None,
                    source_container_sha256: None,
                    storage_kind: LayerStorageKind::Directory,
                    replace_paths: Vec::new(),
                    required_for_playback: false,
                },
                RecordedContentLayer {
                    ordinal: 1,
                    layer_kind: ContentLayerKind::Mod,
                    layer_id: "mod/portable.mod".into(),
                    display_name: "Portable".into(),
                    descriptor_sha256: Some(semantic_descriptor_sha256(descriptor_text)),
                    source_container_sha256: None,
                    storage_kind: LayerStorageKind::Directory,
                    replace_paths: Vec::new(),
                    required_for_playback: false,
                },
            ];
            let vfs = effective_vfs_from_launch(
                &RuntimeContentRoots {
                    install_root: game,
                    user_dir: user,
                },
                &layers,
                &"a".repeat(64),
            )
            .unwrap();
            results.push(
                vfs.resolve_required("interface/topbar.gui", "topbar")
                    .unwrap(),
            );
            fs::remove_dir_all(root.parent().unwrap()).unwrap();
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[0].bytes, b"mod");
    }
}
