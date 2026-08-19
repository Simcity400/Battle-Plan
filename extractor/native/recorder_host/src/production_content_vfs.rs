use crate::modset::ModsetReport;
use crate::portable_content_manifest::{
    descriptor_replace_paths, descriptor_scalar, latin1_bytes_to_string, normalize_stable_layer_id,
    semantic_descriptor_sha256, PortableLaunchBinding, RuntimeContentRoots,
};
use portable_content_manifest::effective_vfs::{EffectiveVfs, EffectiveVfsLayerMount};
use portable_content_manifest::{ContentLayerKind, LayerStorageKind, RecordedContentLayer};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const A729_EXE_SHA256: &str = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
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

pub fn d49_base_only_authority_from_resolved_zero_mod_launch(
    report: &ModsetReport,
) -> Result<D49BaseOnlyStrategicRegionAuthority, String> {
    if !report.content_resolved || !report.enabled_mods.is_empty() {
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

    let base_vfs =
        effective_vfs_from_launch(&roots, std::slice::from_ref(&base), exact_exe_sha256)?;
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct D98ContentLayerAuthority {
    pub layer_kind: &'static str,
    pub layer_id: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct D98TheatreSelectorContentIdentityAuthority {
    pub schema: &'static str,
    pub source: &'static str,
    pub exact_exe_sha256: String,
    pub language: String,
    pub inventory_sha256: String,
    pub layers: Vec<D98ContentLayerAuthority>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundContentLayer {
    pub recorded: RecordedContentLayer,
    pub descriptor_path: Option<PathBuf>,
    pub descriptor_file_sha256: Option<String>,
}

struct D98OrderedLayer {
    recorded: RecordedContentLayer,
    authority_sha256: String,
}

pub fn d98_theatre_selector_content_identity_from_resolved_launch(
    report: &ModsetReport,
) -> Result<D98TheatreSelectorContentIdentityAuthority, String> {
    if !report.content_resolved {
        return Err("D98 content identity requires a resolved launch binding".into());
    }
    let exact_exe_sha256 = report
        .exe_sha256
        .as_deref()
        .ok_or("D98 content identity omitted executable identity")?;
    if exact_exe_sha256 != A729_EXE_SHA256 {
        return Err("D98 content identity requires the exact a729 executable".into());
    }
    let language = report
        .localisation_language()
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("D98 content identity omitted the launch-bound localisation language")?
        .to_string();
    if language.len() > 64
        || language
            .chars()
            .any(|value| value.is_control() || value == '/' || value == '\\')
    {
        return Err("D98 content identity language token is not a selectable token".into());
    }

    let roots = RuntimeContentRoots::from_modset(report)?;
    let launch = PortableLaunchBinding::from_modset(report)?;
    let ordered = d98_ordered_layers(&roots, &launch, report, exact_exe_sha256)?;

    let layers = ordered
        .iter()
        .map(|layer| D98ContentLayerAuthority {
            layer_kind: match layer.recorded.layer_kind {
                ContentLayerKind::Base => "base",
                ContentLayerKind::IntegratedDlc => "integrated_dlc",
                ContentLayerKind::Dlc => "dlc",
                ContentLayerKind::Mod => "mod",
            },
            layer_id: layer.recorded.layer_id.clone(),
            sha256: layer.authority_sha256.clone(),
        })
        .collect::<Vec<_>>();
    validate_d98_layer_inventory(&layers)?;
    let inventory_bytes = serde_json::to_vec(&layers)
        .map_err(|error| format!("serialize D98 layer inventory failed: {error}"))?;
    let inventory_sha256 = format!("{:x}", Sha256::digest(&inventory_bytes));

    Ok(D98TheatreSelectorContentIdentityAuthority {
        schema: "hoi4.d98-theatre-selector-content-identity.v1",
        source: "resolved_effective_vfs_ordered_content_identity",
        exact_exe_sha256: exact_exe_sha256.into(),
        language,
        inventory_sha256,
        layers,
    })
}

pub fn ordered_content_layers_from_resolved_launch(
    roots: &RuntimeContentRoots,
    launch: &PortableLaunchBinding,
    report: &ModsetReport,
) -> Result<Vec<BoundContentLayer>, String> {
    let mut ordered: Vec<BoundContentLayer> = Vec::new();
    ordered.push(BoundContentLayer {
        recorded: RecordedContentLayer {
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
        descriptor_path: None,
        descriptor_file_sha256: None,
    });

    let mut enabled_descriptor_evidence = BTreeMap::new();
    for entry in &report.enabled_dlcs {
        let Some(evidence) = report
            .dlc_descriptor_evidence
            .get(entry)
            .and_then(Option::as_ref)
        else {
            return Err(format!(
                "bound content layers: enabled DLC lacks launch-bound descriptor evidence: {entry:?}"
            ));
        };
        enabled_descriptor_evidence.insert(
            normalize_stable_layer_id(entry)?,
            evidence.descriptor_file_sha256.clone(),
        );
    }

    for (kind, bound) in launch
        .integrated_dlc_layers
        .values()
        .map(|bound| (ContentLayerKind::IntegratedDlc, bound))
        .chain(
            launch
                .enabled_dlc_layers
                .values()
                .map(|bound| (ContentLayerKind::Dlc, bound)),
        )
    {
        let descriptor = stable_relative_join(&roots.install_root, &bound.layer_id)?;
        let expected = enabled_descriptor_evidence
            .get(&bound.layer_id)
            .map(String::as_str);
        let (text, file_sha256) = stable_descriptor_text_and_file_sha256(&descriptor, expected)?;
        if semantic_descriptor_sha256(&text) != bound.descriptor_sha256 {
            return Err(format!(
                "bound content layers: DLC descriptor drifted while binding: {}",
                bound.layer_id
            ));
        }
        ordered.push(BoundContentLayer {
            recorded: RecordedContentLayer {
                ordinal: ordered.len() as u32,
                layer_kind: kind,
                layer_id: bound.layer_id.clone(),
                display_name: bound.layer_id.clone(),
                descriptor_sha256: Some(bound.descriptor_sha256.clone()),
                source_container_sha256: bound.source_container_sha256.clone(),
                storage_kind: bound.storage_kind,
                replace_paths: descriptor_replace_paths(&text),
                required_for_playback: false,
            },
            descriptor_path: Some(descriptor),
            descriptor_file_sha256: Some(file_sha256),
        });
    }

    for bound in &launch.ordered_mods {
        let descriptor = stable_relative_join(&roots.user_dir, &bound.layer_id)?;
        let (text, file_sha256) = stable_descriptor_text_and_file_sha256(&descriptor, None)?;
        if semantic_descriptor_sha256(&text) != bound.descriptor_sha256 {
            return Err(format!(
                "bound content layers: mod descriptor drifted while binding: {}",
                bound.layer_id
            ));
        }
        ordered.push(BoundContentLayer {
            recorded: RecordedContentLayer {
                ordinal: ordered.len() as u32,
                layer_kind: ContentLayerKind::Mod,
                layer_id: bound.layer_id.clone(),
                display_name: bound.layer_id.clone(),
                descriptor_sha256: Some(bound.descriptor_sha256.clone()),
                source_container_sha256: (bound.storage_kind == LayerStorageKind::ZipArchive)
                    .then(|| bound.content_sha256.clone()),
                storage_kind: bound.storage_kind,
                replace_paths: descriptor_replace_paths(&text),
                required_for_playback: false,
            },
            descriptor_path: Some(descriptor),
            descriptor_file_sha256: Some(file_sha256),
        });
    }
    Ok(ordered)
}

fn d98_ordered_layers(
    roots: &RuntimeContentRoots,
    launch: &PortableLaunchBinding,
    report: &ModsetReport,
    exact_exe_sha256: &str,
) -> Result<Vec<D98OrderedLayer>, String> {
    let mut mods = launch.ordered_mods.iter();
    ordered_content_layers_from_resolved_launch(roots, launch, report)?
        .into_iter()
        .map(|layer| {
            let authority_sha256 = match layer.recorded.layer_kind {
                ContentLayerKind::Base => exact_exe_sha256.to_string(),
                ContentLayerKind::Mod => {
                    let bound = mods.next().ok_or_else(|| {
                        format!(
                            "D98 content identity lost the bound mod {}",
                            layer.recorded.layer_id
                        )
                    })?;
                    if bound.layer_id != layer.recorded.layer_id {
                        return Err(format!(
                            "D98 content identity mod order diverged at {}",
                            layer.recorded.layer_id
                        ));
                    }
                    bound.content_sha256.clone()
                }
                ContentLayerKind::Dlc | ContentLayerKind::IntegratedDlc => {
                    layer.descriptor_file_sha256.clone().ok_or_else(|| {
                        format!(
                            "D98 content identity DLC layer {} carries no descriptor digest",
                            layer.recorded.layer_id
                        )
                    })?
                }
            };
            Ok(D98OrderedLayer {
                recorded: layer.recorded,
                authority_sha256,
            })
        })
        .collect()
}

fn stable_descriptor_text_and_file_sha256(
    descriptor: &Path,
    expected_file_sha256: Option<&str>,
) -> Result<(String, String), String> {
    let first = fs::read(descriptor).map_err(|error| {
        format!(
            "read D98 layer descriptor {} failed: {error}",
            descriptor.display()
        )
    })?;
    let second = fs::read(descriptor).map_err(|error| {
        format!(
            "re-read D98 layer descriptor {} failed: {error}",
            descriptor.display()
        )
    })?;
    if first != second {
        return Err(format!(
            "D98 layer descriptor changed while binding: {}",
            descriptor.display()
        ));
    }
    let file_sha256 = format!("{:x}", Sha256::digest(&first));
    if let Some(expected) = expected_file_sha256 {
        if file_sha256 != expected {
            return Err(format!(
                "D98 layer descriptor differs from the launch-bound bytes: {}",
                descriptor.display()
            ));
        }
    }
    Ok((latin1_bytes_to_string(&first), file_sha256))
}

fn validate_d98_layer_inventory(layers: &[D98ContentLayerAuthority]) -> Result<(), String> {
    if layers.is_empty() {
        return Err("D98 content identity has no content layers".into());
    }
    if layers.len() > 4_096 {
        return Err("D98 content identity layer count exceeds the transport bound".into());
    }
    let mut seen = BTreeSet::new();
    for layer in layers {
        if layer.layer_id.is_empty()
            || layer.layer_id.len() > 1_024
            || layer.layer_id.chars().any(char::is_control)
        {
            return Err(format!(
                "D98 content identity layer id is not transportable: {:?}",
                layer.layer_id
            ));
        }
        if layer.sha256.len() != 64
            || !layer
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(format!(
                "D98 content identity layer {} digest is not a lowercase sha256",
                layer.layer_id
            ));
        }
        if !seen.insert((layer.layer_kind, layer.layer_id.as_str())) {
            return Err(format!(
                "D98 content identity repeats content layer {} {}",
                layer.layer_kind, layer.layer_id
            ));
        }
    }
    Ok(())
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
        ContentLayerKind::Dlc => {
            descriptor_content_source(&roots.install_root, &descriptor, &text, storage_kind, "DLC")?
        }
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
        let local_source = layer_local_source(roots, layer)?;
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

pub fn layer_local_source(
    roots: &RuntimeContentRoots,
    layer: &RecordedContentLayer,
) -> Result<PathBuf, String> {
    Ok(match layer.layer_kind {
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
                return Err("integrated DLC must mount its physical descriptor directory".into());
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
    })
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
        static TEMP_SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "recorder_production_content_vfs_{}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            TEMP_SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    /// different claim entirely -- it made this test flaky in the release gate
    fn remove_scratch_tree(path: &Path) {
        for attempt in 0..10 {
            match fs::remove_dir_all(path) {
                Ok(()) => return,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(_) if attempt < 9 => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(_) => return,
            }
        }
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
            remove_scratch_tree(root.parent().unwrap());
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[0].bytes, b"mod");
    }

    use crate::modset::{DisplayVariantSettings, ResolvedMod};

    struct Fixture {
        root: PathBuf,
        install: PathBuf,
        user: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let root = temp().join(label);
            let install = root.join("game");
            let user = root.join("profile");
            fs::create_dir_all(&install).unwrap();
            fs::create_dir_all(&user).unwrap();
            fs::write(install.join("hoi4.exe"), b"not the real executable").unwrap();
            Self {
                root,
                install,
                user,
            }
        }

        fn write(&self, root: &Path, relative: &str, bytes: &str) {
            let path = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes.as_bytes()).unwrap();
        }

        fn base(&self, relative: &str, bytes: &str) {
            self.write(&self.install.clone(), relative, bytes);
        }

        fn add_mod(&self, id: &str, descriptor_extra: &str) -> ResolvedMod {
            let descriptor_entry = format!("mod/{id}.mod");
            let text = format!("name=\"{id}\"\npath=\"mod/{id}\"\n{descriptor_extra}");
            self.write(&self.user.clone(), &descriptor_entry, &text);
            let content = self.user.join("mod").join(id);
            fs::create_dir_all(&content).unwrap();
            ResolvedMod {
                descriptor_entry: descriptor_entry.clone(),
                descriptor_path: Some(self.user.join("mod").join(format!("{id}.mod"))),
                descriptor_sha256: Some(format!("{:x}", Sha256::digest(text.as_bytes()))),
                name: Some(id.into()),
                version: None,
                remote_file_id: None,
                content_path: Some(content.display().to_string()),
                content_fingerprint: None,
                content_sha256: Some(format!("{:02x}", id.len()).repeat(32)[..64].to_string()),
                content_max_mtime: None,
                descriptor_mtime: None,
                content_location_conflict: false,
            }
        }

        fn mod_file(&self, id: &str, relative: &str, bytes: &str) {
            let root = self.user.join("mod").join(id);
            self.write(&root, relative, bytes);
        }

        fn report(&self, language: Option<&str>) -> ModsetReport {
            self.report_started_at(
                language,
                Self::now_unix()
                    .checked_add(3_600)
                    .expect("fixture launch time"),
            )
        }

        fn now_unix() -> i64 {
            i64::try_from(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            )
            .unwrap()
        }

        fn report_started_at(&self, language: Option<&str>, started_unix: i64) -> ModsetReport {
            ModsetReport {
                content_resolved: true,
                errors: Vec::new(),
                settings: crate::modset::SettingsBinding {
                    authority: Some(crate::modset::SettingsAuthority::PreLaunchBracketed),
                    errors: Vec::new(),
                    language: language.map(Into::into),
                    display: Some(DisplayVariantSettings {
                        physical_width: 1920,
                        physical_height: 1080,
                        min_gui_width: 1920,
                        min_gui_height: 1080,
                        gui_scale_f32_bits: 1_065_353_216,
                        refresh_rate_hz: 60,
                        max_refresh_rate_hz: Some(60),
                        fullscreen: false,
                        borderless: false,
                        display_mode: "windowed".into(),
                        settings_file_sha256: "b".repeat(64),
                    }),
                    attach_sha256: Some("b".repeat(64)),
                    seal_sha256: None,
                    endpoint_semantics_equal: None,
                },
                exe_path: Some(self.install.join("hoi4.exe")),
                exe_size_bytes: None,
                exe_sha256: Some(A729_EXE_SHA256.into()),
                user_dir: Some(self.user.clone()),
                user_dir_source: Some("probed"),
                dlc_load_mtime_unix: None,
                dlc_load_sha256: None,
                process_start_unix: Some(started_unix),
                process_start_filetime: None,
                enabled_mods: Vec::new(),
                installed_dlcs: Vec::new(),
                disabled_dlcs: Vec::new(),
                enabled_dlcs: Vec::new(),
                dlc_display_names: BTreeMap::new(),
                dlc_descriptor_evidence: BTreeMap::new(),
            }
        }
    }

    fn base_only(label: &str) -> Fixture {
        let fixture = Fixture::new(label);
        fixture.base(
            "localisation/english/theatre_l_english.yml",
            "l_english:\n THEATRE_ONE:0 \"Home Theatre\"\n",
        );
        fixture.base(
            "localisation/french/theatre_l_french.yml",
            "l_french:\n THEATRE_ONE:0 \"Theatre National\"\n",
        );
        fixture.base(
            "localisation/english/notes_l_english.txt",
            "not localisation content",
        );
        fixture
    }

    fn identity(report: &ModsetReport) -> D98TheatreSelectorContentIdentityAuthority {
        d98_theatre_selector_content_identity_from_resolved_launch(report)
            .expect("domain 98 content identity")
    }

    thread_local! {
        static PROOFS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    #[test]
    fn a_repeated_enabled_mod_is_refused_before_it_reaches_the_collector() {
        let fixture = base_only("d98_duplicate_mod");
        let twice = fixture.add_mod("twice", "");
        fixture.mod_file(
            "twice",
            "localisation/english/twice_l_english.yml",
            "l_english:\n TWICE:0 \"Twice\"\n",
        );
        let mut report = fixture.report(Some("l_english"));
        report.enabled_mods = vec![twice.clone(), twice];
        let error = d98_theatre_selector_content_identity_from_resolved_launch(&report)
            .expect_err("a repeated content layer must be refused host-side");
        assert!(
            error.contains("repeats content layer"),
            "unexpected refusal: {error}"
        );
    }

    #[test]
    fn a_mixed_layer_stack_keeps_each_kinds_own_identity_and_the_authored_mod_order() {
        let fixture = base_only("d98_mixed_layers");
        let descriptor_text = "name=\"Integrated\"\n";
        fixture.write(
            &fixture.install.clone(),
            "integrated_dlc/pack/pack.dlc",
            descriptor_text,
        );
        fixture.base(
            "integrated_dlc/pack/localisation/english/pack_l_english.yml",
            "l_english:\n",
        );
        let descriptor_file_sha256 = format!("{:x}", Sha256::digest(descriptor_text.as_bytes()));

        let second = fixture.add_mod("second", "");
        let first = fixture.add_mod("first", "");
        fixture.mod_file(
            "first",
            "localisation/english/a_l_english.yml",
            "l_english:\n",
        );
        fixture.mod_file(
            "second",
            "localisation/english/b_l_english.yml",
            "l_english:\n",
        );

        let mut report = fixture.report(Some("l_english"));
        report.enabled_mods = vec![second.clone(), first.clone()];
        report.installed_dlcs = vec!["integrated_dlc/pack/pack.dlc".into()];

        let authority = identity(&report);
        let kinds = authority
            .layers
            .iter()
            .map(|layer| {
                (
                    layer.layer_kind,
                    layer.layer_id.as_str(),
                    layer.sha256.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                ("base", "base", A729_EXE_SHA256),
                (
                    "integrated_dlc",
                    "integrated_dlc/pack/pack.dlc",
                    descriptor_file_sha256.as_str()
                ),
                (
                    "mod",
                    "mod/second.mod",
                    second.content_sha256.as_deref().unwrap()
                ),
                (
                    "mod",
                    "mod/first.mod",
                    first.content_sha256.as_deref().unwrap()
                ),
            ],
            "base carries the executable, a DLC its raw descriptor digest, and a mod its content \
             digest, in dlc_load.json order"
        );
    }

    #[test]
    fn absent_localisation_language_fails_closed() {
        let fixture = base_only("d98_no_language");
        let error =
            d98_theatre_selector_content_identity_from_resolved_launch(&fixture.report(None))
                .expect_err("an absent language must fail, never default to English");
        assert_eq!(
            error, "D98 content identity omitted the launch-bound localisation language",
            "the refusal must come from D98's own language gate"
        );
    }

    #[test]
    fn a_blank_or_unselectable_language_token_fails_closed() {
        let fixture = base_only("d98_bad_language");
        let error = d98_theatre_selector_content_identity_from_resolved_launch(
            &fixture.report(Some("l_english/")),
        )
        .expect_err("a token that is not a selectable language must fail");
        assert_eq!(
            error,
            "D98 content identity language token is not a selectable token"
        );
    }

    #[test]
    fn ordered_layer_inventory_matches_the_collector_recomputation() {
        let fixture = base_only("d98_inventory_shape");
        let mut report = fixture.report(Some("l_english"));
        report.enabled_mods = vec![fixture.add_mod("shaped", "")];
        fixture.mod_file(
            "shaped",
            "localisation/english/shaped_l_english.yml",
            "l_english:\n SHAPED:0 \"Shaped\"\n",
        );
        let authority = identity(&report);
        let encoded = serde_json::to_string(&authority.layers).unwrap();
        assert_eq!(
            encoded,
            format!(
                "[{{\"layer_kind\":\"base\",\"layer_id\":\"base\",\"sha256\":\"{}\"}},\
                 {{\"layer_kind\":\"mod\",\"layer_id\":\"mod/shaped.mod\",\"sha256\":\"{}\"}}]",
                A729_EXE_SHA256, authority.layers[1].sha256
            )
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(encoded.as_bytes())),
            authority.inventory_sha256
        );
        assert_eq!(
            authority.schema,
            "hoi4.d98-theatre-selector-content-identity.v1"
        );
        assert_eq!(
            authority.source,
            "resolved_effective_vfs_ordered_content_identity"
        );
    }
}
