//! Recorder-owned physical mounting for the neutral portable effective VFS.
//!
//! Local launch roots are admitted only long enough to build exact mounts.
//! They never enter the portable replay or any neutral schema.

use crate::portable_content_manifest::{
    descriptor_scalar, latin1_bytes_to_string, normalize_stable_layer_id,
    semantic_descriptor_sha256, RuntimeContentRoots,
};
use portable_content_manifest::effective_vfs::{EffectiveVfs, EffectiveVfsLayerMount};
use portable_content_manifest::{ContentLayerKind, LayerStorageKind, RecordedContentLayer};
use std::fs;
use std::path::{Path, PathBuf};

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
