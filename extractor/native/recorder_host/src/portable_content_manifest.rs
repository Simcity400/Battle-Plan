use hud_role_observer_live_lane::{
    EffectiveVfsHudRoleResolver, HudContentObservation, HudRoleSnapshot, HudSurface,
    HudSurfaceActivationFrame, OmittedHudSurface,
};
use portable_content_manifest::{
    filesystem_case_key, normalize_game_relative_path, ContentLayerKind, ContentManifest,
    DlcSelection, LayerStorageKind, RecordedContentLayer, RequiredContent, RequiredContentKind,
    SourceGameIdentity, SourceProfileIdentity, UsedContentKey, AUTHORED_GFX_NAMESPACE_CONSUMER,
    AUTHORED_GUI_NAMESPACE_CONSUMER, AUTHORED_LOCALISATION_NAMESPACES_CONSUMER,
    SCHEMA as CONTENT_MANIFEST_SCHEMA,
};
use portable_render_stream::ProductionRenderFrame;
use serde::{Deserialize, Serialize};
use snapshot::sha256::{to_hex, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::early_render_launch::{EarlyRenderCaptureReceipt, EarlyRenderLaunchManifest};
use crate::modset::{ModsetError, ModsetReport};
use crate::production_content_vfs::effective_vfs_from_launch;
use crate::production_map_content_index::{
    resolve_authenticated_o05_building_content, resolve_authenticated_static_program_content,
    resolve_authenticated_ui_pass_content, StaticProgramContentObservation,
    RUNTIME_UI_CONSUMER_PREFIX, STATIC_PROGRAM_CONSUMER_PREFIX,
};

pub const TRACE_SCHEMA: &str = "extractor.portable-content-use-trace.v1";
pub const DEFAULT_TRACE_FILE: &str = "portable_content_use_trace.json";
pub const MAX_TRACE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_OBSERVATIONS: usize = 2_000_000;
const M02_CONTENT_CONSUMER: &str = "replay_app.production_ui.m02_combat_indicator";
const PRODUCTION_RENDER_COMPACT_DOMAIN_ID: u16 = 77;
const LEGACY_FIXED_ROLE_MAP_SNAPSHOT_DOMAIN_ID: u16 = 80;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecorderContentTrace {
    pub schema: String,
    pub source_game: SourceGameIdentity,
    pub source_profile: SourceProfileIdentity,
    pub effective_vfs_manifest_sha256: String,
    pub ordered_layers: Vec<RecordedContentLayer>,
    pub dlc_selection: Vec<DlcSelection>,
    pub localisation_language: String,
    pub required_settings: BTreeMap<String, String>,
    pub recorder_local_settings: BTreeMap<String, String>,
    pub observations: Vec<ContentUseObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentUseObservation {
    pub content_key: String,
    pub kind: RequiredContentKind,
    pub content: UsedContentKey,
    pub consumer: String,
}

impl RecorderContentTrace {
    pub fn record_observation(&mut self, observation: ContentUseObservation) -> Result<(), String> {
        if self.observations.len() >= MAX_OBSERVATIONS {
            return Err(format!(
                "portable content trace reached the {MAX_OBSERVATIONS} observation bound"
            ));
        }
        validate_observation_header(&observation, &self.effective_vfs_manifest_sha256)?;
        self.observations.push(observation);
        Ok(())
    }

    pub fn write_atomic(&self, path: &Path) -> Result<(), String> {
        if self.schema != TRACE_SCHEMA {
            return Err(format!(
                "unsupported portable content trace schema {}",
                self.schema
            ));
        }
        if self.observations.is_empty() {
            return Err("production portable content trace has no observed content".into());
        }
        if self.observations.len() > MAX_OBSERVATIONS {
            return Err(format!(
                "portable content trace has {} observations, above the {MAX_OBSERVATIONS} bound",
                self.observations.len()
            ));
        }
        for observation in &self.observations {
            validate_observation_header(observation, &self.effective_vfs_manifest_sha256)?;
        }
        let bytes = serde_json::to_vec(self)
            .map_err(|error| format!("serialize portable content trace failed: {error}"))?;
        if bytes.len() as u64 > MAX_TRACE_BYTES {
            return Err(format!(
                "portable content trace is {} bytes, above the {MAX_TRACE_BYTES} byte bound",
                bytes.len()
            ));
        }
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)
            .map_err(|error| format!("create portable content trace directory failed: {error}"))?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("portable content trace path has no UTF-8 file name")?;
        let mut attempt = 0u32;
        let (temporary_path, mut temporary) = loop {
            let candidate: PathBuf = parent.join(format!(
                ".{file_name}.{}.{}.tmp",
                std::process::id(),
                attempt
            ));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => break (candidate, file),
                Err(error)
                    if error.kind() == std::io::ErrorKind::AlreadyExists && attempt < 100 =>
                {
                    attempt += 1;
                }
                Err(error) => {
                    return Err(format!(
                        "create portable content trace temporary failed: {error}"
                    ))
                }
            }
        };
        let write_result = (|| -> Result<(), String> {
            temporary
                .write_all(&bytes)
                .map_err(|error| format!("write portable content trace failed: {error}"))?;
            temporary
                .sync_all()
                .map_err(|error| format!("sync portable content trace failed: {error}"))?;
            drop(temporary);
            if path.exists() {
                return Err(format!(
                    "portable content trace destination already exists: {}",
                    path.display()
                ));
            }
            fs::rename(&temporary_path, path)
                .map_err(|error| format!("publish portable content trace failed: {error}"))?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        write_result
    }
}

fn validate_observation_header(
    observation: &ContentUseObservation,
    effective_vfs_manifest_sha256: &str,
) -> Result<(), String> {
    if observation.content_key.trim().is_empty() || observation.consumer.trim().is_empty() {
        return Err("portable content observation has blank content_key/consumer".into());
    }
    observation.content.validate()?;
    if observation.content.effective_vfs_manifest_sha256 != effective_vfs_manifest_sha256 {
        return Err(format!(
            "content observation {} names a different effective VFS",
            observation.content.normalized_game_relative_path
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableLaunchBinding {
    pub executable_sha256: String,
    pub ordered_mods: Vec<BoundMod>,
    pub enabled_dlc_ids: BTreeSet<String>,
    pub enabled_dlc_layers: BTreeMap<String, BoundDlcLayer>,
    pub integrated_dlc_layers: BTreeMap<String, BoundDlcLayer>,
    pub localisation_language: String,
    pub required_settings: BTreeMap<String, String>,
    pub recorder_local_settings: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeContentRoots {
    pub install_root: PathBuf,
    pub user_dir: PathBuf,
}

impl RuntimeContentRoots {
    pub fn from_modset(report: &ModsetReport) -> Result<Self, String> {
        if !report.content_resolved {
            return Err("runtime content roots require a resolved modset".into());
        }
        let install_root = report
            .exe_path
            .as_ref()
            .and_then(|path| path.parent())
            .ok_or("resolved modset omitted the game install root")?
            .to_path_buf();
        let user_dir = report
            .user_dir
            .clone()
            .ok_or("resolved modset omitted the corroborated Paradox user directory")?;
        Ok(Self {
            install_root,
            user_dir,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundMod {
    pub layer_id: String,
    pub descriptor_sha256: String,
    pub content_sha256: String,
    pub storage_kind: LayerStorageKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundDlcLayer {
    pub layer_id: String,
    pub descriptor_sha256: String,
    pub source_container_sha256: Option<String>,
    pub storage_kind: LayerStorageKind,
}

impl PortableLaunchBinding {
    pub fn from_modset(report: &ModsetReport) -> Result<Self, String> {
        if !report.content_resolved {
            return Err(format!(
                "portable content manifest requires a resolved launch binding: {}",
                report
                    .errors
                    .iter()
                    .map(ModsetError::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        let executable_sha256 = report
            .exe_sha256
            .clone()
            .ok_or("resolved launch binding omitted executable SHA-256")?;
        let settings_authority = report
            .settings
            .authority
            .ok_or("launch binding omitted the settings authority")?;
        let localisation_language = report
            .localisation_language()
            .cloned()
            .filter(|value| !value.trim().is_empty())
            .ok_or("resolved launch binding omitted localisation language")?;
        let display = report
            .display_variant()
            .ok_or("resolved launch binding omitted display settings")?;
        let required_settings = BTreeMap::new();
        let recorder_local_settings = BTreeMap::from([
            ("physical_width".into(), display.physical_width.to_string()),
            (
                "physical_height".into(),
                display.physical_height.to_string(),
            ),
            ("min_gui_width".into(), display.min_gui_width.to_string()),
            ("min_gui_height".into(), display.min_gui_height.to_string()),
            (
                "gui_scale_f32_bits".into(),
                display.gui_scale_f32_bits.to_string(),
            ),
            (
                "refresh_rate_hz".into(),
                display.refresh_rate_hz.to_string(),
            ),
            (
                "max_refresh_rate_hz".into(),
                display
                    .max_refresh_rate_hz
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "unavailable".into()),
            ),
            ("fullscreen".into(), display.fullscreen.to_string()),
            ("borderless".into(), display.borderless.to_string()),
            ("display_mode".into(), display.display_mode.clone()),
            (
                "settings_file_sha256".into(),
                display.settings_file_sha256.clone(),
            ),
            (
                "settings_authority".into(),
                settings_authority.as_str().to_string(),
            ),
        ]);
        let ordered_mods = report
            .enabled_mods
            .iter()
            .enumerate()
            .map(|(ordinal, source)| {
                Ok(BoundMod {
                    layer_id: normalize_stable_layer_id(&source.descriptor_entry)
                        .map_err(|error| format!("enabled mod {ordinal} descriptor id: {error}"))?,
                    descriptor_sha256: semantic_mod_descriptor_sha256(source).map_err(|error| {
                        format!("enabled mod {ordinal} semantic descriptor: {error}")
                    })?,
                    content_sha256: source
                        .content_sha256
                        .clone()
                        .ok_or_else(|| format!("enabled mod {ordinal} has no content hash"))?,
                    storage_kind: source
                        .content_path
                        .as_ref()
                        .and_then(|path| fs::metadata(path).ok())
                        .map(|metadata| {
                            if metadata.is_file() {
                                LayerStorageKind::ZipArchive
                            } else {
                                LayerStorageKind::Directory
                            }
                        })
                        .unwrap_or(LayerStorageKind::Directory),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let install_root = report
            .exe_path
            .as_ref()
            .and_then(|path| path.parent())
            .ok_or("resolved launch binding omitted executable install root")?;
        let enabled_dlc_layers = report
            .enabled_dlcs
            .iter()
            .map(|entry| -> Result<(String, String, String), String> {
                let evidence = report
                    .dlc_descriptor_evidence
                    .get(entry)
                    .and_then(Option::as_ref)
                    .ok_or_else(|| {
                        format!("enabled DLC lacks launch-bound descriptor evidence: {entry:?}")
                    })?;
                Ok((
                    stable_dlc_id(entry)?,
                    normalize_stable_layer_id(entry)?,
                    evidence.descriptor_file_sha256.clone(),
                ))
            })
            .map(|result| {
                let (dlc_id, layer_id, descriptor_file_sha256) = result?;
                let bound = bound_dlc_layer_with_expected_file_sha256(
                    install_root,
                    &layer_id,
                    Some(&descriptor_file_sha256),
                )?;
                Ok((dlc_id, bound))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let enabled_dlc_ids = enabled_dlc_layers.keys().cloned().collect();
        let integrated_dlc_layers = discover_integrated_dlc_layers(install_root)?;
        Ok(Self {
            executable_sha256,
            ordered_mods,
            enabled_dlc_ids,
            enabled_dlc_layers,
            integrated_dlc_layers,
            localisation_language,
            required_settings,
            recorder_local_settings,
        })
    }

    pub fn from_effective_runtime_candidate(report: &ModsetReport) -> Result<Self, String> {
        let mut candidate = Self::from_modset(report)?;
        let install_root = report
            .exe_path
            .as_ref()
            .and_then(|path| path.parent())
            .ok_or("resolved launch binding omitted executable install root")?;
        for descriptor_id in &report.installed_dlcs {
            let dlc_id = stable_dlc_id(descriptor_id)?;
            if candidate.enabled_dlc_layers.contains_key(&dlc_id) {
                continue;
            }
            let evidence = report
                .dlc_descriptor_evidence
                .get(descriptor_id)
                .and_then(Option::as_ref)
                .ok_or_else(|| {
                    format!(
                        "installed DLC lacks launch-bound descriptor evidence: {descriptor_id:?}"
                    )
                })?;
            let layer_id = normalize_stable_layer_id(descriptor_id)?;
            let bound = bound_dlc_layer_with_expected_file_sha256(
                install_root,
                &layer_id,
                Some(&evidence.descriptor_file_sha256),
            )?;
            if candidate
                .enabled_dlc_layers
                .insert(dlc_id.clone(), bound)
                .is_some()
            {
                return Err(format!(
                    "installed DLC roster repeats effective candidate ID {dlc_id:?}"
                ));
            }
        }
        candidate.enabled_dlc_ids = candidate.enabled_dlc_layers.keys().cloned().collect();
        if candidate.enabled_dlc_ids.len() != report.installed_dlcs.len() {
            return Err("effective-runtime DLC candidate collapsed an installed descriptor".into());
        }
        Ok(candidate)
    }
}

fn bound_dlc_layer(install_root: &Path, layer_id: &str) -> Result<BoundDlcLayer, String> {
    bound_dlc_layer_with_expected_file_sha256(install_root, layer_id, None)
}

fn bound_dlc_layer_with_expected_file_sha256(
    install_root: &Path,
    layer_id: &str,
    expected_file_sha256: Option<&str>,
) -> Result<BoundDlcLayer, String> {
    let layer_id = normalize_stable_layer_id(layer_id)?;
    let descriptor_path = install_root.join(layer_id.replace('/', std::path::MAIN_SEPARATOR_STR));
    let first = fs::read(&descriptor_path)
        .map_err(|error| format!("read DLC descriptor {layer_id:?} failed: {error}"))?;
    let second = fs::read(&descriptor_path)
        .map_err(|error| format!("re-read DLC descriptor {layer_id:?} failed: {error}"))?;
    if first != second {
        return Err(format!("DLC descriptor {layer_id:?} changed while binding"));
    }
    if let Some(expected) = expected_file_sha256 {
        let observed = Sha256::hex(&first);
        if observed != expected {
            return Err(format!(
                "DLC descriptor {layer_id:?} differs from the launch-bound bytes"
            ));
        }
    }
    let text = latin1_bytes_to_string(&first);
    let archive = descriptor_scalar(&text, "archive");
    let path = descriptor_scalar(&text, "path");
    if archive.is_some() && path.is_some() {
        return Err(format!(
            "DLC descriptor {layer_id:?} has both path and archive"
        ));
    }
    let source_container_sha256 = match archive {
        Some(relative) => {
            let normalized = normalize_stable_layer_id(&relative)?;
            let archive_path =
                install_root.join(normalized.replace('/', std::path::MAIN_SEPARATOR_STR));
            let first = fs::read(&archive_path)
                .map_err(|error| format!("read DLC archive {relative:?} failed: {error}"))?;
            let second = fs::read(&archive_path)
                .map_err(|error| format!("re-read DLC archive {relative:?} failed: {error}"))?;
            if first != second {
                return Err(format!("DLC archive {relative:?} changed while binding"));
            }
            Some(Sha256::hex(&first))
        }
        None => None,
    };
    Ok(BoundDlcLayer {
        layer_id,
        descriptor_sha256: semantic_descriptor_sha256(&text),
        storage_kind: if source_container_sha256.is_some() {
            LayerStorageKind::ZipArchive
        } else {
            LayerStorageKind::Directory
        },
        source_container_sha256,
    })
}

fn discover_integrated_dlc_layers(
    install_root: &Path,
) -> Result<BTreeMap<String, BoundDlcLayer>, String> {
    let root = install_root.join("integrated_dlc");
    if !root.is_dir() {
        return Ok(BTreeMap::new());
    }
    let mut descriptors = Vec::new();
    for directory in
        fs::read_dir(&root).map_err(|error| format!("enumerate integrated DLC failed: {error}"))?
    {
        let directory = directory.map_err(|error| format!("read integrated DLC entry: {error}"))?;
        if !directory.path().is_dir() {
            continue;
        }
        for entry in fs::read_dir(directory.path())
            .map_err(|error| format!("enumerate integrated DLC descriptors failed: {error}"))?
        {
            let entry =
                entry.map_err(|error| format!("read integrated DLC descriptor: {error}"))?;
            if entry.path().extension().and_then(|value| value.to_str()) == Some("dlc") {
                let relative = entry
                    .path()
                    .strip_prefix(install_root)
                    .map_err(|_| "integrated DLC descriptor escaped install root")?
                    .to_string_lossy()
                    .replace('\\', "/");
                descriptors.push(relative);
            }
        }
    }
    descriptors.sort();
    let mut out = BTreeMap::new();
    for layer_id in descriptors {
        let dlc_id = stable_dlc_id(&layer_id.replace("integrated_dlc/", "dlc/"))?;
        if out
            .insert(
                dlc_id.clone(),
                bound_integrated_dlc_layer(install_root, &layer_id)?,
            )
            .is_some()
        {
            return Err(format!("integrated DLC repeats product id {dlc_id:?}"));
        }
    }
    Ok(out)
}

fn bound_integrated_dlc_layer(
    install_root: &Path,
    layer_id: &str,
) -> Result<BoundDlcLayer, String> {
    let layer_id = normalize_stable_layer_id(layer_id)?;
    let descriptor_path = install_root.join(layer_id.replace('/', std::path::MAIN_SEPARATOR_STR));
    let first = fs::read(&descriptor_path)
        .map_err(|error| format!("read integrated DLC descriptor {layer_id:?} failed: {error}"))?;
    let second = fs::read(&descriptor_path).map_err(|error| {
        format!("re-read integrated DLC descriptor {layer_id:?} failed: {error}")
    })?;
    if first != second {
        return Err(format!(
            "integrated DLC descriptor {layer_id:?} changed while binding"
        ));
    }
    let text = latin1_bytes_to_string(&first);
    Ok(BoundDlcLayer {
        layer_id,
        descriptor_sha256: semantic_descriptor_sha256(&text),
        storage_kind: LayerStorageKind::Directory,
        source_container_sha256: None,
    })
}

pub(crate) fn descriptor_scalar(text: &str, wanted: &str) -> Option<String> {
    text.lines().find_map(|raw_line| {
        let line = raw_line
            .split_once('#')
            .map(|(head, _)| head)
            .unwrap_or(raw_line)
            .trim();
        let (key, value) = line.split_once('=')?;
        (key.trim().eq_ignore_ascii_case(wanted))
            .then(|| parse_descriptor_scalar(value.trim()))
            .flatten()
    })
}

fn semantic_mod_descriptor_sha256(source: &crate::modset::ResolvedMod) -> Result<String, String> {
    let path = source
        .descriptor_path
        .as_ref()
        .ok_or("descriptor path is absent")?;
    let first = fs::read(path).map_err(|error| format!("read descriptor failed: {error}"))?;
    let second = fs::read(path).map_err(|error| format!("re-read descriptor failed: {error}"))?;
    if first != second {
        return Err("descriptor changed while computing portable identity".into());
    }
    let raw_expected = source
        .descriptor_sha256
        .as_deref()
        .ok_or("raw descriptor hash is absent")?;
    if Sha256::hex(&first) != raw_expected {
        return Err("descriptor bytes differ from launch binding".into());
    }
    let text = latin1_bytes_to_string(&first);
    Ok(semantic_descriptor_sha256(&text))
}

pub(crate) fn latin1_bytes_to_string(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

pub fn semantic_descriptor_sha256(text: &str) -> String {
    let mut name = None;
    let mut version = None;
    let mut supported_version = None;
    let mut remote_file_id = None;
    let mut replace_path = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line
            .split_once('#')
            .map(|(head, _)| head)
            .unwrap_or(raw_line)
            .trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(value) = parse_descriptor_scalar(value.trim()) else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "name" => name = Some(value),
            "version" => version = Some(value),
            "supported_version" => supported_version = Some(value),
            "remote_file_id" => remote_file_id = Some(value),
            "replace_path" => {
                let normalized = normalize_descriptor_rel_path(&value).to_ascii_lowercase();
                if !normalized.is_empty() {
                    replace_path.push(normalized);
                }
            }
            _ => {}
        }
    }
    let dependencies = parse_descriptor_quoted_list(text, "dependencies");
    let canonical = serde_json::json!({
        "schema": "hoi4.semantic-descriptor.v1",
        "name": name.as_deref().map(str::trim),
        "version": version.as_deref().map(str::trim),
        "supported_version": supported_version.as_deref().map(str::trim),
        "remote_file_id": remote_file_id.as_deref().map(str::trim),
        "replace_path": replace_path,
        "dependencies": dependencies,
    });
    Sha256::hex(&serde_json::to_vec(&canonical).expect("semantic descriptor JSON"))
}

fn parse_descriptor_scalar(value: &str) -> Option<String> {
    if value.starts_with('{') {
        return None;
    }
    if let Some(rest) = value.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(rest[..end].to_string());
    }
    value
        .split_whitespace()
        .next()
        .map(|value| value.trim_matches('"').to_string())
        .filter(|value| !value.is_empty())
}

fn parse_descriptor_quoted_list(text: &str, key: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let Some(key_start) = lower.find(&key.to_ascii_lowercase()) else {
        return Vec::new();
    };
    let rest = &text[key_start + key.len()..];
    let Some(open) = rest.find('{') else {
        return Vec::new();
    };
    let body = &rest[open + 1..];
    let Some(close) = body.find('}') else {
        return Vec::new();
    };
    let mut values = Vec::new();
    let mut remaining = &body[..close];
    while let Some(start) = remaining.find('"') {
        remaining = &remaining[start + 1..];
        let Some(end) = remaining.find('"') else {
            break;
        };
        let value = remaining[..end].trim();
        if !value.is_empty() {
            values.push(value.to_string());
        }
        remaining = &remaining[end + 1..];
    }
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn normalize_descriptor_rel_path(value: &str) -> String {
    let mut value = value
        .replace('\\', "/")
        .trim()
        .trim_matches('"')
        .to_string();
    while value.contains("//") {
        value = value.replace("//", "/");
    }
    value
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/")
}

pub(crate) fn normalize_stable_layer_id(value: &str) -> Result<String, String> {
    let normalized = value.trim().replace('\\', "/").to_ascii_lowercase();
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains(':')
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("unsafe/non-portable layer id {value:?}"));
    }
    Ok(normalized)
}

fn stable_dlc_id(entry: &str) -> Result<String, String> {
    let normalized = normalize_stable_layer_id(entry)?;
    let parts = normalized.split('/').collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != "dlc" || !parts[2].ends_with(".dlc") {
        return Err(format!(
            "DLC selection is not dlc/<id>/<descriptor>.dlc: {entry:?}"
        ));
    }
    Ok(parts[1].to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AggregatedUse {
    content_key: String,
    kind: RequiredContentKind,
    content: UsedContentKey,
    consumers: BTreeSet<String>,
    observation_count: u64,
}

pub fn load_and_build(
    path: &Path,
    launch: &PortableLaunchBinding,
    product_modset_sha256: &str,
) -> Result<(ContentManifest, Vec<u8>), String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("portable content trace metadata failed: {error}"))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_TRACE_BYTES {
        return Err(format!(
            "portable content trace must be a nonempty regular file no larger than {MAX_TRACE_BYTES} bytes"
        ));
    }
    let bytes =
        fs::read(path).map_err(|error| format!("read portable content trace failed: {error}"))?;
    let second = fs::read(path)
        .map_err(|error| format!("re-read portable content trace failed: {error}"))?;
    if bytes != second {
        return Err("portable content trace changed while finalizing the recording".into());
    }
    let trace: RecorderContentTrace = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse portable content trace failed: {error}"))?;
    let manifest = build(trace, launch, product_modset_sha256)?;
    let encoded = manifest.encode_json()?;
    Ok((manifest, encoded))
}

pub fn load_and_build_bound(
    trace_path: &Path,
    live_resolution_report_path: &Path,
    launch: &PortableLaunchBinding,
    product_modset_sha256: &str,
) -> Result<(ContentManifest, Vec<u8>), String> {
    let (manifest, bytes) = load_and_build(trace_path, launch, product_modset_sha256)?;
    verify_live_resolution_report(live_resolution_report_path, &manifest, launch)?;
    Ok((manifest, bytes))
}

/// generated from the extractor's effective VFS and records only source files
pub fn build_from_finalized_static_bundle(
    source_inventory_path: &Path,
    catalog_source_join_path: &Path,
    live_resolution_report_path: &Path,
    launch: &PortableLaunchBinding,
    product_modset_sha256: &str,
) -> Result<ContentManifest, String> {
    build_from_finalized_static_bundle_inner(
        source_inventory_path,
        catalog_source_join_path,
        live_resolution_report_path,
        launch,
        product_modset_sha256,
        None,
        &[],
        &[],
        &[],
    )
}

pub fn finalized_directory_layer_content_sha256(
    source_inventory_path: &Path,
    manifest: &ContentManifest,
) -> Result<BTreeMap<String, String>, String> {
    manifest.validate()?;
    let inventory = read_stable_json(source_inventory_path, "live source inventory")?;
    if required_json_sha(
        inventory.get("inventory_sha256"),
        "live source inventory digest",
    )? != manifest.effective_vfs_manifest_sha256
    {
        return Err("source inventory differs from finalized content manifest".into());
    }
    let namespaces = inventory
        .get("namespaces")
        .and_then(serde_json::Value::as_array)
        .ok_or("live source inventory namespaces is not an array")?;
    let mut playback_index = 0usize;
    let mut output = BTreeMap::new();
    for (raw_ordinal, value) in namespaces.iter().enumerate() {
        let row = value
            .as_object()
            .ok_or_else(|| format!("source namespace {raw_ordinal} is not an object"))?;
        if row.get("binding").and_then(serde_json::Value::as_str) == Some("asset_only") {
            continue;
        }
        let layer = manifest
            .ordered_layers
            .get(playback_index)
            .ok_or("source inventory contains more playback layers than manifest")?;
        if layer.ordinal as usize != playback_index {
            return Err("finalized content layer ordinals are not contiguous".into());
        }
        if layer.storage_kind == LayerStorageKind::Directory {
            output.insert(
                layer.layer_id.clone(),
                required_json_sha(
                    row.get("content_sha256"),
                    "source namespace directory tree digest",
                )?,
            );
        }
        playback_index += 1;
    }
    if playback_index != manifest.ordered_layers.len() {
        return Err("source inventory contains fewer playback layers than manifest".into());
    }
    Ok(output)
}

pub fn build_from_finalized_static_bundle_with_hud_snapshot(
    source_inventory_path: &Path,
    catalog_source_join_path: &Path,
    live_resolution_report_path: &Path,
    launch: &PortableLaunchBinding,
    product_modset_sha256: &str,
    hud_snapshot: &HudRoleSnapshot,
) -> Result<ContentManifest, String> {
    build_from_finalized_static_bundle_inner(
        source_inventory_path,
        catalog_source_join_path,
        live_resolution_report_path,
        launch,
        product_modset_sha256,
        Some(hud_snapshot),
        &[],
        &[],
        &[],
    )
}

pub fn build_from_finalized_static_bundle_with_runtime_stream(
    source_inventory_path: &Path,
    catalog_source_join_path: &Path,
    live_resolution_report_path: &Path,
    launch: &PortableLaunchBinding,
    product_modset_sha256: &str,
    runtime_roots: &RuntimeContentRoots,
    compact_stream_path: &Path,
    identity_mode: compact_contract::stream_validator::StreamIdentityMode,
) -> Result<ContentManifest, String> {
    build_from_finalized_static_bundle_with_runtime_stream_admission(
        source_inventory_path,
        catalog_source_join_path,
        live_resolution_report_path,
        launch,
        product_modset_sha256,
        runtime_roots,
        compact_stream_path,
        identity_mode,
        None,
    )
}

pub fn build_from_finalized_static_bundle_with_early_render_receipt(
    source_inventory_path: &Path,
    catalog_source_join_path: &Path,
    live_resolution_report_path: &Path,
    launch: &PortableLaunchBinding,
    product_modset_sha256: &str,
    runtime_roots: &RuntimeContentRoots,
    compact_stream_path: &Path,
    identity_mode: compact_contract::stream_validator::StreamIdentityMode,
    early_launch_manifest: &EarlyRenderLaunchManifest,
    early_capture_receipt: &EarlyRenderCaptureReceipt,
) -> Result<ContentManifest, String> {
    build_from_finalized_static_bundle_with_runtime_stream_admission(
        source_inventory_path,
        catalog_source_join_path,
        live_resolution_report_path,
        launch,
        product_modset_sha256,
        runtime_roots,
        compact_stream_path,
        identity_mode,
        Some((early_launch_manifest, early_capture_receipt)),
    )
}

fn build_from_finalized_static_bundle_with_runtime_stream_admission(
    source_inventory_path: &Path,
    catalog_source_join_path: &Path,
    live_resolution_report_path: &Path,
    launch: &PortableLaunchBinding,
    product_modset_sha256: &str,
    runtime_roots: &RuntimeContentRoots,
    compact_stream_path: &Path,
    identity_mode: compact_contract::stream_validator::StreamIdentityMode,
    early_admission: Option<(&EarlyRenderLaunchManifest, &EarlyRenderCaptureReceipt)>,
) -> Result<ContentManifest, String> {
    let base = build_from_finalized_static_bundle_inner(
        source_inventory_path,
        catalog_source_join_path,
        live_resolution_report_path,
        launch,
        product_modset_sha256,
        None,
        &[],
        &[],
        &[],
    )?;
    let runtime =
        scan_runtime_content_snapshots(compact_stream_path, identity_mode, early_admission)?;
    let selected_vfs = (!runtime.hud_activations.is_empty()
        || !runtime.static_program_frames.is_empty())
    .then(|| {
        effective_vfs_from_launch(
            runtime_roots,
            &base.ordered_layers,
            &base.effective_vfs_manifest_sha256,
        )
    })
    .transpose()?;
    let hud_snapshot = if runtime.hud_activations.is_empty() {
        None
    } else {
        let vfs = selected_vfs
            .as_ref()
            .ok_or("HUD activation requires the selected launch VFS")?;
        let mut winning_content = base
            .required_content
            .iter()
            .map(|required| required.content.clone())
            .map(|content| (content.normalized_game_relative_path.clone(), content))
            .collect::<BTreeMap<_, _>>();
        let mut closures = BTreeMap::new();
        for frame in &runtime.hud_activations {
            let index = portable_content_manifest::hud_emitter::emit_effective_vfs_hud_role_index(
                vfs,
                frame,
                &base.effective_vfs_manifest_sha256,
                &base.localisation_language,
            )?;
            for content in index.winning_content {
                match winning_content.get(&content.normalized_game_relative_path) {
                    Some(existing) if existing != &content => {
                        return Err(format!(
                            "HUD effective-VFS winner disagrees with static winner for {}",
                            content.normalized_game_relative_path
                        ));
                    }
                    _ => {
                        winning_content
                            .insert(content.normalized_game_relative_path.clone(), content);
                    }
                }
            }
            for closure in index.closures {
                let key = (closure.surface, closure.runtime_root_name.clone());
                match closures.get(&key) {
                    Some(existing) if existing != &closure => {
                        return Err(format!(
                            "HUD effective-VFS closure changed within recording for {}/{}",
                            closure.surface.token(),
                            closure.runtime_root_name
                        ));
                    }
                    _ => {
                        closures.insert(key, closure);
                    }
                }
            }
        }
        let resolver = EffectiveVfsHudRoleResolver::new(
            base.effective_vfs_manifest_sha256.clone(),
            winning_content.into_values(),
            closures.into_values(),
        )?;
        let snapshots = runtime
            .hud_activations
            .iter()
            .map(|frame| hud_role_observer_live_lane::promote_activation_frame(frame, &resolver))
            .collect::<Result<Vec<_>, _>>()?;
        Some(merge_hud_role_snapshots(snapshots)?)
    };
    let mut static_program_content = if runtime.static_program_frames.is_empty() {
        Vec::new()
    } else {
        resolve_authenticated_static_program_content(
            selected_vfs
                .as_ref()
                .ok_or("static-map programs require the selected launch VFS")?,
            runtime.static_program_frames.iter(),
            &base,
        )?
    };
    if !runtime.static_program_frames.is_empty() {
        static_program_content.extend(resolve_authenticated_o05_building_content(
            selected_vfs
                .as_ref()
                .ok_or("O05 buildings require the selected launch VFS")?,
            runtime.static_program_frames.iter(),
            &base,
        )?);
    }
    let runtime_ui_content = if runtime.static_program_frames.is_empty() {
        Vec::new()
    } else {
        resolve_authenticated_ui_pass_content(
            selected_vfs
                .as_ref()
                .ok_or("selected-Present UI commands require the selected launch VFS")?,
            runtime.static_program_frames.iter(),
            &base,
        )?
    };
    build_from_finalized_static_bundle_inner(
        source_inventory_path,
        catalog_source_join_path,
        live_resolution_report_path,
        launch,
        product_modset_sha256,
        hud_snapshot.as_ref(),
        &static_program_content,
        &runtime_ui_content,
        &runtime.m02_used_content,
    )
}

#[derive(Debug)]
struct RuntimeContentSnapshots {
    hud_activations: Vec<HudSurfaceActivationFrame>,
    static_program_frames: Vec<ProductionRenderFrame>,
    m02_used_content: Vec<UsedContentKey>,
}

pub(crate) fn descriptor_replace_paths(text: &str) -> Vec<String> {
    let mut replace_paths = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line
            .split_once('#')
            .map(|(head, _)| head)
            .unwrap_or(raw_line)
            .trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("replace_path") {
            continue;
        }
        let Some(value) = parse_descriptor_scalar(value.trim()) else {
            continue;
        };
        let normalized = normalize_descriptor_rel_path(&value).to_ascii_lowercase();
        if !normalized.is_empty() {
            replace_paths.push(normalized);
        }
    }
    replace_paths
}

fn merge_hud_role_snapshots(snapshots: Vec<HudRoleSnapshot>) -> Result<HudRoleSnapshot, String> {
    let mut snapshots = snapshots.into_iter();
    let first = snapshots
        .next()
        .ok_or("HUD activation history produced no role snapshots")?;
    let schema = first.schema.clone();
    let authority_manifest_sha256 = first.authority_manifest_sha256.clone();
    let mut activated_surfaces = BTreeSet::new();
    let mut observations = BTreeMap::<(String, String), HudContentObservation>::new();
    let mut omitted_surfaces = BTreeMap::<HudSurface, OmittedHudSurface>::new();

    let mut merge = |snapshot: HudRoleSnapshot| -> Result<(), String> {
        if snapshot.schema != schema
            || snapshot.authority_manifest_sha256 != authority_manifest_sha256
        {
            return Err("HUD role snapshot history changed schema or VFS authority".into());
        }
        activated_surfaces.extend(snapshot.activated_surfaces);
        for observation in snapshot.observations {
            let key = (
                observation.content_key.clone(),
                observation.consumer.clone(),
            );
            match observations.get(&key) {
                Some(existing) if existing != &observation => {
                    return Err(format!(
                        "HUD role observation changed within recording for {}/{}",
                        observation.content_key, observation.consumer
                    ));
                }
                _ => {
                    observations.insert(key, observation);
                }
            }
        }
        for omitted in snapshot.omitted_surfaces {
            match omitted_surfaces.get(&omitted.surface) {
                Some(existing) if existing != &omitted => {
                    return Err(format!(
                        "HUD omitted-surface disposition changed within recording for {}",
                        omitted.surface.token()
                    ));
                }
                _ => {
                    omitted_surfaces.insert(omitted.surface, omitted);
                }
            }
        }
        Ok(())
    };
    merge(first)?;
    for snapshot in snapshots {
        merge(snapshot)?;
    }
    let merged = HudRoleSnapshot {
        schema,
        authority_manifest_sha256,
        activated_surfaces: activated_surfaces.into_iter().collect(),
        observations: observations.into_values().collect(),
        omitted_surfaces: omitted_surfaces.into_values().collect(),
    };
    merged.validate()?;
    Ok(merged)
}

pub(crate) fn validate_render_frame_early_admission(
    frame: &ProductionRenderFrame,
    manifest: &EarlyRenderLaunchManifest,
    receipt: &EarlyRenderCaptureReceipt,
) -> Result<(), String> {
    manifest.validate()?;
    receipt.validate()?;
    let manifest_sha256 = manifest.sha256()?;
    let receipt_sha256 = receipt.sha256()?;
    if receipt.launch_manifest_sha256 != manifest_sha256
        || receipt.effective_vfs_manifest_sha256 != manifest.effective_vfs_manifest_sha256
        || receipt.executable_sha256 != manifest.executable_sha256
        || receipt.collector_sha256 != manifest.collector_sha256
    {
        return Err("early capture receipt differs from the selected launch manifest".into());
    }
    frame
        .validate()
        .map_err(|error| format!("production-render frame authority is invalid: {error}"))?;
    let authority = &frame.authority;
    if authority.process_id != receipt.process_id
        || authority.process_start_nonce != receipt.process_start_nonce
        || authority.hook_epoch_nonce != receipt.hook_epoch_nonce
        || authority.hook_set_receipt_sha256 != receipt.hook_set_sha256
        || authority.capture_receipt_sha256 != receipt_sha256
        || authority.effective_vfs_manifest_sha256 != receipt.effective_vfs_manifest_sha256
        || authority.observer_installed_qpc != receipt.hook_ready_qpc
        || authority.d3d_device_created_qpc != receipt.first_game_d3d_creation_qpc
        || authority.first_creation_sequence != receipt.first_creation_sequence
    {
        return Err(
            "production-render frame is foreign to the authenticated early-capture receipt".into(),
        );
    }
    Ok(())
}

fn scan_runtime_content_snapshots(
    stream_path: &Path,
    identity_mode: compact_contract::stream_validator::StreamIdentityMode,
    early_admission: Option<(&EarlyRenderLaunchManifest, &EarlyRenderCaptureReceipt)>,
) -> Result<RuntimeContentSnapshots, String> {
    use compact_contract::stream_validator::{AcceptedRecordKind, StatefulStreamValidator};

    const MAX_RECORD_LEN: usize = 256 * 1024 * 1024;
    let mut reader = compact_stream::open(stream_path)
        .map_err(|error| format!("open runtime-content stream failed: {error}"))?;
    let mut validator = match identity_mode {
        compact_contract::stream_validator::StreamIdentityMode::Legacy => {
            StatefulStreamValidator::legacy()
        }
        compact_contract::stream_validator::StreamIdentityMode::IdentityV1 => {
            StatefulStreamValidator::identity_v1()
        }
    };
    let mut hud_activations = Vec::new();
    let mut hud_roots = BTreeMap::<HudSurface, String>::new();
    let mut hud_partitions = BTreeSet::<Vec<(HudSurface, String)>>::new();
    let mut static_program_frames = Vec::new();
    let mut last_static_program_update_index = None;
    let mut m02_used_content = BTreeMap::<String, UsedContentKey>::new();
    let mut stream_offset = 0usize;
    loop {
        let mut header_bytes = [0u8; compact_contract::COMPACT_RECORD_HEADER_LEN];
        let count = read_stream_exact_or_eof(&mut reader, &mut header_bytes)
            .map_err(|error| format!("read runtime-content H4CR header failed: {error}"))?;
        if count == 0 {
            break;
        }
        if count != header_bytes.len() {
            return Err(format!(
                "runtime-content stream has truncated H4CR header at offset {stream_offset}"
            ));
        }
        let header = compact_contract::parse_compact_record_header(&header_bytes)
            .map_err(|error| format!("H4CR header at offset {stream_offset} failed: {error}"))?;
        if header.record_len() > MAX_RECORD_LEN {
            return Err(format!(
                "H4CR record at offset {stream_offset} exceeds {MAX_RECORD_LEN} byte bound"
            ));
        }
        let mut payload = vec![0u8; header.payload_len];
        if read_stream_exact_or_eof(&mut reader, &mut payload)
            .map_err(|error| format!("read runtime-content payload failed: {error}"))?
            != payload.len()
        {
            return Err(format!(
                "runtime-content stream has truncated payload at offset {stream_offset}"
            ));
        }
        // A SEMANTICALLY MALFORMED DOMAIN_DATA_REPEAT IS A LOST RECORD, NOT
        // A DEAD MANIFEST (Codex review round 15, finding 4): skip it by
        // name and clear the validator's stored failure so the stream is not
        // poisoned for every record after it. A repeat contributes no new
        // runtime content, so nothing is missing from this scan.
        let accepted = match validator.validate_record(&header, &payload) {
            Ok(accepted) => accepted,
            Err(
                error @ (compact_contract::stream_validator::StreamValidationError::InvalidRecordPayload(_)
                | compact_contract::stream_validator::StreamValidationError::PayloadLengthMismatch { .. }
                | compact_contract::stream_validator::StreamValidationError::ReservedDomainZero),
            ) if header.record_type
                == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT =>
            {
                // Only SEMANTIC failures are tolerable; sequence-level
                // errors stay fatal (Codex review round 18, finding 2).
                let _ = error;
                validator.tolerate_failed_record(&header);
                stream_offset = stream_offset
                    .checked_add(header.record_len())
                    .ok_or_else(|| "runtime-content stream offset overflow".to_string())?;
                continue;
            }
            Err(error) => {
                return Err(format!(
                    "H4CR validation at offset {stream_offset} failed: {error}"
                ))
            }
        };
        if accepted == AcceptedRecordKind::Ordinary
            && header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA
        {
            if header.domain_id == hud_role_observer_live_lane::DOMAIN_ID {
                let frame = hud_role_observer_live_lane::decode_activation_payload(&payload)
                    .map_err(|error| {
                        format!("d79 activation at {stream_offset} invalid: {error}")
                    })?;
                if frame.status_flags == 0 {
                    for row in &frame.active_surfaces {
                        match hud_roots.get(&row.surface) {
                            Some(existing) if existing != &row.runtime_root_name => {
                                return Err(format!(
                                    "d79 runtime root changed within recording for {}: {:?} -> {:?}",
                                    row.surface.token(),
                                    existing,
                                    row.runtime_root_name
                                ));
                            }
                            _ => {
                                hud_roots.insert(row.surface, row.runtime_root_name.clone());
                            }
                        }
                    }
                    let partition = frame
                        .active_surfaces
                        .iter()
                        .map(|row| (row.surface, row.runtime_root_name.clone()))
                        .collect::<Vec<_>>();
                    if hud_partitions.insert(partition) {
                        hud_activations.push(frame);
                    }
                }
            } else if header.domain_id == LEGACY_FIXED_ROLE_MAP_SNAPSHOT_DOMAIN_ID {
                return Err(format!(
                    "legacy fixed-role map snapshot at {stream_offset} is not admitted by the opaque production finalizer"
                ));
            } else if header.domain_id == PRODUCTION_RENDER_COMPACT_DOMAIN_ID {
                let frame = ProductionRenderFrame::decode_json(&payload).map_err(|error| {
                    format!("d77 production render at {stream_offset} invalid: {error}")
                })?;
                let (early_launch_manifest, early_capture_receipt) = early_admission.ok_or(
                    "production-render carrier has no authenticated suspended-launch receipt",
                )?;
                validate_render_frame_early_admission(
                    &frame,
                    early_launch_manifest,
                    early_capture_receipt,
                )?;
                if let Some(m02) = &frame.m02_combat {
                    for indicator in &m02.indicators {
                        for content in &indicator.used_content {
                            let path = content.normalized_game_relative_path.clone();
                            match m02_used_content.get(&path) {
                                Some(existing) if existing != content => {
                                    return Err(format!(
                                        "M02 used-content winner changed within recording for {path}"
                                    ));
                                }
                                _ => {
                                    m02_used_content.insert(path, content.clone());
                                }
                            }
                        }
                    }
                }
                let has_static_programs = frame
                    .display_submit_index
                    .as_deref()
                    .and_then(|index| index.parse::<u64>().ok())
                    .and_then(|index| frame.submit(index))
                    .is_some_and(|submit| !submit.ordered_static_map_programs.is_empty());
                if has_static_programs {
                    if last_static_program_update_index
                        .is_some_and(|previous| frame.update_index <= previous)
                    {
                        return Err(
                            "static-program render-frame update indexes are not strictly increasing"
                                .into(),
                        );
                    }
                    last_static_program_update_index = Some(frame.update_index);
                    static_program_frames.push(frame);
                }
            }
        }
        stream_offset = stream_offset
            .checked_add(header.record_len())
            .ok_or_else(|| "runtime-content stream offset overflow".to_string())?;
    }
    Ok(RuntimeContentSnapshots {
        hud_activations,
        static_program_frames,
        m02_used_content: m02_used_content.into_values().collect(),
    })
}

fn read_stream_exact_or_eof(reader: &mut impl Read, output: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0usize;
    while filled < output.len() {
        match reader.read(&mut output[filled..])? {
            0 => break,
            count => filled += count,
        }
    }
    Ok(filled)
}

fn build_from_finalized_static_bundle_inner(
    source_inventory_path: &Path,
    catalog_source_join_path: &Path,
    live_resolution_report_path: &Path,
    launch: &PortableLaunchBinding,
    product_modset_sha256: &str,
    hud_snapshot: Option<&HudRoleSnapshot>,
    static_program_content: &[StaticProgramContentObservation],
    runtime_ui_content: &[StaticProgramContentObservation],
    m02_used_content: &[UsedContentKey],
) -> Result<ContentManifest, String> {
    let inventory = read_stable_json(source_inventory_path, "live source inventory")?;
    if inventory.get("schema").and_then(serde_json::Value::as_str)
        != Some("extractor.live_binding_source_inventory.v3")
    {
        return Err("live source inventory is not v3".into());
    }
    let effective_vfs_manifest_sha256 = required_json_sha(
        inventory.get("inventory_sha256"),
        "live source inventory digest",
    )?;
    let identity = inventory
        .get("identity")
        .and_then(serde_json::Value::as_object)
        .ok_or("live source inventory identity is not an object")?;
    let profile_id = required_json_string(identity.get("profile_id"), "profile id")?;
    let playset_id = required_json_string(identity.get("playset_id"), "playset id")?;

    let namespace_values = inventory
        .get("namespaces")
        .and_then(serde_json::Value::as_array)
        .filter(|rows| !rows.is_empty())
        .ok_or("live source inventory has no namespaces")?;
    let mod_by_id: BTreeMap<_, _> = launch
        .ordered_mods
        .iter()
        .map(|row| (row.layer_id.as_str(), row))
        .collect();
    let mut raw_namespace_ids = Vec::with_capacity(namespace_values.len());
    let mut raw_to_layer_ordinal = Vec::with_capacity(namespace_values.len());
    let mut ordered_layers = Vec::with_capacity(namespace_values.len());
    for (ordinal, value) in namespace_values.iter().enumerate() {
        let row = value
            .as_object()
            .ok_or_else(|| format!("source namespace {ordinal} is not an object"))?;
        if row.get("ordinal").and_then(serde_json::Value::as_u64) != Some(ordinal as u64) {
            return Err(format!(
                "source namespace {ordinal} has a different ordinal"
            ));
        }
        let raw_kind = required_json_string(row.get("kind"), "source namespace kind")?;
        let raw_id = required_json_string(row.get("id"), "source namespace id")?;
        raw_namespace_ids.push((raw_kind.clone(), raw_id.clone()));
        if row.get("binding").and_then(serde_json::Value::as_str) == Some("asset_only") {
            raw_to_layer_ordinal.push(None);
            continue;
        }
        let (layer_kind, layer_id) = match raw_kind.as_str() {
            "vanilla" if ordinal == 0 => (ContentLayerKind::Base, "base".to_string()),
            "integrated_dlc" => {
                let bound = launch.integrated_dlc_layers.get(&raw_id).ok_or_else(|| {
                    format!("source namespace integrated DLC {raw_id:?} is absent from install")
                })?;
                (ContentLayerKind::IntegratedDlc, bound.layer_id.clone())
            }
            "dlc" => {
                let bound = launch.enabled_dlc_layers.get(&raw_id).ok_or_else(|| {
                    format!("source namespace DLC {raw_id:?} is not selected by the launch")
                })?;
                (ContentLayerKind::Dlc, bound.layer_id.clone())
            }
            "mod" => (ContentLayerKind::Mod, normalize_stable_layer_id(&raw_id)?),
            _ => {
                return Err(format!(
                    "unsupported source namespace kind/id {raw_kind}:{raw_id}"
                ))
            }
        };
        let _content_sha256 =
            required_json_sha(row.get("content_sha256"), "source namespace content digest")?;
        let binding_content_sha256 = required_json_sha(
            row.get("binding_content_sha256"),
            "source namespace binding digest",
        )?;
        let inventory_storage_kind =
            match row.get("storage_kind").and_then(serde_json::Value::as_str) {
                Some("directory") => LayerStorageKind::Directory,
                Some("zip_archive") => LayerStorageKind::ZipArchive,
                _ => return Err("source namespace has unsupported storage_kind".into()),
            };
        let inventory_source_container = match row.get("source_container_sha256") {
            Some(serde_json::Value::String(_)) => Some(required_json_sha(
                row.get("source_container_sha256"),
                "source namespace container",
            )?),
            Some(serde_json::Value::Null) | None => None,
            _ => return Err("source namespace container identity is malformed".into()),
        };
        let mod_binding = (layer_kind == ContentLayerKind::Mod)
            .then(|| mod_by_id.get(layer_id.as_str()).copied())
            .flatten();
        if layer_kind == ContentLayerKind::Mod && mod_binding.is_none() {
            return Err(format!(
                "source namespace mod {layer_id:?} is not in launch order"
            ));
        }
        if let Some(binding) = mod_binding {
            if binding.storage_kind != inventory_storage_kind {
                return Err(format!(
                    "source namespace mod {layer_id:?} storage kind differs from launch binding"
                ));
            }
            if binding.content_sha256 != binding_content_sha256 {
                return Err(format!(
                    "source namespace mod {layer_id:?} content digest differs from suspended-launch binding"
                ));
            }
        }
        let dlc_binding = match layer_kind {
            ContentLayerKind::Dlc => launch.enabled_dlc_layers.get(&raw_id),
            ContentLayerKind::IntegratedDlc => launch.integrated_dlc_layers.get(&raw_id),
            _ => None,
        };
        if let Some(binding) = dlc_binding {
            if binding.storage_kind != inventory_storage_kind {
                return Err(format!(
                    "source namespace DLC {layer_id:?} storage kind differs from descriptor"
                ));
            }
        }
        let storage_kind = dlc_binding
            .map(|binding| binding.storage_kind)
            .or_else(|| mod_binding.map(|binding| binding.storage_kind))
            .unwrap_or(inventory_storage_kind);
        let source_container_sha256 = dlc_binding
            .and_then(|binding| binding.source_container_sha256.clone())
            .or_else(|| {
                (storage_kind == LayerStorageKind::ZipArchive
                    && layer_kind == ContentLayerKind::Mod)
                    .then_some(binding_content_sha256.clone())
            });
        if source_container_sha256 != inventory_source_container {
            return Err("source namespace container identity differs from launch binding".into());
        }
        let raw_replace_paths = row
            .get("replace_paths")
            .and_then(serde_json::Value::as_array)
            .ok_or("source namespace replace_paths is not an array")?;
        let mut replace_paths = Vec::with_capacity(raw_replace_paths.len());
        let mut seen_replace_paths = BTreeSet::new();
        for value in raw_replace_paths {
            let normalized = normalize_game_relative_path(
                value
                    .as_str()
                    .ok_or("source namespace replace_path is not a string")?,
            )?;
            if !seen_replace_paths.insert(normalized.clone()) {
                return Err("source namespace repeats a replace_path after normalization".into());
            }
            replace_paths.push(normalized);
        }
        let layer_ordinal = ordered_layers.len() as u32;
        raw_to_layer_ordinal.push(Some(layer_ordinal));
        ordered_layers.push(RecordedContentLayer {
            ordinal: layer_ordinal,
            layer_kind,
            layer_id,
            display_name: required_json_string(row.get("label"), "source namespace label")?,
            descriptor_sha256: dlc_binding
                .map(|binding| binding.descriptor_sha256.clone())
                .or_else(|| mod_binding.map(|binding| binding.descriptor_sha256.clone())),
            source_container_sha256,
            storage_kind,
            replace_paths,
            required_for_playback: false,
        });
    }
    if mod_by_id.len()
        != ordered_layers
            .iter()
            .filter(|layer| layer.layer_kind == ContentLayerKind::Mod)
            .count()
    {
        return Err("source namespace mod roster differs from launch binding".into());
    }
    let inventory_winners = source_inventory_winners(
        &inventory,
        &ordered_layers,
        &raw_to_layer_ordinal,
        &effective_vfs_manifest_sha256,
    )?;
    let exact_winner_binding =
        crate::production_vfs_winner_binding::build_effective_vfs_winner_binding_from_inventory(
            source_inventory_path,
            &ordered_layers,
            1,
            &effective_vfs_manifest_sha256,
        )?;
    if exact_winner_binding.ordered_rows.len() != inventory_winners.len()
        || exact_winner_binding.ordered_rows.iter().any(|row| {
            inventory_winners
                .get(&row.content.logical_path)
                .is_none_or(|expected| {
                    expected.normalized_game_relative_path != row.content.logical_path
                        || expected.winning_layer_ordinal != row.content.winning_layer_ordinal
                        || match expected.layer_kind {
                            ContentLayerKind::Base => "base",
                            ContentLayerKind::IntegratedDlc => "integrated_dlc",
                            ContentLayerKind::Dlc => "dlc",
                            ContentLayerKind::Mod => "mod",
                        } != row.content.layer_kind
                        || expected.layer_id != row.content.layer_id
                        || expected.source_sha256 != row.content.source_sha256
                        || expected.payload_sha256 != row.content.payload_sha256
                        || expected.payload_bytes != row.content.payload_bytes
                })
        })
    {
        return Err(
            "source inventory v3 exact winner binding disagrees with manifest winners".into(),
        );
    }

    let join = read_stable_json(catalog_source_join_path, "catalog source join")?;
    if join.get("schema").and_then(serde_json::Value::as_str)
        != Some("extractor.static_catalog_source_join.v1")
        || join.get("passed").and_then(serde_json::Value::as_bool) != Some(true)
        || join
            .get("inventory_sha256")
            .and_then(serde_json::Value::as_str)
            != Some(effective_vfs_manifest_sha256.as_str())
    {
        return Err("catalog source join is not a passed binding to this source inventory".into());
    }
    let binding_values = join
        .get("bindings")
        .and_then(serde_json::Value::as_array)
        .filter(|rows| !rows.is_empty())
        .ok_or("catalog source join has no used source bindings")?;
    let mut observations = Vec::with_capacity(binding_values.len());
    for (index, value) in binding_values.iter().enumerate() {
        let row = value
            .as_object()
            .ok_or_else(|| format!("catalog source binding {index} is not an object"))?;
        let namespace_ordinal = row
            .get("namespace_ordinal")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| format!("catalog source binding {index} has no u32 ordinal"))?;
        let layer_ordinal = raw_to_layer_ordinal
            .get(namespace_ordinal as usize)
            .copied()
            .flatten()
            .ok_or_else(|| {
                format!("catalog source binding {index} names an asset-only or missing namespace")
            })?;
        let layer = ordered_layers.get(layer_ordinal as usize).ok_or_else(|| {
            format!("catalog source binding {index} names a missing playback layer")
        })?;
        let raw_namespace = &raw_namespace_ids[namespace_ordinal as usize];
        if row.get("layer_kind").and_then(serde_json::Value::as_str)
            != Some(raw_namespace.0.as_str())
            || row.get("layer_id").and_then(serde_json::Value::as_str)
                != Some(raw_namespace.1.as_str())
        {
            return Err(format!(
                "catalog source binding {index} disagrees with its namespace"
            ));
        }
        let relpath = normalize_game_relative_path(&required_json_string(
            row.get("relpath"),
            "catalog source relpath",
        )?)?;
        let payload_sha256 = required_json_sha(row.get("sha256"), "catalog source payload")?;
        let payload_bytes = row
            .get("bytes")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("catalog source binding {index} has no byte count"))?;
        let artifact_role = required_json_string(row.get("artifact_role"), "artifact role")?;
        observations.push(ContentUseObservation {
            content_key: relpath.clone(),
            kind: required_kind_for_path(&relpath),
            content: UsedContentKey {
                effective_vfs_manifest_sha256: effective_vfs_manifest_sha256.clone(),
                normalized_game_relative_path: relpath,
                winning_layer_ordinal: layer_ordinal,
                layer_kind: layer.layer_kind,
                layer_id: layer.layer_id.clone(),
                source_sha256: layer
                    .source_container_sha256
                    .clone()
                    .unwrap_or_else(|| payload_sha256.clone()),
                payload_sha256,
                payload_bytes,
            },
            consumer: format!("static_catalog/{artifact_role}"),
        });
    }
    if let Some(snapshot) = hud_snapshot {
        append_hud_snapshot_observations(
            &mut observations,
            snapshot,
            &effective_vfs_manifest_sha256,
            &inventory_winners,
        )?;
    }
    append_complete_authored_namespace_observations(&mut observations, &inventory_winners)?;
    append_static_program_content_observations(
        &mut observations,
        static_program_content,
        &effective_vfs_manifest_sha256,
        &ordered_layers,
    )?;
    append_runtime_ui_content_observations(
        &mut observations,
        runtime_ui_content,
        &effective_vfs_manifest_sha256,
        &ordered_layers,
    )?;
    append_m02_content_observations(
        &mut observations,
        m02_used_content,
        &effective_vfs_manifest_sha256,
        &inventory_winners,
    )?;

    let live_report = read_stable_json(live_resolution_report_path, "live resolution report")?;
    let runtime_version = live_report
        .pointer("/runtime/version")
        .and_then(serde_json::Value::as_str)
        .ok_or("live resolution report omitted runtime version")?;
    let (version, build_id) = runtime_version.rsplit_once('.').ok_or_else(|| {
        format!("runtime version has no separate build component: {runtime_version:?}")
    })?;
    if version.trim().is_empty() || build_id.trim().is_empty() {
        return Err("runtime version/build is blank".into());
    }
    let trace = RecorderContentTrace {
        schema: TRACE_SCHEMA.into(),
        source_game: SourceGameIdentity {
            version: version.to_string(),
            build: build_id.to_string(),
            executable_sha256: launch.executable_sha256.clone(),
            build_profile: format!("hoi4-{version}-{build_id}"),
        },
        source_profile: SourceProfileIdentity {
            profile_id,
            playset_id,
            selection_sha256: "0".repeat(64),
        },
        effective_vfs_manifest_sha256,
        ordered_layers,
        dlc_selection: launch
            .enabled_dlc_ids
            .iter()
            .map(|dlc_id| DlcSelection {
                dlc_id: dlc_id.clone(),
                enabled: true,
                required_for_playback: false,
            })
            .collect(),
        localisation_language: launch.localisation_language.clone(),
        required_settings: launch.required_settings.clone(),
        recorder_local_settings: launch.recorder_local_settings.clone(),
        observations,
    };
    let manifest = build(trace, launch, product_modset_sha256)?;
    verify_live_resolution_report(live_resolution_report_path, &manifest, launch)?;
    Ok(manifest)
}

fn append_hud_snapshot_observations(
    observations: &mut Vec<ContentUseObservation>,
    snapshot: &HudRoleSnapshot,
    effective_vfs_manifest_sha256: &str,
    inventory_winners: &BTreeMap<String, UsedContentKey>,
) -> Result<(), String> {
    snapshot
        .validate()
        .map_err(|error| format!("invalid HUD role snapshot: {error}"))?;
    if snapshot.authority_manifest_sha256 != effective_vfs_manifest_sha256 {
        return Err("HUD role snapshot names a different effective VFS".into());
    }
    if observations
        .len()
        .saturating_add(snapshot.observations.len())
        > MAX_OBSERVATIONS
    {
        return Err(format!(
            "portable content trace would exceed the {MAX_OBSERVATIONS} observation bound"
        ));
    }
    let mut seen_consumers = BTreeSet::new();
    for row in &snapshot.observations {
        if !row.consumer.starts_with("hud_role/v1/") || !seen_consumers.insert(&row.consumer) {
            return Err("HUD role snapshot has an invalid or duplicate semantic consumer".into());
        }
        row.content.validate()?;
        if row.content.effective_vfs_manifest_sha256 != effective_vfs_manifest_sha256 {
            return Err(format!(
                "HUD role {} names a different effective VFS",
                row.consumer
            ));
        }
        let normalized = normalize_game_relative_path(&row.content_key)?;
        if normalized != row.content_key || normalized != row.content.normalized_game_relative_path
        {
            return Err(format!(
                "HUD role {} content key/path is not one canonical VFS path",
                row.consumer
            ));
        }
        let expected = inventory_winners.get(&normalized).ok_or_else(|| {
            format!(
                "HUD role {} path is not an effective source-inventory winner",
                row.consumer
            )
        })?;
        if expected != &row.content {
            return Err(format!(
                "HUD role {} UsedContentKey differs from the effective source-inventory winner",
                row.consumer
            ));
        }
        observations.push(ContentUseObservation {
            content_key: row.content_key.clone(),
            kind: required_kind_for_path(&normalized),
            content: row.content.clone(),
            consumer: row.consumer.clone(),
        });
    }
    Ok(())
}

fn source_inventory_winners(
    inventory: &serde_json::Value,
    ordered_layers: &[RecordedContentLayer],
    raw_to_layer_ordinal: &[Option<u32>],
    authority: &str,
) -> Result<BTreeMap<String, UsedContentKey>, String> {
    let values = inventory
        .get("rows")
        .and_then(serde_json::Value::as_array)
        .filter(|rows| !rows.is_empty())
        .ok_or("live source inventory has no file rows")?;
    let mut rows = BTreeMap::<(u32, String), (String, String, u64)>::new();
    let mut path_keys = BTreeSet::new();
    for (index, value) in values.iter().enumerate() {
        let row = value
            .as_object()
            .ok_or_else(|| format!("source inventory row {index} is not an object"))?;
        let raw_ordinal = row
            .get("namespace_ordinal")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| format!("source inventory row {index} has no u32 ordinal"))?;
        let Some(ordinal) = raw_to_layer_ordinal
            .get(raw_ordinal as usize)
            .copied()
            .flatten()
        else {
            continue;
        };
        if ordered_layers
            .get(ordinal as usize)
            .filter(|layer| layer.ordinal == ordinal)
            .is_none()
        {
            return Err(format!(
                "source inventory row {index} names a missing playback layer"
            ));
        }
        let path = normalize_game_relative_path(&required_json_string(
            row.get("relpath"),
            "source inventory relpath",
        )?)?;
        let payload_sha256 =
            required_json_sha(row.get("sha256"), "source inventory payload digest")?;
        let payload_bytes = row
            .get("bytes")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("source inventory row {index} has no byte count"))?;
        let path_key = filesystem_case_key(&path);
        if rows
            .insert(
                (ordinal, path_key.clone()),
                (path.clone(), payload_sha256, payload_bytes),
            )
            .is_some()
        {
            return Err(format!(
                "source inventory repeats or filesystem-case-collides at namespace/path {ordinal}:{path}"
            ));
        }
        path_keys.insert(path_key);
    }
    let mut winners = BTreeMap::new();
    for path_key in path_keys {
        let parent_key = path_key
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or_default();
        for layer in ordered_layers.iter().rev() {
            if let Some((path, payload_sha256, payload_bytes)) =
                rows.get(&(layer.ordinal, path_key.clone()))
            {
                let content = UsedContentKey {
                    effective_vfs_manifest_sha256: authority.to_string(),
                    normalized_game_relative_path: path.clone(),
                    winning_layer_ordinal: layer.ordinal,
                    layer_kind: layer.layer_kind,
                    layer_id: layer.layer_id.clone(),
                    source_sha256: layer
                        .source_container_sha256
                        .clone()
                        .unwrap_or_else(|| payload_sha256.clone()),
                    payload_sha256: payload_sha256.clone(),
                    payload_bytes: *payload_bytes,
                };
                content.validate()?;
                winners.insert(path.clone(), content);
                break;
            }
            if layer
                .replace_paths
                .iter()
                .any(|replace| filesystem_case_key(replace) == parent_key)
            {
                break;
            }
        }
    }
    Ok(winners)
}

fn append_complete_authored_namespace_observations(
    observations: &mut Vec<ContentUseObservation>,
    inventory_winners: &BTreeMap<String, UsedContentKey>,
) -> Result<(), String> {
    let matching = inventory_winners
        .iter()
        .filter_map(|(path, content)| {
            let folded = path.to_ascii_lowercase();
            let consumer = if folded.starts_with("interface/") && folded.ends_with(".gui") {
                Some(AUTHORED_GUI_NAMESPACE_CONSUMER)
            } else if folded.starts_with("interface/") && folded.ends_with(".gfx") {
                Some(AUTHORED_GFX_NAMESPACE_CONSUMER)
            } else if folded.starts_with("localisation/") && folded.ends_with(".yml") {
                Some(AUTHORED_LOCALISATION_NAMESPACES_CONSUMER)
            } else {
                None
            }?;
            Some((path, content, consumer))
        })
        .collect::<Vec<_>>();
    if matching.len() < 3
        || !matching
            .iter()
            .any(|(_, _, consumer)| *consumer == AUTHORED_GUI_NAMESPACE_CONSUMER)
        || !matching
            .iter()
            .any(|(_, _, consumer)| *consumer == AUTHORED_GFX_NAMESPACE_CONSUMER)
        || !matching
            .iter()
            .any(|(_, _, consumer)| *consumer == AUTHORED_LOCALISATION_NAMESPACES_CONSUMER)
    {
        return Err(format!(
            "production source inventory does not contain complete authored GUI/GFX/localisation namespaces"
        ));
    }
    if observations.len().saturating_add(matching.len()) > MAX_OBSERVATIONS {
        return Err(format!(
            "portable content trace would exceed the {MAX_OBSERVATIONS} observation bound"
        ));
    }
    let mut folded_paths = BTreeSet::new();
    for (path, content, consumer) in matching {
        content.validate()?;
        if path != &content.normalized_game_relative_path
            || !folded_paths.insert(filesystem_case_key(path))
        {
            return Err(format!(
                "authored namespace winner key differs from its content identity or case-collides: {path}"
            ));
        }
        observations.push(ContentUseObservation {
            content_key: path.clone(),
            kind: required_kind_for_path(path),
            content: content.clone(),
            consumer: consumer.into(),
        });
    }
    Ok(())
}

fn append_static_program_content_observations(
    observations: &mut Vec<ContentUseObservation>,
    static_program_content: &[StaticProgramContentObservation],
    effective_vfs_manifest_sha256: &str,
    ordered_layers: &[RecordedContentLayer],
) -> Result<(), String> {
    if observations
        .len()
        .saturating_add(static_program_content.len())
        > MAX_OBSERVATIONS
    {
        return Err(format!(
            "portable content trace would exceed the {MAX_OBSERVATIONS} observation bound"
        ));
    }
    let mut seen_pairs = BTreeSet::new();
    for row in static_program_content {
        let identity = row
            .consumer
            .strip_prefix(STATIC_PROGRAM_CONSUMER_PREFIX)
            .ok_or("static-program content has a non-canonical consumer")?;
        if identity.len() != 64
            || !identity.bytes().all(|byte| byte.is_ascii_hexdigit())
            || identity != identity.to_ascii_lowercase()
            || !seen_pairs.insert((row.consumer.as_str(), row.content_key.as_str()))
        {
            return Err("static-program content has an invalid identity or duplicate key".into());
        }
        row.content.validate()?;
        if row.content.effective_vfs_manifest_sha256 != effective_vfs_manifest_sha256 {
            return Err(format!(
                "static program {identity} names a different effective VFS"
            ));
        }
        let normalized = normalize_game_relative_path(&row.content_key)?;
        if normalized != row.content_key || normalized != row.content.normalized_game_relative_path
        {
            return Err(format!(
                "static program {identity} content key/path is not one canonical VFS path"
            ));
        }
        let layer = ordered_layers
            .get(row.content.winning_layer_ordinal as usize)
            .filter(|layer| layer.ordinal == row.content.winning_layer_ordinal)
            .ok_or_else(|| format!("static program {identity} names a missing winning layer"))?;
        if layer.layer_kind != row.content.layer_kind || layer.layer_id != row.content.layer_id {
            return Err(format!(
                "static program {identity} winner kind/id disagrees with launch order"
            ));
        }
        match &layer.source_container_sha256 {
            Some(expected) if expected != &row.content.source_sha256 => {
                return Err(format!(
                    "static program {identity} source container hash disagrees with winning layer"
                ));
            }
            None if row.content.source_sha256 != row.content.payload_sha256 => {
                return Err(format!(
                    "static program {identity} directory source hash differs from payload"
                ));
            }
            _ => {}
        }
        observations.push(ContentUseObservation {
            content_key: normalized.clone(),
            kind: required_kind_for_path(&normalized),
            content: row.content.clone(),
            consumer: row.consumer.clone(),
        });
    }
    Ok(())
}

fn append_runtime_ui_content_observations(
    observations: &mut Vec<ContentUseObservation>,
    runtime_ui_content: &[StaticProgramContentObservation],
    effective_vfs_manifest_sha256: &str,
    ordered_layers: &[RecordedContentLayer],
) -> Result<(), String> {
    if observations.len().saturating_add(runtime_ui_content.len()) > MAX_OBSERVATIONS {
        return Err(format!(
            "portable content trace would exceed the {MAX_OBSERVATIONS} observation bound"
        ));
    }
    let mut seen_pairs = BTreeSet::new();
    for row in runtime_ui_content {
        let identity = row
            .consumer
            .strip_prefix(RUNTIME_UI_CONSUMER_PREFIX)
            .ok_or("runtime UI content has a non-canonical consumer")?;
        if identity.len() != 64
            || !identity.bytes().all(|byte| byte.is_ascii_hexdigit())
            || identity != identity.to_ascii_lowercase()
            || !seen_pairs.insert((row.consumer.as_str(), row.content_key.as_str()))
        {
            return Err("runtime UI content has an invalid identity or duplicate key".into());
        }
        row.content.validate()?;
        if row.content.effective_vfs_manifest_sha256 != effective_vfs_manifest_sha256 {
            return Err(format!(
                "runtime UI command {identity} names a different effective VFS"
            ));
        }
        let normalized = normalize_game_relative_path(&row.content_key)?;
        if normalized != row.content_key || normalized != row.content.normalized_game_relative_path
        {
            return Err(format!(
                "runtime UI command {identity} content key/path is not one canonical VFS path"
            ));
        }
        let layer = ordered_layers
            .get(row.content.winning_layer_ordinal as usize)
            .filter(|layer| layer.ordinal == row.content.winning_layer_ordinal)
            .ok_or_else(|| {
                format!("runtime UI command {identity} names a missing winning layer")
            })?;
        if layer.layer_kind != row.content.layer_kind || layer.layer_id != row.content.layer_id {
            return Err(format!(
                "runtime UI command {identity} winner kind/id disagrees with launch order"
            ));
        }
        match &layer.source_container_sha256 {
            Some(expected) if expected != &row.content.source_sha256 => {
                return Err(format!(
                    "runtime UI command {identity} source container hash disagrees with winning layer"
                ));
            }
            None if row.content.source_sha256 != row.content.payload_sha256 => {
                return Err(format!(
                    "runtime UI command {identity} directory source hash differs from payload"
                ));
            }
            _ => {}
        }
        observations.push(ContentUseObservation {
            content_key: normalized.clone(),
            kind: required_kind_for_path(&normalized),
            content: row.content.clone(),
            consumer: row.consumer.clone(),
        });
    }
    Ok(())
}

fn append_m02_content_observations(
    observations: &mut Vec<ContentUseObservation>,
    used_content: &[UsedContentKey],
    effective_vfs_manifest_sha256: &str,
    inventory_winners: &BTreeMap<String, UsedContentKey>,
) -> Result<(), String> {
    if observations.len().saturating_add(used_content.len()) > MAX_OBSERVATIONS {
        return Err(format!(
            "portable content trace would exceed the {MAX_OBSERVATIONS} observation bound"
        ));
    }
    let mut seen = BTreeSet::new();
    for content in used_content {
        content
            .validate()
            .map_err(|error| format!("invalid M02 UsedContentKey: {error}"))?;
        if content.effective_vfs_manifest_sha256 != effective_vfs_manifest_sha256 {
            return Err(format!(
                "M02 content {} names a different effective VFS",
                content.normalized_game_relative_path
            ));
        }
        let normalized = normalize_game_relative_path(&content.normalized_game_relative_path)?;
        if normalized != content.normalized_game_relative_path || !seen.insert(normalized.clone()) {
            return Err("M02 used-content paths are non-canonical or duplicated".into());
        }
        let expected = inventory_winners.get(&normalized).ok_or_else(|| {
            format!("M02 path {normalized} is not an effective source-inventory winner")
        })?;
        if expected != content {
            return Err(format!(
                "M02 UsedContentKey for {normalized} differs from the effective source-inventory winner"
            ));
        }
        observations.push(ContentUseObservation {
            content_key: normalized.clone(),
            kind: required_kind_for_path(&normalized),
            content: content.clone(),
            consumer: M02_CONTENT_CONSUMER.into(),
        });
    }
    Ok(())
}

fn read_stable_json(path: &Path, label: &str) -> Result<serde_json::Value, String> {
    let first = fs::read(path).map_err(|error| format!("read {label} failed: {error}"))?;
    let second = fs::read(path).map_err(|error| format!("re-read {label} failed: {error}"))?;
    if first != second {
        return Err(format!("{label} changed while finalizing the recording"));
    }
    serde_json::from_slice(&first).map_err(|error| format!("parse {label} failed: {error}"))
}

fn required_json_string(value: Option<&serde_json::Value>, label: &str) -> Result<String, String> {
    value
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{label} is blank or missing"))
}

fn required_json_sha(value: Option<&serde_json::Value>, label: &str) -> Result<String, String> {
    let value = required_json_string(value, label)?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{label} is not SHA-256"));
    }
    Ok(value.to_ascii_lowercase())
}

fn required_kind_for_path(path: &str) -> RequiredContentKind {
    let folded_path = path.to_ascii_lowercase();
    let extension = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if folded_path.starts_with("localisation/") {
        RequiredContentKind::Localisation
    } else if folded_path.starts_with("interface/") {
        RequiredContentKind::GuiDefinition
    } else if extension == "shader" {
        RequiredContentKind::ShaderSource
    } else if matches!(extension.as_str(), "dds" | "tga" | "bmp" | "png") {
        RequiredContentKind::Texture
    } else if extension == "mesh" {
        RequiredContentKind::Mesh
    } else if matches!(extension.as_str(), "ttf" | "otf" | "fnt") {
        RequiredContentKind::Font
    } else if matches!(extension.as_str(), "bin" | "dxbc") {
        RequiredContentKind::CompiledShader
    } else if folded_path.starts_with("gfx/") {
        RequiredContentKind::Definition
    } else {
        RequiredContentKind::Definition
    }
}

fn verify_live_resolution_report(
    path: &Path,
    manifest: &ContentManifest,
    launch: &PortableLaunchBinding,
) -> Result<(), String> {
    let first = fs::read(path)
        .map_err(|error| format!("read live binding resolution report failed: {error}"))?;
    let second = fs::read(path)
        .map_err(|error| format!("re-read live binding resolution report failed: {error}"))?;
    if first != second {
        return Err("live binding resolution report changed during finalization".into());
    }
    let report: serde_json::Value = serde_json::from_slice(&first)
        .map_err(|error| format!("parse live binding resolution report failed: {error}"))?;
    if report.get("schema").and_then(serde_json::Value::as_str)
        != Some("extractor.live_binding_resolution.v1")
        || report.get("passed").and_then(serde_json::Value::as_bool) != Some(true)
    {
        return Err("live binding resolution report is not a passed v1 report".into());
    }
    if report
        .pointer("/process/exe_sha256")
        .and_then(serde_json::Value::as_str)
        != Some(launch.executable_sha256.as_str())
    {
        return Err("live binding report executable hash differs from launch binding".into());
    }
    crate::production_vfs_winner_binding::require_engine_runtime_artifact_declaration(&report)?;
    let runtime_version = report
        .pointer("/runtime/version")
        .and_then(serde_json::Value::as_str)
        .ok_or("live binding report omitted runtime version")?;
    if !runtime_version.contains(&manifest.source_game.version)
        || !runtime_version.contains(&manifest.source_game.build)
    {
        return Err(format!(
            "portable source version/build {}/{} is not attested by runtime version {runtime_version:?}",
            manifest.source_game.version, manifest.source_game.build
        ));
    }
    let expected_profile = format!(
        "hoi4-{}-{}",
        manifest.source_game.version, manifest.source_game.build
    );
    if manifest.source_game.build_profile != expected_profile {
        return Err(format!(
            "portable source build_profile must be reproducible {expected_profile:?}"
        ));
    }
    for (field, expected) in [
        ("profile_id", manifest.source_profile.profile_id.as_str()),
        ("playset_id", manifest.source_profile.playset_id.as_str()),
    ] {
        if report
            .pointer(&format!("/binding_identity/{field}"))
            .and_then(serde_json::Value::as_str)
            != Some(expected)
        {
            return Err(format!(
                "portable source profile {field} differs from live binding report"
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachCorpusCensusStats {
    pub files: u64,
    pub payload_bytes: u64,
    pub elapsed_ms: u64,
    /// Paths the walk SAW and the census deliberately did not record, because
    /// they name the engine's own install-root write sinks rather than content
    /// (class `engine_written_install_root_sink`). Named, never silent: the
    /// manifest publishes this list so a reader can see exactly what the
    /// recording chose not to bind replay to.
    pub excluded_engine_written_sinks: Vec<String>,
}

const MAX_ATTACH_CORPUS_ROWS: usize = 400_000;

pub fn census_effective_vfs_corpus(
    vfs: &portable_content_manifest::effective_vfs::EffectiveVfs,
) -> Result<(Vec<RequiredContent>, AttachCorpusCensusStats), String> {
    let started = std::time::Instant::now();
    let mut rows: BTreeMap<String, RequiredContent> = BTreeMap::new();
    let mut excluded_engine_written_sinks: Vec<String> = Vec::new();
    let stats = vfs.walk_content_keys(|logical_path, content| {
        if rows.len() >= MAX_ATTACH_CORPUS_ROWS {
            return Err(format!(
                "attach-time content census exceeded its {MAX_ATTACH_CORPUS_ROWS}-row ceiling"
            ));
        }
        let normalized = normalize_game_relative_path(logical_path)?;
        if normalized != content.normalized_game_relative_path {
            return Err(format!(
                "attach-time content census path is not canonical: {logical_path}"
            ));
        }
        content.validate()?;
        // The engine writes these itself and never resolves them through the
        // VFS, so they are its scratch and not this install's content. Binding
        // replay to them makes every later session a content mismatch: playing
        // one console command rewrites `console_history.txt`, and the recording
        // that captured yesterday's copy refuses to open today. Owner ruling
        // 2026-08-15 already put the class out of the product-modset key; the
        // effective-VFS census was the remaining place still collecting it.
        if portable_content_manifest::is_engine_written_install_root_sink(
            &normalized,
            content.layer_kind,
        ) {
            excluded_engine_written_sinks.push(normalized);
            return Ok(());
        }
        if rows
            .insert(
                normalized.clone(),
                RequiredContent {
                    content_key: normalized.clone(),
                    kind: attach_census_content_kind(&normalized),
                    content,
                    observation_count: 1,
                    consumers: vec![ATTACH_CENSUS_CONSUMER.to_string()],
                },
            )
            .is_some()
        {
            return Err(format!(
                "attach-time content census repeated path {normalized}"
            ));
        }
        Ok(())
    })?;
    if rows.is_empty() {
        return Err("attach-time content census found no content in the effective VFS".into());
    }
    Ok((
        rows.into_values().collect(),
        AttachCorpusCensusStats {
            files: stats.files,
            payload_bytes: stats.payload_bytes,
            elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            excluded_engine_written_sinks,
        },
    ))
}

pub const ATTACH_CENSUS_CONSUMER: &str = "attach_time_effective_vfs_census";

fn attach_census_content_kind(normalized_path: &str) -> RequiredContentKind {
    let extension = normalized_path
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "txt" | "lua" | "settings" | "info" | "csv" => RequiredContentKind::Definition,
        "shader" | "fxh" | "hlsl" => RequiredContentKind::ShaderSource,
        "bin" | "cso" => RequiredContentKind::CompiledShader,
        "dds" | "tga" | "png" | "jpg" | "jpeg" => RequiredContentKind::Texture,
        "mesh" => RequiredContentKind::Mesh,
        "anim" | "asset" => RequiredContentKind::Geometry,
        "yml" | "yaml" => RequiredContentKind::Localisation,
        "gui" => RequiredContentKind::GuiDefinition,
        "ttf" | "otf" | "fnt" => RequiredContentKind::Font,
        "json" => RequiredContentKind::Catalog,
        _ => RequiredContentKind::Other,
    }
}

pub fn build_from_attach_time_corpus(
    ordered_layers: Vec<RecordedContentLayer>,
    required_content: Vec<RequiredContent>,
    launch: &PortableLaunchBinding,
    effective_vfs_manifest_sha256: &str,
    product_modset_sha256: &str,
    source_game: SourceGameIdentity,
    profile_id: &str,
    playset_id: &str,
) -> Result<ContentManifest, String> {
    portable_content_manifest::hud_role::validate_sha256(product_modset_sha256, "product modset")?;
    if source_game.executable_sha256 != launch.executable_sha256 {
        return Err(
            "attach-time content manifest executable differs from the attached process".into(),
        );
    }
    if required_content.is_empty() {
        return Err("attach-time content manifest has no required content".into());
    }
    if ordered_layers.is_empty() {
        return Err("attach-time content manifest has no content layers".into());
    }

    let required_ordinals: BTreeSet<u32> = required_content
        .iter()
        .map(|row| row.content.winning_layer_ordinal)
        .collect();
    let mut ordered_layers = ordered_layers;
    for (position, layer) in ordered_layers.iter_mut().enumerate() {
        if layer.ordinal as usize != position {
            return Err("attach-time content layer ordinals are not contiguous".into());
        }

        layer.required_for_playback = required_ordinals.contains(&layer.ordinal);
    }
    for row in &required_content {
        if row.content.effective_vfs_manifest_sha256 != effective_vfs_manifest_sha256 {
            return Err(format!(
                "attach-time content census row {} names a different effective VFS",
                row.content.normalized_game_relative_path
            ));
        }
        let layer = ordered_layers
            .get(row.content.winning_layer_ordinal as usize)
            .ok_or_else(|| {
                format!(
                    "attach-time content census row {} names a missing layer",
                    row.content.normalized_game_relative_path
                )
            })?;
        if layer.layer_kind != row.content.layer_kind || layer.layer_id != row.content.layer_id {
            return Err(format!(
                "attach-time content census row {} disagrees with its winning layer",
                row.content.normalized_game_relative_path
            ));
        }
    }

    let required_dlc_ids: BTreeSet<String> = ordered_layers
        .iter()
        .filter(|layer| layer.layer_kind == ContentLayerKind::Dlc && layer.required_for_playback)
        .map(|layer| stable_dlc_id(&layer.layer_id))
        .collect::<Result<_, _>>()?;
    let dlc_selection = launch
        .enabled_dlc_ids
        .iter()
        .map(|dlc_id| DlcSelection {
            dlc_id: dlc_id.clone(),
            enabled: true,
            required_for_playback: required_dlc_ids.contains(dlc_id),
        })
        .collect::<Vec<_>>();

    let mut manifest = ContentManifest {
        schema: CONTENT_MANIFEST_SCHEMA.to_string(),
        product_modset_sha256: product_modset_sha256.to_string(),
        source_game,
        source_profile: SourceProfileIdentity {
            profile_id: profile_id.to_string(),
            playset_id: playset_id.to_string(),
            selection_sha256: "0".repeat(64),
        },
        effective_vfs_manifest_sha256: effective_vfs_manifest_sha256.to_string(),
        ordered_layers,
        dlc_selection,
        localisation_language: launch.localisation_language.clone(),
        required_settings: launch.required_settings.clone(),
        recorder_local_settings: launch.recorder_local_settings.clone(),
        required_content,
        manifest_digest_sha256: None,
    };
    manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256()?;
    manifest.with_computed_digest()
}

pub fn build(
    trace: RecorderContentTrace,
    launch: &PortableLaunchBinding,
    product_modset_sha256: &str,
) -> Result<ContentManifest, String> {
    portable_content_manifest::hud_role::validate_sha256(product_modset_sha256, "product modset")?;
    if trace.schema != TRACE_SCHEMA {
        return Err(format!(
            "unsupported portable content trace schema {}",
            trace.schema
        ));
    }
    if trace.observations.is_empty() {
        return Err("production portable content trace has no observed content".into());
    }
    if trace.observations.len() > MAX_OBSERVATIONS {
        return Err(format!(
            "portable content trace has {} observations, above the {MAX_OBSERVATIONS} bound",
            trace.observations.len()
        ));
    }
    validate_launch_binding(&trace, launch)?;

    let layer_by_ordinal: BTreeMap<u32, &RecordedContentLayer> = trace
        .ordered_layers
        .iter()
        .map(|layer| (layer.ordinal, layer))
        .collect();
    if layer_by_ordinal.len() != trace.ordered_layers.len() {
        return Err("portable content trace repeats a layer ordinal".into());
    }

    let mut uses: BTreeMap<String, AggregatedUse> = BTreeMap::new();
    for observation in trace.observations {
        let normalized =
            normalize_game_relative_path(&observation.content.normalized_game_relative_path)?;
        if normalized != observation.content.normalized_game_relative_path {
            return Err(format!(
                "content observation path is not canonical: {}",
                observation.content.normalized_game_relative_path
            ));
        }
        if observation.content.effective_vfs_manifest_sha256 != trace.effective_vfs_manifest_sha256
        {
            return Err(format!(
                "content observation {normalized} names a different effective VFS"
            ));
        }
        let layer = layer_by_ordinal
            .get(&observation.content.winning_layer_ordinal)
            .ok_or_else(|| format!("content observation {normalized} names a missing layer"))?;
        if layer.layer_kind != observation.content.layer_kind
            || layer.layer_id != observation.content.layer_id
        {
            return Err(format!(
                "content observation {normalized} disagrees with its winning layer"
            ));
        }
        if observation.content_key.trim().is_empty() || observation.consumer.trim().is_empty() {
            return Err(format!(
                "content observation {normalized} has blank identity/consumer"
            ));
        }
        match uses.get_mut(&normalized) {
            None => {
                let mut consumers = BTreeSet::new();
                consumers.insert(observation.consumer);
                uses.insert(
                    normalized,
                    AggregatedUse {
                        content_key: observation.content_key,
                        kind: observation.kind,
                        content: observation.content,
                        consumers,
                        observation_count: 1,
                    },
                );
            }
            Some(existing) => {
                if existing.content_key != observation.content_key
                    || existing.kind != observation.kind
                    || existing.content != observation.content
                {
                    return Err(format!(
                        "repeated content path {normalized} changed key/kind/winner/hash during recording"
                    ));
                }
                existing.consumers.insert(observation.consumer);
                existing.observation_count =
                    existing.observation_count.checked_add(1).ok_or_else(|| {
                        format!("content observation count overflow for {normalized}")
                    })?;
            }
        }
    }

    let required_ordinals: BTreeSet<u32> = uses
        .values()
        .map(|entry| entry.content.winning_layer_ordinal)
        .collect();
    let mut ordered_layers = trace.ordered_layers;
    for layer in &mut ordered_layers {
        layer.required_for_playback = required_ordinals.contains(&layer.ordinal);
    }
    let required_dlc_ids: BTreeSet<String> = ordered_layers
        .iter()
        .filter(|layer| layer.layer_kind == ContentLayerKind::Dlc && layer.required_for_playback)
        .map(|layer| stable_dlc_id(&layer.layer_id))
        .collect::<Result<_, _>>()?;
    let mut dlc_selection = trace.dlc_selection;
    for dlc in &mut dlc_selection {
        dlc.required_for_playback = required_dlc_ids.contains(&dlc.dlc_id);
    }

    let required_content = uses
        .into_values()
        .map(|entry| RequiredContent {
            content_key: entry.content_key,
            kind: entry.kind,
            content: entry.content,
            consumers: entry.consumers.into_iter().collect(),
            observation_count: entry.observation_count,
        })
        .collect();
    let mut manifest = ContentManifest {
        schema: CONTENT_MANIFEST_SCHEMA.to_string(),
        product_modset_sha256: product_modset_sha256.to_string(),
        source_game: trace.source_game,
        source_profile: trace.source_profile,
        effective_vfs_manifest_sha256: trace.effective_vfs_manifest_sha256,
        ordered_layers,
        dlc_selection,
        localisation_language: trace.localisation_language,
        required_settings: trace.required_settings,
        recorder_local_settings: trace.recorder_local_settings,
        required_content,
        manifest_digest_sha256: None,
    };
    manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256()?;
    manifest.with_computed_digest()
}

fn validate_launch_binding(
    trace: &RecorderContentTrace,
    launch: &PortableLaunchBinding,
) -> Result<(), String> {
    if trace.source_game.executable_sha256 != launch.executable_sha256 {
        return Err("portable content trace executable hash differs from attached process".into());
    }
    if trace.localisation_language != launch.localisation_language {
        return Err("portable content trace language differs from launch binding".into());
    }
    if trace.required_settings != launch.required_settings {
        return Err("portable content trace settings differ from launch binding".into());
    }
    if trace.recorder_local_settings != launch.recorder_local_settings {
        return Err(
            "portable content trace settings provenance differs from launch binding".into(),
        );
    }

    let trace_mods: Vec<_> = trace
        .ordered_layers
        .iter()
        .filter(|layer| layer.layer_kind == ContentLayerKind::Mod)
        .collect();
    if trace_mods.len() != launch.ordered_mods.len() {
        return Err("portable content trace mod count differs from launch binding".into());
    }
    let mut launch_mods = BTreeMap::new();
    for launch_mod in &launch.ordered_mods {
        if launch_mods
            .insert(launch_mod.layer_id.as_str(), launch_mod)
            .is_some()
        {
            return Err("portable launch binding repeats a mod layer ID".into());
        }
    }
    let mut seen_trace_mods = BTreeSet::new();
    for trace_mod in trace_mods {
        if !seen_trace_mods.insert(trace_mod.layer_id.as_str()) {
            return Err("portable content trace repeats a mod layer ID".into());
        }
        let launch_mod = launch_mods
            .get(trace_mod.layer_id.as_str())
            .ok_or_else(|| {
                format!(
                "portable content trace engine-ordered mod {} is absent from launcher membership",
                trace_mod.layer_id
            )
            })?;
        if trace_mod.layer_id != launch_mod.layer_id
            || trace_mod.descriptor_sha256.as_deref() != Some(launch_mod.descriptor_sha256.as_str())
            || trace_mod.storage_kind != launch_mod.storage_kind
        {
            return Err(format!(
                "portable content trace mod {} differs from launch binding",
                trace_mod.layer_id
            ));
        }
        let expected_container = (launch_mod.storage_kind == LayerStorageKind::ZipArchive)
            .then_some(launch_mod.content_sha256.as_str());
        if trace_mod.source_container_sha256.as_deref() != expected_container {
            return Err(format!(
                "portable content trace archive hash for {} differs from launch binding",
                trace_mod.layer_id
            ));
        }
    }

    let trace_enabled_dlcs: BTreeSet<_> = trace
        .dlc_selection
        .iter()
        .filter(|dlc| dlc.enabled)
        .map(|dlc| dlc.dlc_id.clone())
        .collect();
    if trace_enabled_dlcs != launch.enabled_dlc_ids {
        return Err("portable content trace enabled DLC set differs from launch binding".into());
    }
    let trace_dlc_layers = trace
        .ordered_layers
        .iter()
        .filter(|layer| layer.layer_kind == ContentLayerKind::Dlc)
        .map(|layer| stable_dlc_id(&layer.layer_id).map(|id| (id, layer)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    if trace_dlc_layers.keys().cloned().collect::<BTreeSet<_>>() != trace_enabled_dlcs {
        return Err(
            "portable content trace DLC layer identities differ from enabled DLC selection".into(),
        );
    }
    for (dlc_id, layer) in trace_dlc_layers {
        let expected = launch
            .enabled_dlc_layers
            .get(&dlc_id)
            .ok_or_else(|| format!("portable content trace DLC {dlc_id:?} is not launch-bound"))?;
        if layer.layer_id != expected.layer_id
            || layer.descriptor_sha256.as_deref() != Some(expected.descriptor_sha256.as_str())
            || layer.storage_kind != expected.storage_kind
            || layer.source_container_sha256 != expected.source_container_sha256
        {
            return Err(format!(
                "portable content trace DLC {dlc_id:?} descriptor/storage identity differs from launch binding"
            ));
        }
    }
    let trace_integrated = trace
        .ordered_layers
        .iter()
        .filter(|layer| layer.layer_kind == ContentLayerKind::IntegratedDlc)
        .map(|layer| {
            let comparable = layer.layer_id.replacen("integrated_dlc/", "dlc/", 1);
            stable_dlc_id(&comparable).map(|id| (id, layer))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    if trace_integrated.len() != launch.integrated_dlc_layers.len() {
        return Err("portable content trace integrated DLC roster differs from install".into());
    }
    for (dlc_id, layer) in trace_integrated {
        let expected = launch
            .integrated_dlc_layers
            .get(&dlc_id)
            .ok_or_else(|| format!("integrated DLC {dlc_id:?} is not install-bound"))?;
        if layer.layer_id != expected.layer_id
            || layer.descriptor_sha256.as_deref() != Some(expected.descriptor_sha256.as_str())
            || layer.storage_kind != expected.storage_kind
            || layer.source_container_sha256 != expected.source_container_sha256
        {
            return Err(format!(
                "portable content trace integrated DLC {dlc_id:?} identity differs from install"
            ));
        }
    }
    Ok(())
}

pub fn trace_sha256(trace: &RecorderContentTrace) -> Result<String, String> {
    let bytes = serde_json::to_vec(trace).map_err(|error| error.to_string())?;
    Ok(to_hex(&Sha256::digest(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modset::{DisplayVariantSettings, DlcDescriptorEvidence, ModsetReport, ResolvedMod};
    use portable_content_manifest::LayerStorageKind;
    use std::path::PathBuf;

    fn sha(byte: char) -> String {
        byte.to_string().repeat(64)
    }

    fn engine_runtime_artifact_declaration() -> serde_json::Value {
        serde_json::json!({
            "schema": "extractor.engine_runtime_artifacts.v1",
            "class": "engine_written_install_root_sink",
            "authority_sha256": Sha256::hex(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../game_defs/engine_runtime_artifacts.json"
                ))
                .as_bytes()
            ),
            "observed_in_base_layer": [],
        })
    }

    fn namespace_content(path: &str, marker: char) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: sha('a'),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: sha(marker),
            payload_sha256: sha(marker),
            payload_bytes: 17,
        }
    }

    #[test]
    fn complete_authored_namespaces_include_every_viewer_language() {
        let winners = [
            (
                "interface/base.gui".to_string(),
                namespace_content("interface/base.gui", '1'),
            ),
            (
                "interface/base.gfx".to_string(),
                namespace_content("interface/base.gfx", '2'),
            ),
            (
                "localisation/french/ui_l_french.yml".to_string(),
                namespace_content("localisation/french/ui_l_french.yml", '3'),
            ),
            (
                "localisation/english/ui_l_english.yml".to_string(),
                namespace_content("localisation/english/ui_l_english.yml", '4'),
            ),
            (
                "common/technologies/infantry.txt".to_string(),
                namespace_content("common/technologies/infantry.txt", '5'),
            ),
        ]
        .into_iter()
        .collect::<BTreeMap<_, _>>();
        let mut observations = Vec::new();
        append_complete_authored_namespace_observations(&mut observations, &winners).unwrap();
        assert_eq!(observations.len(), 4);
        assert!(observations.iter().any(|row| {
            row.consumer == AUTHORED_GUI_NAMESPACE_CONSUMER
                && row.content_key == "interface/base.gui"
        }));
        assert!(observations.iter().any(|row| {
            row.consumer == AUTHORED_GFX_NAMESPACE_CONSUMER
                && row.content_key == "interface/base.gfx"
        }));
        assert!(observations.iter().any(|row| {
            row.consumer == AUTHORED_LOCALISATION_NAMESPACES_CONSUMER
                && row.content_key == "localisation/french/ui_l_french.yml"
        }));
        assert!(observations.iter().any(|row| {
            row.consumer == AUTHORED_LOCALISATION_NAMESPACES_CONSUMER
                && row.content_key == "localisation/english/ui_l_english.yml"
        }));
    }

    #[test]
    fn production_refuses_any_missing_authored_namespace() {
        let winners = [
            (
                "interface/base.gui".to_string(),
                namespace_content("interface/base.gui", '1'),
            ),
            (
                "interface/base.gfx".to_string(),
                namespace_content("interface/base.gfx", '2'),
            ),
        ]
        .into_iter()
        .collect::<BTreeMap<_, _>>();
        let error =
            append_complete_authored_namespace_observations(&mut Vec::new(), &winners).unwrap_err();
        assert!(error.contains("GUI/GFX/localisation"));
    }

    #[test]
    fn source_inventory_uses_filesystem_case_key_for_cross_layer_winners() {
        let inventory = serde_json::json!({
            "rows": [
                {
                    "namespace_ordinal": 0,
                    "relpath": "interface/Ä.gui",
                    "sha256": sha('1'),
                    "bytes": 11
                },
                {
                    "namespace_ordinal": 1,
                    "relpath": "interface/ä.gui",
                    "sha256": sha('2'),
                    "bytes": 22
                }
            ]
        });
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
                layer_id: "mod/unicode.mod".into(),
                display_name: "unicode".into(),
                descriptor_sha256: Some(sha('3')),
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: false,
            },
        ];
        let winners =
            source_inventory_winners(&inventory, &layers, &[Some(0), Some(1)], &sha('a')).unwrap();
        assert_eq!(winners.len(), 1);
        let winner = winners.values().next().unwrap();
        assert_eq!(winner.normalized_game_relative_path, "interface/ä.gui");
        assert_eq!(winner.winning_layer_ordinal, 1);
        assert_eq!(winner.payload_sha256, sha('2'));
    }

    /// The attach-time census walks the effective VFS and records what it
    /// finds as this recording's content identity. Three of the files in that
    /// walk are the ENGINE'S OWN write sinks, which it opens by bare name with
    /// raw stream I/O and never resolves through the VFS. Recording them binds
    /// replay to the game's scratch: `console_history.txt` is appended to on
    /// every accepted console command, so a later session refuses the recording
    /// on a payload mismatch over a file no replay reads. Owner ruling
    /// 2026-08-15 put this class out of the product-modset key for the same
    /// reason; this is the census catching up.
    ///
    /// The census keeps counting them in `files` (the walk saw them) and names
    /// each one it dropped, so the difference between what was seen and what
    /// was recorded is never unexplained.
    ///
    /// kills: removing the exclusion from `census_effective_vfs_corpus`;
    /// dropping the rows without listing them; excluding a nested file of the
    /// same name; or excluding a mod layer's own file of that name.
    #[test]
    fn the_attach_census_excludes_the_engine_written_install_root_sinks_by_name() {
        use portable_content_manifest::effective_vfs::{EffectiveVfs, EffectiveVfsLayerMount};

        let root = std::env::temp_dir().join(format!(
            "hoi4_attach_census_sinks_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _cleanup = TempFixtureCleanup(root.clone());
        let write = |layer: &str, relative: &str, bytes: &[u8]| {
            let path = root.join(layer).join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes).unwrap();
        };
        write("game", "common/units/infantry.txt", b"content");
        // The engine's own sinks, at the install root.
        write("game", "console_history.txt", b"tag fra\n");
        write("game", "imgui.ini", b"[Window][Debug]\n");
        write("game", "imgui_log.txt", b"log\n");
        // Same NAME, one directory down: content the VFS really resolves.
        write("game", "common/console_history.txt", b"authored content");
        // Same name at a MOD's own root: that mod's content.
        write("mod", "console_history.txt", b"mod payload");

        let vfs = EffectiveVfs::new(
            "a".repeat(64),
            vec![
                EffectiveVfsLayerMount {
                    ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    storage_kind: LayerStorageKind::Directory,
                    local_source: root.join("game"),
                    source_container_sha256: None,
                    replace_paths: Vec::new(),
                },
                EffectiveVfsLayerMount {
                    ordinal: 1,
                    layer_kind: ContentLayerKind::Mod,
                    layer_id: "mod/active.mod".into(),
                    storage_kind: LayerStorageKind::Directory,
                    local_source: root.join("mod"),
                    source_container_sha256: None,
                    replace_paths: Vec::new(),
                },
            ],
        )
        .unwrap();

        let (rows, stats) = census_effective_vfs_corpus(&vfs).unwrap();
        let recorded: Vec<&str> = rows
            .iter()
            .map(|row| row.content.normalized_game_relative_path.as_str())
            .collect();
        // The mod wins `console_history.txt`, so it stays as that mod's
        // content; only the base-layer sinks the mod did not shadow are
        // dropped.
        assert_eq!(
            recorded,
            vec![
                "common/console_history.txt",
                "common/units/infantry.txt",
                "console_history.txt",
            ]
        );
        assert_eq!(
            rows.iter()
                .find(|row| row.content.normalized_game_relative_path == "console_history.txt")
                .unwrap()
                .content
                .layer_kind,
            ContentLayerKind::Mod
        );
        assert_eq!(
            stats.excluded_engine_written_sinks,
            vec!["imgui.ini".to_string(), "imgui_log.txt".to_string()]
        );
        // `files` counts the walk; `rows` counts what was kept. The gap is
        // exactly the named exclusions.
        assert_eq!(
            stats.files as usize,
            rows.len() + stats.excluded_engine_written_sinks.len()
        );
    }

    struct TempFixtureCleanup(PathBuf);

    impl Drop for TempFixtureCleanup {
        fn drop(&mut self) {
            if self.0.starts_with(std::env::temp_dir()) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }

    fn sha_json_value(value: &serde_json::Value) -> String {
        Sha256::hex(&serde_json::to_vec(value).unwrap())
    }

    fn finalize_source_inventory(mut inventory: serde_json::Value) -> serde_json::Value {
        let rows = inventory.get("rows").unwrap().as_array().unwrap().clone();
        for namespace in inventory
            .get_mut("namespaces")
            .unwrap()
            .as_array_mut()
            .unwrap()
        {
            let ordinal = namespace.get("ordinal").unwrap().as_u64().unwrap();
            let content_rows = rows
                .iter()
                .filter(|row| row.get("namespace_ordinal").unwrap().as_u64() == Some(ordinal))
                .map(|row| {
                    serde_json::json!({
                        "relpath": row.get("relpath").unwrap(),
                        "bytes": row.get("bytes").unwrap(),
                        "sha256": row.get("sha256").unwrap(),
                    })
                })
                .collect::<Vec<_>>();
            namespace["file_count"] = serde_json::json!(content_rows.len());
            namespace["content_sha256"] =
                serde_json::json!(sha_json_value(&serde_json::Value::Array(content_rows),));
        }
        let namespace_sha256 = sha_json_value(inventory.get("namespaces").unwrap());
        let rows_sha256 = sha_json_value(inventory.get("rows").unwrap());
        inventory["namespace_sha256"] = serde_json::json!(namespace_sha256);
        inventory["rows_sha256"] = serde_json::json!(rows_sha256);
        let core = serde_json::json!({
            "schema": inventory.get("schema").unwrap(),
            "identity": inventory.get("identity").unwrap(),
            "namespaces": inventory.get("namespaces").unwrap(),
            "rows": inventory.get("rows").unwrap(),
            "namespace_sha256": inventory.get("namespace_sha256").unwrap(),
            "rows_sha256": inventory.get("rows_sha256").unwrap(),
        });
        inventory["inventory_sha256"] = serde_json::json!(sha_json_value(&core));
        inventory
    }

    fn base_trace() -> (RecorderContentTrace, PortableLaunchBinding) {
        let vfs = sha('a');
        let layers = vec![
            RecordedContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                display_name: "the game base game".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: vec![],
                required_for_playback: false,
            },
            RecordedContentLayer {
                ordinal: 1,
                layer_kind: ContentLayerKind::Dlc,
                layer_id: "dlc/dlc_used/dlc_used.dlc".into(),
                display_name: "Used DLC".into(),
                descriptor_sha256: Some(sha('b')),
                source_container_sha256: Some(sha('c')),
                storage_kind: LayerStorageKind::ZipArchive,
                replace_paths: vec!["gfx/models".into()],
                required_for_playback: true,
            },
            RecordedContentLayer {
                ordinal: 2,
                layer_kind: ContentLayerKind::Dlc,
                layer_id: "dlc/dlc_unused/dlc_unused.dlc".into(),
                display_name: "Unused DLC".into(),
                descriptor_sha256: Some(sha('d')),
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: vec![],
                required_for_playback: false,
            },
            RecordedContentLayer {
                ordinal: 3,
                layer_kind: ContentLayerKind::Mod,
                layer_id: "mod/archive.mod".into(),
                display_name: "Archive override".into(),
                descriptor_sha256: Some(sha('e')),
                source_container_sha256: Some(sha('f')),
                storage_kind: LayerStorageKind::ZipArchive,
                replace_paths: vec!["common/defines".into()],
                required_for_playback: true,
            },
        ];
        let observations = vec![
            ContentUseObservation {
                content_key: "tank-model".into(),
                kind: RequiredContentKind::Mesh,
                content: UsedContentKey {
                    effective_vfs_manifest_sha256: vfs.clone(),
                    normalized_game_relative_path: "gfx/models/tank.mesh".into(),
                    winning_layer_ordinal: 1,
                    layer_kind: ContentLayerKind::Dlc,
                    layer_id: "dlc/dlc_used/dlc_used.dlc".into(),
                    source_sha256: sha('c'),
                    payload_sha256: sha('1'),
                    payload_bytes: 123,
                },
                consumer: "map_renderer".into(),
            },
            ContentUseObservation {
                content_key: "defines".into(),
                kind: RequiredContentKind::Definition,
                content: UsedContentKey {
                    effective_vfs_manifest_sha256: vfs.clone(),
                    normalized_game_relative_path: "common/defines/00_defines.lua".into(),
                    winning_layer_ordinal: 3,
                    layer_kind: ContentLayerKind::Mod,
                    layer_id: "mod/archive.mod".into(),
                    source_sha256: sha('f'),
                    payload_sha256: sha('2'),
                    payload_bytes: 456,
                },
                consumer: "script_interpreter".into(),
            },
        ];
        let required_settings = BTreeMap::new();
        let recorder_local_settings = BTreeMap::from([
            ("gui_scale_f32_bits".into(), "1065353216".into()),
            ("display_mode".into(), "borderless".into()),
        ]);
        let trace = RecorderContentTrace {
            schema: TRACE_SCHEMA.into(),
            source_game: SourceGameIdentity {
                version: "1.19.2.0".into(),
                build: "a729".into(),
                executable_sha256: sha('9'),
                build_profile: "hoi4-1.19.2.0-a729".into(),
            },
            source_profile: SourceProfileIdentity {
                profile_id: "profile:fixture".into(),
                playset_id: "playset:fixture".into(),
                selection_sha256: sha('8'),
            },
            effective_vfs_manifest_sha256: vfs,
            ordered_layers: layers,
            dlc_selection: vec![
                DlcSelection {
                    dlc_id: "dlc_used".into(),
                    enabled: true,
                    required_for_playback: true,
                },
                DlcSelection {
                    dlc_id: "dlc_unused".into(),
                    enabled: true,
                    required_for_playback: false,
                },
            ],
            localisation_language: "english".into(),
            required_settings: required_settings.clone(),
            recorder_local_settings: recorder_local_settings.clone(),
            observations,
        };
        let launch = PortableLaunchBinding {
            executable_sha256: sha('9'),
            ordered_mods: vec![BoundMod {
                layer_id: "mod/archive.mod".into(),
                descriptor_sha256: sha('e'),
                content_sha256: sha('f'),
                storage_kind: LayerStorageKind::ZipArchive,
            }],
            enabled_dlc_ids: BTreeSet::from(["dlc_used".into(), "dlc_unused".into()]),
            enabled_dlc_layers: BTreeMap::from([
                (
                    "dlc_used".into(),
                    BoundDlcLayer {
                        layer_id: "dlc/dlc_used/dlc_used.dlc".into(),
                        descriptor_sha256: sha('b'),
                        source_container_sha256: Some(sha('c')),
                        storage_kind: LayerStorageKind::ZipArchive,
                    },
                ),
                (
                    "dlc_unused".into(),
                    BoundDlcLayer {
                        layer_id: "dlc/dlc_unused/dlc_unused.dlc".into(),
                        descriptor_sha256: sha('d'),
                        source_container_sha256: None,
                        storage_kind: LayerStorageKind::Directory,
                    },
                ),
            ]),
            integrated_dlc_layers: BTreeMap::new(),
            localisation_language: "english".into(),
            required_settings,
            recorder_local_settings,
        };
        (trace, launch)
    }

    #[test]
    fn repeated_identical_use_is_deterministically_merged() {
        let (mut trace, launch) = base_trace();
        let mut repeated = trace.observations[0].clone();
        repeated.consumer = "ui_renderer".into();
        trace.observations.push(repeated);
        let first = build(trace.clone(), &launch, &sha('9')).unwrap();
        let second = build(trace, &launch, &sha('9')).unwrap();
        assert_eq!(first.encode_json().unwrap(), second.encode_json().unwrap());
        let row = first
            .required_content
            .iter()
            .find(|row| row.content_key == "tank-model")
            .unwrap();
        assert_eq!(row.observation_count, 2);
        assert_eq!(row.consumers, vec!["map_renderer", "ui_renderer"]);
        assert!(first.dlc_selection[0].required_for_playback);
        assert!(!first.dlc_selection[1].required_for_playback);
    }

    #[test]
    fn launch_membership_accepts_distinct_authenticated_engine_mod_order() {
        let (mut trace, mut launch) = base_trace();
        trace.ordered_layers.insert(
            3,
            RecordedContentLayer {
                ordinal: 3,
                layer_kind: ContentLayerKind::Mod,
                layer_id: "mod/a-first.mod".into(),
                display_name: "Engine first".into(),
                descriptor_sha256: Some(sha('7')),
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: false,
            },
        );
        for (ordinal, layer) in trace.ordered_layers.iter_mut().enumerate() {
            layer.ordinal = ordinal as u32;
        }
        trace.observations[1].content.winning_layer_ordinal = 4;
        launch.ordered_mods.push(BoundMod {
            layer_id: "mod/a-first.mod".into(),
            descriptor_sha256: sha('7'),
            content_sha256: sha('8'),
            storage_kind: LayerStorageKind::Directory,
        });
        validate_launch_binding(&trace, &launch)
            .expect("launcher membership must not override authenticated engine order");

        launch.ordered_mods[1] = launch.ordered_mods[0].clone();
        assert!(validate_launch_binding(&trace, &launch)
            .unwrap_err()
            .contains("repeats a mod layer ID"));
    }

    #[test]
    fn recorder_aggregation_writes_standard_trace_atomically() {
        let (mut trace, launch) = base_trace();
        let original = trace.observations.clone();
        trace.observations.clear();
        for observation in original {
            trace.record_observation(observation).unwrap();
        }
        trace
            .record_observation(trace.observations[0].clone())
            .unwrap();
        let dir = std::env::temp_dir().join(format!(
            "hoi4_portable_trace_writer_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join(DEFAULT_TRACE_FILE);
        trace.write_atomic(&path).unwrap();
        assert!(path.is_file());
        assert!(!dir
            .join(format!(
                ".{DEFAULT_TRACE_FILE}.{}.0.tmp",
                std::process::id()
            ))
            .exists());
        assert!(trace
            .write_atomic(&path)
            .unwrap_err()
            .contains("already exists"));
        let (manifest, _) = load_and_build(&path, &launch, &sha('9')).unwrap();
        assert_eq!(
            manifest
                .required_content
                .iter()
                .find(|row| row.content_key == "tank-model")
                .unwrap()
                .observation_count,
            2
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recorder_aggregation_rejects_foreign_vfs_key_before_write() {
        let (mut trace, _) = base_trace();
        let mut observation = trace.observations[0].clone();
        trace.observations.clear();
        observation.content.effective_vfs_manifest_sha256 = sha('0');
        assert!(trace
            .record_observation(observation)
            .unwrap_err()
            .contains("different effective VFS"));
    }

    #[test]
    fn hud_role_snapshot_adds_only_exact_mod_winner_and_omits_unresolved_surface() {
        use hud_role_observer_live_lane::{
            HudContentObservation, HudRoleSnapshot, HudSurface, OmittedHudSurface,
        };

        let (mut trace, launch) = base_trace();
        let content = UsedContentKey {
            effective_vfs_manifest_sha256: sha('a'),
            normalized_game_relative_path: "interface/modded_topbar.gui".into(),
            winning_layer_ordinal: 3,
            layer_kind: ContentLayerKind::Mod,
            layer_id: "mod/archive.mod".into(),
            source_sha256: sha('f'),
            payload_sha256: sha('6'),
            payload_bytes: 99,
        };
        let snapshot = HudRoleSnapshot {
            schema: "extractor.hud-role-snapshot.v1".into(),
            authority_manifest_sha256: sha('a'),
            activated_surfaces: vec![HudSurface::Topbar, HudSurface::TheatreSelector],
            observations: vec![HudContentObservation {
                content_key: content.normalized_game_relative_path.clone(),
                kind: RequiredContentKind::Catalog,
                content: content.clone(),
                consumer: format!("hud_role/v1/topbar/gui/topbar/{}", sha('7')),
            }],
            omitted_surfaces: vec![OmittedHudSurface {
                surface: HudSurface::TheatreSelector,
                reason: "unresolved authored roles".into(),
            }],
        };
        append_hud_snapshot_observations(
            &mut trace.observations,
            &snapshot,
            &trace.effective_vfs_manifest_sha256,
            &BTreeMap::from([(content.normalized_game_relative_path.clone(), content)]),
        )
        .unwrap();
        let manifest = build(trace, &launch, &sha('9')).unwrap();
        let row = manifest
            .required_content
            .iter()
            .find(|row| row.content.normalized_game_relative_path == "interface/modded_topbar.gui")
            .unwrap();
        assert_eq!(row.kind, RequiredContentKind::GuiDefinition);
        assert_eq!(row.content.layer_id, "mod/archive.mod");
        assert_eq!(row.consumers.len(), 1);
        assert!(row.consumers[0].starts_with("hud_role/v1/topbar/"));
        assert!(!manifest.required_content.iter().any(|row| row
            .consumers
            .iter()
            .any(|consumer| consumer.contains("theatre_selector"))));
    }

    #[test]
    fn hud_role_snapshot_foreign_vfs_is_rejected() {
        let (mut trace, _) = base_trace();
        let snapshot = HudRoleSnapshot {
            schema: "extractor.hud-role-snapshot.v1".into(),
            authority_manifest_sha256: sha('0'),
            activated_surfaces: vec![hud_role_observer_live_lane::HudSurface::Topbar],
            observations: vec![],
            omitted_surfaces: vec![hud_role_observer_live_lane::OmittedHudSurface {
                surface: hud_role_observer_live_lane::HudSurface::Topbar,
                reason: "unresolved authored roles".into(),
            }],
        };
        assert!(append_hud_snapshot_observations(
            &mut trace.observations,
            &snapshot,
            &trace.effective_vfs_manifest_sha256,
            &BTreeMap::new(),
        )
        .unwrap_err()
        .contains("different effective VFS"));
    }

    #[test]
    fn m02_used_content_is_admitted_from_the_exact_inventory_winner() {
        let (mut trace, launch) = base_trace();
        let mut content = trace.observations[0].content.clone();
        content.normalized_game_relative_path = "gfx/models/m02.mesh".into();
        let winners = BTreeMap::from([(
            content.normalized_game_relative_path.clone(),
            content.clone(),
        )]);
        append_m02_content_observations(
            &mut trace.observations,
            std::slice::from_ref(&content),
            &trace.effective_vfs_manifest_sha256,
            &winners,
        )
        .unwrap();
        let manifest = build(trace, &launch, &sha('9')).unwrap();
        let row = manifest
            .required_content
            .iter()
            .find(|row| row.content == content)
            .unwrap();
        assert!(row
            .consumers
            .iter()
            .any(|consumer| consumer == M02_CONTENT_CONSUMER));

        let mut drifted = content.clone();
        drifted.payload_sha256 = sha('9');
        assert!(append_m02_content_observations(
            &mut Vec::new(),
            &[drifted],
            &content.effective_vfs_manifest_sha256,
            &winners,
        )
        .unwrap_err()
        .contains("differs from the effective source-inventory winner"));
    }

    #[test]
    fn normal_runtime_stream_scanner_carries_d79_and_rejects_legacy_fixed_role_map_snapshot() {
        fn push_record(
            out: &mut Vec<u8>,
            sequence: u64,
            game_hour_raw: i32,
            domain_id: u16,
            payload: &[u8],
        ) {
            out.extend_from_slice(compact_contract::COMPACT_RECORD_MAGIC);
            out.extend_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
            out.extend_from_slice(
                &(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes(),
            );
            out.extend_from_slice(&sequence.to_le_bytes());
            out.extend_from_slice(&game_hour_raw.to_le_bytes());
            out.extend_from_slice(&domain_id.to_le_bytes());
            out.extend_from_slice(&compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA.to_le_bytes());
            out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(payload);
        }

        let d79 = hud_role_observer_live_lane::encode_activation_payload(
            &hud_role_observer_live_lane::HudSurfaceActivationFrame {
                frame_hour: 123,
                status_flags: 0,
                observation_sequence: 9,
                active_surfaces: vec![
                    hud_role_observer_live_lane::HudSurfaceActivation {
                        surface: hud_role_observer_live_lane::HudSurface::Topbar,
                        runtime_root_name: "topbar_root".into(),
                    },
                    hud_role_observer_live_lane::HudSurfaceActivation {
                        surface: hud_role_observer_live_lane::HudSurface::Minimap,
                        runtime_root_name: "minimap_root".into(),
                    },
                    hud_role_observer_live_lane::HudSurfaceActivation {
                        surface: hud_role_observer_live_lane::HudSurface::Mapmodes,
                        runtime_root_name: "mapmodes_root".into(),
                    },
                    hud_role_observer_live_lane::HudSurfaceActivation {
                        surface: hud_role_observer_live_lane::HudSurface::TheatreSelector,
                        runtime_root_name: "theatre_root".into(),
                    },
                ],
            },
        )
        .unwrap();
        let mut bytes = Vec::new();
        push_record(
            &mut bytes,
            1,
            123,
            hud_role_observer_live_lane::DOMAIN_ID,
            &d79,
        );
        let dir = std::env::temp_dir().join(format!(
            "hoi4_runtime_content_stream_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("capture.h4cr");
        fs::write(&path, &bytes).unwrap();
        let runtime = scan_runtime_content_snapshots(
            &path,
            compact_contract::stream_validator::StreamIdentityMode::Legacy,
            None,
        )
        .unwrap();
        assert_eq!(runtime.hud_activations.len(), 1);
        assert_eq!(runtime.hud_activations[0].observation_sequence, 9);

        let mut conflicting_frame =
            hud_role_observer_live_lane::decode_activation_payload(&d79).unwrap();
        conflicting_frame.frame_hour = 124;
        conflicting_frame.observation_sequence = 10;
        conflicting_frame.active_surfaces[0].runtime_root_name = "changed_topbar_root".into();
        let conflicting_d79 =
            hud_role_observer_live_lane::encode_activation_payload(&conflicting_frame).unwrap();
        let mut conflicting = bytes.clone();
        push_record(
            &mut conflicting,
            2,
            124,
            hud_role_observer_live_lane::DOMAIN_ID,
            &conflicting_d79,
        );
        let conflicting_path = dir.join("conflicting.h4cr");
        fs::write(&conflicting_path, conflicting).unwrap();
        assert!(scan_runtime_content_snapshots(
            &conflicting_path,
            compact_contract::stream_validator::StreamIdentityMode::Legacy,
            None,
        )
        .unwrap_err()
        .contains("runtime root changed within recording"));

        let mut legacy = bytes;
        push_record(
            &mut legacy,
            2,
            124,
            LEGACY_FIXED_ROLE_MAP_SNAPSHOT_DOMAIN_ID,
            b"legacy-fixed-role-map-snapshot",
        );
        let legacy_path = dir.join("legacy.h4cr");
        fs::write(&legacy_path, legacy).unwrap();
        assert!(scan_runtime_content_snapshots(
            &legacy_path,
            compact_contract::stream_validator::StreamIdentityMode::Legacy,
            None,
        )
        .unwrap_err()
        .contains("legacy fixed-role"));
        portable_test_fs_retry::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn normal_runtime_finalizer_emits_modded_hud_roles_and_is_relocation_stable_twice() {
        fn gui(name: &str) -> String {
            format!("guiTypes = {{ containerWindowType = {{ name = \"{name}\" }} }}")
        }
        fn write_fixture(root: &Path) -> (PathBuf, PathBuf, Vec<(u32, String, Vec<u8>)>) {
            let game = root.join("game");
            let user = root.join("profile");
            fs::create_dir_all(game.join("localisation")).unwrap();
            let files = [
                ("interface/topbar.gui", gui("topbar")),
                ("interface/base.gfx", "spriteTypes = { }".into()),
                (
                    "localisation/english/ui_l_english.yml",
                    "l_english:\n XUI01_FIXTURE:0 \"Fixture\"\n".into(),
                ),
                ("interface/minimap.gui", gui("mini_map_interface")),
                (
                    "interface/mapmodes_interface.gui",
                    gui("MapmodesInterface_Ingame"),
                ),
                (
                    "interface/theatreselector.gui",
                    gui("theatre_selection_window"),
                ),
                ("interface/tank_designer.gui", gui("tank_designer_view")),
                ("interface/plane_designer.gui", gui("plane_designer_view")),
                ("common/base.txt", "base-static".into()),
            ];
            let mut rows = Vec::new();
            for (rel, text) in files {
                let path = game.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, text.as_bytes()).unwrap();
                rows.push((0, rel.to_string(), text.into_bytes()));
            }
            let descriptor = user.join("mod/portable.mod");
            fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
            fs::write(&descriptor, "name=\"Portable HUD\"\npath=\"mod/content\"\n").unwrap();
            fs::write(
                user.join("dlc_load.json"),
                r#"{"enabled_mods":["mod/portable.mod"],"disabled_dlcs":[]}"#,
            )
            .unwrap();
            let mod_text = gui("topbar");
            let mod_path = user.join("mod/content/interface/topbar.gui");
            fs::create_dir_all(mod_path.parent().unwrap()).unwrap();
            fs::write(&mod_path, mod_text.as_bytes()).unwrap();
            rows.push((1, "interface/topbar.gui".into(), mod_text.into_bytes()));
            (game, user, rows)
        }
        fn push_d79(out: &mut Vec<u8>, sequence: u64, game_hour_raw: i32, payload: &[u8]) {
            out.extend_from_slice(compact_contract::COMPACT_RECORD_MAGIC);
            out.extend_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
            out.extend_from_slice(
                &(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes(),
            );
            out.extend_from_slice(&sequence.to_le_bytes());
            out.extend_from_slice(&game_hour_raw.to_le_bytes());
            out.extend_from_slice(&hud_role_observer_live_lane::DOMAIN_ID.to_le_bytes());
            out.extend_from_slice(&compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA.to_le_bytes());
            out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(payload);
        }

        let root = std::env::temp_dir().join(format!(
            "hoi4_runtime_hud_finalizer_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _cleanup = TempFixtureCleanup(root.clone());
        let first = root.join("first-machine");
        let second = root.join("relocated-machine");
        let (game_a, user_a, rows) = write_fixture(&first);
        let (game_b, user_b, _) = write_fixture(&second);
        let bundle = root.join("bundle");
        fs::create_dir_all(bundle.join("proofs")).unwrap();
        let inventory_rows = rows
            .iter()
            .map(|(ordinal, rel, bytes)| {
                serde_json::json!({
                    "namespace_ordinal": ordinal,
                    "layer_kind": if *ordinal == 0 { "vanilla" } else { "mod" },
                    "layer_id": if *ordinal == 0 { "vanilla" } else { "mod/portable.mod" },
                    "relpath": rel,
                    "bytes": bytes.len(),
                    "sha256": Sha256::hex(bytes),
                    "storage_kind": "directory",
                    "source_container_sha256": serde_json::Value::Null,
                    "source_byte_offset": 0,
                    "source_byte_count": bytes.len(),
                    "source_range_sha256": Sha256::hex(bytes),
                })
            })
            .collect::<Vec<_>>();
        let inventory = finalize_source_inventory(serde_json::json!({
            "schema": "extractor.live_binding_source_inventory.v3",
            "identity": {"profile_id":"profile:hud","playset_id":"playset:hud"},
            "namespaces": [
                {"ordinal":0,"kind":"vanilla","binding":"base","id":"vanilla","label":"vanilla","replace_paths":[],"storage_kind":"directory","source_container_sha256":null,"source_container_bytes":null,"file_count":0,"content_sha256":sha('1'),"binding_content_sha256":sha('1')},
                {"ordinal":1,"kind":"mod","binding":"active_mod","id":"mod/portable.mod","label":"Portable HUD","replace_paths":[],"storage_kind":"directory","source_container_sha256":null,"source_container_bytes":null,"file_count":0,"content_sha256":sha('2'),"binding_content_sha256":sha('2')}
            ],
            "rows": inventory_rows,
            "namespace_sha256": sha('3'),
            "rows_sha256": sha('4'),
            "inventory_sha256": sha('a')
        }));
        let inventory_sha256 = inventory
            .get("inventory_sha256")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        let base_static = rows
            .iter()
            .find(|(ordinal, rel, _)| *ordinal == 0 && rel == "common/base.txt")
            .unwrap();
        let join = serde_json::json!({
            "schema":"extractor.static_catalog_source_join.v1",
            "passed":true,
            "inventory_sha256":inventory_sha256,
            "roles":[],
            "bindings":[{
                "artifact_role":"fixture_static",
                "relpath":"common/base.txt",
                "bytes":base_static.2.len(),
                "sha256":Sha256::hex(&base_static.2),
                "namespace_ordinal":0,
                "layer_kind":"vanilla",
                "layer_id":"vanilla"
            }],
            "bindings_sha256":sha('5')
        });
        let report = serde_json::json!({
            "schema":"extractor.live_binding_resolution.v1",
            "passed":true,
            "engine_runtime_artifacts": engine_runtime_artifact_declaration(),
            "process":{"exe_sha256":sha('9')},
            "runtime":{"version":"1.19.2.0.a729"},
            "binding_identity":{"profile_id":"profile:hud","playset_id":"playset:hud"}
        });
        let inventory_path = bundle.join("source_inventory.json");
        let join_path = bundle.join("proofs/catalog_source_join.json");
        let report_path = bundle.join("resolution_report.json");
        fs::write(&inventory_path, serde_json::to_vec(&inventory).unwrap()).unwrap();
        fs::write(&join_path, serde_json::to_vec(&join).unwrap()).unwrap();
        fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
        let mut frame = hud_role_observer_live_lane::HudSurfaceActivationFrame {
            frame_hour: 10,
            status_flags: 0,
            observation_sequence: 1,
            active_surfaces: vec![
                hud_role_observer_live_lane::HudSurfaceActivation {
                    surface: hud_role_observer_live_lane::HudSurface::Topbar,
                    runtime_root_name: "topbar".into(),
                },
                hud_role_observer_live_lane::HudSurfaceActivation {
                    surface: hud_role_observer_live_lane::HudSurface::Minimap,
                    runtime_root_name: "mini_map_interface".into(),
                },
                hud_role_observer_live_lane::HudSurfaceActivation {
                    surface: hud_role_observer_live_lane::HudSurface::Mapmodes,
                    runtime_root_name: "MapmodesInterface_Ingame".into(),
                },
                hud_role_observer_live_lane::HudSurfaceActivation {
                    surface: hud_role_observer_live_lane::HudSurface::TheatreSelector,
                    runtime_root_name: "theatre_selection_window".into(),
                },
                hud_role_observer_live_lane::HudSurfaceActivation {
                    surface: hud_role_observer_live_lane::HudSurface::DesignerTank,
                    runtime_root_name: "tank_designer_view".into(),
                },
            ],
        };
        let mut stream = Vec::new();
        push_d79(
            &mut stream,
            1,
            10,
            &hud_role_observer_live_lane::encode_activation_payload(&frame).unwrap(),
        );
        frame.frame_hour = 11;
        frame.observation_sequence = 2;
        frame.active_surfaces.pop();
        frame
            .active_surfaces
            .push(hud_role_observer_live_lane::HudSurfaceActivation {
                surface: hud_role_observer_live_lane::HudSurface::DesignerPlane,
                runtime_root_name: "plane_designer_view".into(),
            });
        push_d79(
            &mut stream,
            2,
            11,
            &hud_role_observer_live_lane::encode_activation_payload(&frame).unwrap(),
        );
        frame.frame_hour = 12;
        frame.observation_sequence = 3;
        frame.active_surfaces.pop();
        push_d79(
            &mut stream,
            3,
            12,
            &hud_role_observer_live_lane::encode_activation_payload(&frame).unwrap(),
        );
        let stream_path = bundle.join("capture.h4cr");
        fs::write(&stream_path, stream).unwrap();
        let launch = PortableLaunchBinding {
            executable_sha256: sha('9'),
            ordered_mods: vec![BoundMod {
                layer_id: "mod/portable.mod".into(),
                descriptor_sha256: semantic_descriptor_sha256(
                    "name=\"Portable HUD\"\npath=\"mod/content\"\n",
                ),
                content_sha256: sha('2'),
                storage_kind: LayerStorageKind::Directory,
            }],
            enabled_dlc_ids: BTreeSet::new(),
            enabled_dlc_layers: BTreeMap::new(),
            integrated_dlc_layers: BTreeMap::new(),
            localisation_language: "english".into(),
            required_settings: BTreeMap::new(),
            recorder_local_settings: BTreeMap::from([("display_mode".into(), "borderless".into())]),
        };
        let build_at = |game, user| {
            build_from_finalized_static_bundle_with_runtime_stream(
                &inventory_path,
                &join_path,
                &report_path,
                &launch,
                &sha('9'),
                &RuntimeContentRoots {
                    install_root: game,
                    user_dir: user,
                },
                &stream_path,
                compact_contract::stream_validator::StreamIdentityMode::Legacy,
            )
            .unwrap()
        };
        let first_manifest = build_at(game_a.clone(), user_a.clone());
        let repeated_manifest = build_at(game_a, user_a);
        let relocated_manifest = build_at(game_b.clone(), user_b.clone());
        assert_eq!(first_manifest, repeated_manifest);
        assert_eq!(first_manifest, relocated_manifest);
        let hud = first_manifest
            .required_content
            .iter()
            .filter(|row| {
                row.consumers
                    .iter()
                    .any(|consumer| consumer.starts_with("hud_role/v1/"))
            })
            .collect::<Vec<_>>();
        assert_eq!(hud.len(), 6);
        assert!(hud.iter().any(|row| {
            row.content.normalized_game_relative_path == "interface/topbar.gui"
                && row.content.layer_kind == ContentLayerKind::Mod
                && row.content.layer_id == "mod/portable.mod"
        }));
        for surface in ["topbar", "minimap", "mapmodes", "theatre_selector"] {
            assert!(hud.iter().any(|row| row
                .consumers
                .iter()
                .any(|consumer| consumer.starts_with(&format!("hud_role/v1/{surface}/")))));
        }
        for surface in ["designer/tank", "designer/plane"] {
            assert!(hud.iter().any(|row| row
                .consumers
                .iter()
                .any(|consumer| consumer.starts_with(&format!("hud_role/v1/{surface}/")))));
        }

        let incompatible_frame = hud_role_observer_live_lane::HudSurfaceActivationFrame {
            frame_hour: 11,
            status_flags: 0,
            observation_sequence: 2,
            active_surfaces: vec![
                hud_role_observer_live_lane::HudSurfaceActivation {
                    surface: hud_role_observer_live_lane::HudSurface::Topbar,
                    runtime_root_name: "topbar".into(),
                },
                hud_role_observer_live_lane::HudSurfaceActivation {
                    surface: hud_role_observer_live_lane::HudSurface::Minimap,
                    runtime_root_name: "mini_map_interface".into(),
                },
                hud_role_observer_live_lane::HudSurfaceActivation {
                    surface: hud_role_observer_live_lane::HudSurface::Mapmodes,
                    runtime_root_name: "MapmodesInterface_Ingame".into(),
                },
                hud_role_observer_live_lane::HudSurfaceActivation {
                    surface: hud_role_observer_live_lane::HudSurface::TheatreSelector,
                    runtime_root_name: "missing_runtime_theatre".into(),
                },
            ],
        };
        let mut incompatible_stream = Vec::new();
        push_d79(
            &mut incompatible_stream,
            1,
            11,
            &hud_role_observer_live_lane::encode_activation_payload(&incompatible_frame).unwrap(),
        );
        fs::write(&stream_path, incompatible_stream).unwrap();
        let omitted_manifest = build_at(game_b, user_b);
        let omitted_hud = omitted_manifest
            .required_content
            .iter()
            .filter(|row| {
                row.consumers
                    .iter()
                    .any(|consumer| consumer.starts_with("hud_role/v1/"))
            })
            .collect::<Vec<_>>();
        assert_eq!(omitted_hud.len(), 3);
        assert!(!omitted_hud.iter().any(|row| row
            .consumers
            .iter()
            .any(|consumer| { consumer.starts_with("hud_role/v1/theatre_selector/") })));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn production_builds_in_process_from_finalized_catalog_source_join_without_sidecar() {
        let (_, launch) = base_trace();
        let dir = std::env::temp_dir().join(format!(
            "hoi4_portable_in_process_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _cleanup = TempFixtureCleanup(dir.clone());
        fs::create_dir_all(dir.join("proofs")).unwrap();
        let inventory = finalize_source_inventory(serde_json::json!({
            "schema": "extractor.live_binding_source_inventory.v3",
            "identity": {
                "profile_id": "profile:fixture",
                "playset_id": "playset:fixture"
            },
            "namespaces": [
                {"ordinal":0,"kind":"vanilla","binding":"base","id":"vanilla","label":"vanilla","replace_paths":[],"storage_kind":"directory","source_container_sha256":null,"source_container_bytes":null,"file_count":0,"content_sha256":sha('0'),"binding_content_sha256":sha('0')},
                {"ordinal":1,"kind":"dlc","binding":"asset_only","id":"dlc_asset_only","label":"evidence-only DLC","replace_paths":[],"storage_kind":"directory","source_container_sha256":null,"source_container_bytes":null,"file_count":0,"content_sha256":sha('8'),"binding_content_sha256":sha('8')},
                {"ordinal":2,"kind":"dlc","binding":"active_product","id":"dlc_used","label":"dlc/dlc_used","replace_paths":["gfx/models"],"storage_kind":"zip_archive","source_container_sha256":sha('c'),"source_container_bytes":1000,"file_count":0,"content_sha256":sha('1'),"binding_content_sha256":sha('c')},
                {"ordinal":3,"kind":"dlc","binding":"active_product","id":"dlc_unused","label":"dlc/dlc_unused","replace_paths":[],"storage_kind":"directory","source_container_sha256":null,"source_container_bytes":null,"file_count":0,"content_sha256":sha('d'),"binding_content_sha256":sha('d')},
                {"ordinal":4,"kind":"mod","binding":"active_mod","id":"mod/archive.mod","label":"Archive override","replace_paths":["common/defines"],"storage_kind":"zip_archive","source_container_sha256":sha('f'),"source_container_bytes":2000,"file_count":0,"content_sha256":sha('2'),"binding_content_sha256":sha('f')}
            ],
            "rows": [
                {"namespace_ordinal":0,"layer_kind":"vanilla","layer_id":"vanilla","relpath":"common/base.txt","bytes":1,"sha256":sha('0'),"storage_kind":"directory","source_container_sha256":null,"source_byte_offset":0,"source_byte_count":1,"source_range_sha256":sha('0')},
                {"namespace_ordinal":0,"layer_kind":"vanilla","layer_id":"vanilla","relpath":"interface/base.gui","bytes":11,"sha256":sha('a'),"storage_kind":"directory","source_container_sha256":null,"source_byte_offset":0,"source_byte_count":11,"source_range_sha256":sha('a')},
                {"namespace_ordinal":0,"layer_kind":"vanilla","layer_id":"vanilla","relpath":"interface/base.gfx","bytes":12,"sha256":sha('b'),"storage_kind":"directory","source_container_sha256":null,"source_byte_offset":0,"source_byte_count":12,"source_range_sha256":sha('b')},
                {"namespace_ordinal":0,"layer_kind":"vanilla","layer_id":"vanilla","relpath":"localisation/english/base_l_english.yml","bytes":13,"sha256":sha('e'),"storage_kind":"directory","source_container_sha256":null,"source_byte_offset":0,"source_byte_count":13,"source_range_sha256":sha('e')},
                {"namespace_ordinal":1,"layer_kind":"dlc","layer_id":"dlc_asset_only","relpath":"gfx/models/evidence.mesh","bytes":99,"sha256":sha('8'),"storage_kind":"directory","source_container_sha256":null,"source_byte_offset":0,"source_byte_count":99,"source_range_sha256":sha('8')},
                {"namespace_ordinal":2,"layer_kind":"dlc","layer_id":"dlc_used","relpath":"gfx/models/tank.mesh","bytes":123,"sha256":sha('1'),"storage_kind":"zip_archive","source_container_sha256":sha('c'),"source_byte_offset":100,"source_byte_count":77,"source_range_sha256":sha('6')},
                {"namespace_ordinal":3,"layer_kind":"dlc","layer_id":"dlc_unused","relpath":"common/unused.txt","bytes":7,"sha256":sha('d'),"storage_kind":"directory","source_container_sha256":null,"source_byte_offset":0,"source_byte_count":7,"source_range_sha256":sha('d')},
                {"namespace_ordinal":4,"layer_kind":"mod","layer_id":"mod/archive.mod","relpath":"common/defines/00_defines.lua","bytes":456,"sha256":sha('2'),"storage_kind":"zip_archive","source_container_sha256":sha('f'),"source_byte_offset":200,"source_byte_count":111,"source_range_sha256":sha('7')}
            ],
            "namespace_sha256": sha('3'),
            "rows_sha256": sha('4'),
            "inventory_sha256": sha('a')
        }));
        let inventory_sha256 = inventory
            .get("inventory_sha256")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        let join = serde_json::json!({
            "schema": "extractor.static_catalog_source_join.v1",
            "passed": true,
            "inventory_sha256": inventory_sha256,
            "roles": [],
            "bindings": [
                {"artifact_role":"gfx_icon_asset_manifest_static","relpath":"gfx/models/tank.mesh","bytes":123,"sha256":sha('1'),"namespace_ordinal":2,"layer_kind":"dlc","layer_id":"dlc_used"},
                {"artifact_role":"templates_equipment_catalog","relpath":"common/defines/00_defines.lua","bytes":456,"sha256":sha('2'),"namespace_ordinal":4,"layer_kind":"mod","layer_id":"mod/archive.mod"}
            ],
            "bindings_sha256": sha('5')
        });
        let report = serde_json::json!({
            "schema": "extractor.live_binding_resolution.v1",
            "passed": true,
            "engine_runtime_artifacts": engine_runtime_artifact_declaration(),
            "process": {"exe_sha256": sha('9')},
            "runtime": {"version": "1.19.2.0.a729"},
            "binding_identity": {
                "profile_id": "profile:fixture",
                "playset_id": "playset:fixture"
            }
        });
        let inventory_path = dir.join("source_inventory.json");
        let join_path = dir.join("proofs/catalog_source_join.json");
        let report_path = dir.join("resolution_report.json");
        fs::write(&inventory_path, serde_json::to_vec(&inventory).unwrap()).unwrap();
        fs::write(&join_path, serde_json::to_vec(&join).unwrap()).unwrap();
        fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
        assert!(!dir.join(DEFAULT_TRACE_FILE).exists());
        let manifest = build_from_finalized_static_bundle(
            &inventory_path,
            &join_path,
            &report_path,
            &launch,
            &sha('9'),
        )
        .unwrap();
        assert_eq!(manifest.required_content.len(), 5);
        assert!(
            manifest
                .dlc_selection
                .iter()
                .find(|row| row.dlc_id == "dlc_used")
                .unwrap()
                .required_for_playback
        );
        assert!(
            !manifest
                .dlc_selection
                .iter()
                .find(|row| row.dlc_id == "dlc_unused")
                .unwrap()
                .required_for_playback
        );
        assert_eq!(
            manifest.ordered_layers[1].layer_id,
            "dlc/dlc_used/dlc_used.dlc"
        );
        assert_eq!(
            manifest.ordered_layers[1].source_container_sha256,
            Some(sha('c'))
        );
        assert_eq!(manifest.ordered_layers[1].replace_paths, vec!["gfx/models"]);
        assert_eq!(manifest.ordered_layers[3].descriptor_sha256, Some(sha('e')));
        assert_eq!(
            manifest.ordered_layers[3].source_container_sha256,
            Some(sha('f'))
        );
        assert!(!dir.join(DEFAULT_TRACE_FILE).exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn winner_or_payload_drift_for_repeated_path_fails_closed() {
        let (mut trace, launch) = base_trace();
        let mut repeated = trace.observations[0].clone();
        repeated.content.payload_sha256 = sha('3');
        trace.observations.push(repeated);
        assert!(build(trace, &launch, &sha('9'))
            .unwrap_err()
            .contains("changed key/kind/winner/hash"));
    }

    #[test]
    fn launch_mod_dlc_language_and_settings_are_cross_checked() {
        let (trace, mut launch) = base_trace();
        launch.enabled_dlc_ids.remove("dlc_unused");
        assert!(build(trace.clone(), &launch, &sha('9'))
            .unwrap_err()
            .contains("enabled DLC set"));
        let (_, mut launch) = base_trace();
        launch.ordered_mods[0].content_sha256 = sha('0');
        assert!(build(trace.clone(), &launch, &sha('9'))
            .unwrap_err()
            .contains("archive hash"));
        let (_, mut launch) = base_trace();
        launch.localisation_language = "german".into();
        assert!(build(trace, &launch, &sha('9'))
            .unwrap_err()
            .contains("language"));
    }

    #[test]
    fn dlc_selection_identity_is_full_folder_not_descriptor_stem() {
        assert_eq!(
            stable_dlc_id("dlc/dlc003_rocket_launcher_unit_pack/dlc003.dlc").unwrap(),
            "dlc003_rocket_launcher_unit_pack"
        );
    }

    #[test]
    fn manifest_has_no_local_paths_and_survives_relocation() {
        let (trace, launch) = base_trace();
        let manifest = build(trace, &launch, &sha('9')).unwrap();
        let bytes = manifest.encode_json().unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(!text.contains("C:\\"));
        assert!(!text.contains("/Users/"));
        assert!(!text.contains("OneDrive"));
        let relocated = portable_content_manifest::ContentManifest::decode_json(&bytes).unwrap();
        assert_eq!(relocated, manifest);
    }

    #[test]
    fn extractor_manifest_round_trips_through_portable_container() {
        let (trace, launch) = base_trace();
        let manifest = build(trace, &launch, &sha('9')).unwrap();
        let index = br#"{"game_id":"portable-fixture","schema_version":"1","snapshots":[]}"#;
        let entries = [snapshot::BorrowedPackEntry {
            path: "payload/portable-fixture/index.json".into(),
            bytes: index,
            role: "index",
            hour_index: None,
        }];
        let identity = snapshot::PackIdentity {
            schema_version: Some("1".into()),
            content_manifest: Some(manifest.clone()),
            ..Default::default()
        };
        let bytes =
            snapshot::pack_borrowed("portable-fixture", &entries, &identity, false).unwrap();
        let opened =
            snapshot::Container::from_bytes(bytes, &snapshot::ExpectedIdentity::default()).unwrap();
        assert_eq!(opened.content_manifest(), Some(&manifest));
        assert_eq!(
            opened.manifest()["required_features"],
            serde_json::json!(["portable_content_manifest_v2"])
        );
    }

    #[test]
    fn local_modset_paths_are_reduced_to_portable_launch_identity() {
        let dir = std::env::temp_dir().join(format!(
            "hoi4_portable_descriptor_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let descriptor = dir.join("archive.mod");
        let descriptor_text = format!(
            "name=\"Archive override\"\npath=\"{}\"\nversion=\"1\"\nreplace_path=\"common/defines\"\n",
            dir.join("some-machine-root").display()
        );
        fs::write(&descriptor, descriptor_text.as_bytes()).unwrap();
        let archive = dir.join("archive.zip");
        fs::write(&archive, b"fixture archive identity").unwrap();
        let game = dir.join("game");
        fs::create_dir_all(game.join("dlc/dlc_used")).unwrap();
        fs::create_dir_all(game.join("dlc/dlc_unused")).unwrap();
        fs::write(game.join("hoi4.exe"), b"fixture exe").unwrap();
        fs::write(
            game.join("dlc/dlc_used/dlc_used.dlc"),
            b"name=\"Used\"\narchive=\"dlc/dlc_used/content.zip\"\n",
        )
        .unwrap();
        fs::write(
            game.join("dlc/dlc_used/content.zip"),
            b"fixture dlc archive",
        )
        .unwrap();
        fs::write(
            game.join("dlc/dlc_unused/dlc_unused.dlc"),
            b"name=\"Unused\"\npath=\"dlc/dlc_unused\"\n",
        )
        .unwrap();
        let report = ModsetReport {
            content_resolved: true,
            errors: vec![],
            settings: crate::modset::SettingsBinding {
                authority: Some(crate::modset::SettingsAuthority::PreLaunchBracketed),
                errors: Vec::new(),
                language: Some("english".into()),
                display: Some(DisplayVariantSettings {
                    physical_width: 1920,
                    physical_height: 1080,
                    min_gui_width: 1280,
                    min_gui_height: 720,
                    gui_scale_f32_bits: 1.0f32.to_bits(),
                    refresh_rate_hz: 60,
                    max_refresh_rate_hz: Some(144),
                    fullscreen: false,
                    borderless: true,
                    display_mode: "borderless".into(),
                    settings_file_sha256: sha('7'),
                }),
                attach_sha256: Some(sha('7')),
                seal_sha256: None,
                endpoint_semantics_equal: None,
            },
            exe_path: Some(game.join("hoi4.exe")),
            exe_size_bytes: Some(1),
            exe_sha256: Some(sha('9')),
            user_dir: Some(PathBuf::from(r"C:\Users\Example\Documents\Paradox\HOI4")),
            user_dir_source: Some("override"),
            dlc_load_mtime_unix: Some(1),
            dlc_load_sha256: Some(sha('8')),
            process_start_unix: Some(2),
            process_start_filetime: None,
            enabled_mods: vec![ResolvedMod {
                descriptor_entry: "mod/archive.mod".into(),
                descriptor_path: Some(descriptor.clone()),
                descriptor_sha256: Some(Sha256::hex(descriptor_text.as_bytes())),
                name: Some("Archive override".into()),
                version: Some("1".into()),
                remote_file_id: None,
                content_path: Some(archive.to_string_lossy().into_owned()),
                content_fingerprint: Some((2, 99, 1)),
                content_sha256: Some(sha('f')),
                content_max_mtime: None,
                descriptor_mtime: None,
                content_location_conflict: false,
            }],
            installed_dlcs: vec![
                "dlc/dlc_used/dlc_used.dlc".into(),
                "dlc/dlc_unused/dlc_unused.dlc".into(),
            ],
            disabled_dlcs: vec!["dlc/dlc_unused/dlc_unused.dlc".into()],
            enabled_dlcs: vec!["dlc/dlc_used/dlc_used.dlc".into()],
            dlc_display_names: BTreeMap::new(),
            dlc_descriptor_evidence: BTreeMap::from([
                (
                    "dlc/dlc_used/dlc_used.dlc".into(),
                    Some(DlcDescriptorEvidence {
                        display_name: "Used".into(),
                        descriptor_file_sha256: Sha256::hex(
                            b"name=\"Used\"\narchive=\"dlc/dlc_used/content.zip\"\n",
                        ),
                    }),
                ),
                (
                    "dlc/dlc_unused/dlc_unused.dlc".into(),
                    Some(DlcDescriptorEvidence {
                        display_name: "Unused".into(),
                        descriptor_file_sha256: Sha256::hex(
                            b"name=\"Unused\"\npath=\"dlc/dlc_unused\"\n",
                        ),
                    }),
                ),
            ]),
        };
        let portable = PortableLaunchBinding::from_modset(&report).unwrap();
        assert!(
            portable.required_settings.is_empty(),
            "the recorder demands no setting of the replaying install: {:?}",
            portable.required_settings
        );
        for key in portable.recorder_local_settings.keys() {
            assert!(
                portable_content_manifest::is_recorder_local_settings(key),
                "{key:?} is a material setting hiding in recorder_local_settings"
            );
        }
        for key in [
            "settings_authority",
            "settings_file_sha256",
            "display_mode",
            "gui_scale_f32_bits",
        ] {
            assert!(
                portable.recorder_local_settings.contains_key(key),
                "{key} must be recorded as the recorder's own local setting"
            );
        }
        let serialized = format!("{portable:?}");
        assert!(!serialized.contains("C:\\"));
        assert!(!serialized.contains("D:\\"));
        assert_eq!(portable.ordered_mods[0].layer_id, "mod/archive.mod");
        assert_eq!(
            portable.enabled_dlc_ids,
            BTreeSet::from(["dlc_used".into()])
        );
        let effective_candidate =
            PortableLaunchBinding::from_effective_runtime_candidate(&report).unwrap();
        assert_eq!(
            effective_candidate.enabled_dlc_ids,
            BTreeSet::from(["dlc_used".into(), "dlc_unused".into()])
        );
        assert_eq!(
            portable.ordered_mods[0].descriptor_sha256,
            semantic_descriptor_sha256(&descriptor_text)
        );
        let relocated = descriptor_text.replace(
            &dir.join("some-machine-root").display().to_string(),
            r"Z:\Other\Profile\same-content",
        );
        assert_eq!(
            semantic_descriptor_sha256(&descriptor_text),
            semantic_descriptor_sha256(&relocated)
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn latin1_mod_descriptor_hash_matches_playback_semantics() {
        let dir = std::env::temp_dir().join(format!(
            "hoi4_portable_latin1_descriptor_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let descriptor = dir.join("latin1.mod");
        let bytes = b"name=\"M\xfcnchen\"\nversion=\"1\"\nreplace_path=\"common/defines\"\n";
        fs::write(&descriptor, bytes).unwrap();
        let source = ResolvedMod {
            descriptor_entry: "mod/latin1.mod".into(),
            descriptor_path: Some(descriptor),
            descriptor_sha256: Some(Sha256::hex(bytes)),
            name: Some("München".into()),
            version: Some("1".into()),
            remote_file_id: None,
            content_path: Some(dir.to_string_lossy().into_owned()),
            content_fingerprint: Some((1, 1, 1)),
            content_sha256: Some(sha('f')),
            content_max_mtime: None,
            descriptor_mtime: None,
            content_location_conflict: false,
        };
        let playback_decoded = bytes
            .iter()
            .map(|&byte| char::from(byte))
            .collect::<String>();
        assert_eq!(
            semantic_mod_descriptor_sha256(&source).unwrap(),
            semantic_descriptor_sha256(&playback_decoded)
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn latin1_dlc_descriptor_binding_matches_playback_semantics_and_archive_hash() {
        let dir = std::env::temp_dir().join(format!(
            "hoi4_portable_latin1_dlc_descriptor_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dlc_dir = dir.join("dlc").join("dlc_latin1");
        fs::create_dir_all(&dlc_dir).unwrap();
        let descriptor = dlc_dir.join("dlc_latin1.dlc");
        let archive = dlc_dir.join("dlc_latin1.zip");
        let descriptor_bytes =
            b"name=\"M\xfcnchen Pack\"\nversion=\"1\"\narchive=\"dlc/dlc_latin1/dlc_latin1.zip\"\n";
        let archive_bytes = b"stable archive identity bytes";
        fs::write(&descriptor, descriptor_bytes).unwrap();
        fs::write(&archive, archive_bytes).unwrap();

        let bound = bound_dlc_layer(&dir, "dlc/dlc_latin1/dlc_latin1.dlc").unwrap();
        let playback_decoded = descriptor_bytes
            .iter()
            .map(|&byte| char::from(byte))
            .collect::<String>();
        assert_eq!(bound.layer_id, "dlc/dlc_latin1/dlc_latin1.dlc");
        assert_eq!(
            bound.descriptor_sha256,
            semantic_descriptor_sha256(&playback_decoded)
        );
        assert_eq!(bound.storage_kind, LayerStorageKind::ZipArchive);
        assert_eq!(
            bound.source_container_sha256,
            Some(Sha256::hex(archive_bytes))
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn integrated_dlc_ignores_descriptor_archive_and_binds_physical_directory() {
        let dir = std::env::temp_dir().join(format!(
            "hoi4_portable_integrated_dlc_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dlc_dir = dir.join("integrated_dlc/dlc900_pathological");
        fs::create_dir_all(&dlc_dir).unwrap();
        let descriptor = dlc_dir.join("dlc900.dlc");
        let bytes = b"name=\"Integrated\"\narchive=\"missing/ignored.zip\"\n";
        fs::write(&descriptor, bytes).unwrap();

        let layers = discover_integrated_dlc_layers(&dir).unwrap();
        let bound = layers.get("dlc900_pathological").unwrap();
        assert_eq!(bound.storage_kind, LayerStorageKind::Directory);
        assert_eq!(bound.source_container_sha256, None);
        assert_eq!(
            bound.descriptor_sha256,
            semantic_descriptor_sha256(&latin1_bytes_to_string(bytes))
        );
        portable_test_fs_retry::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn directory_mod_override_and_changed_definition_change_manifest_identity() {
        let (mut trace, mut launch) = base_trace();
        let mod_layer = trace
            .ordered_layers
            .iter_mut()
            .find(|layer| layer.layer_kind == ContentLayerKind::Mod)
            .unwrap();
        mod_layer.storage_kind = LayerStorageKind::Directory;
        mod_layer.source_container_sha256 = None;
        launch.ordered_mods[0].content_sha256 = sha('6');
        launch.ordered_mods[0].storage_kind = LayerStorageKind::Directory;
        let definition = trace
            .observations
            .iter_mut()
            .find(|row| row.content_key == "defines")
            .unwrap();
        definition.content.source_sha256 = definition.content.payload_sha256.clone();
        let first = build(trace.clone(), &launch, &sha('9')).unwrap();
        let first_digest = first.manifest_digest_sha256.clone().unwrap();

        let definition = trace
            .observations
            .iter_mut()
            .find(|row| row.content_key == "defines")
            .unwrap();
        definition.content.source_sha256 = sha('5');
        definition.content.payload_sha256 = sha('5');
        definition.content.payload_bytes += 1;
        let changed = build(trace, &launch, &sha('9')).unwrap();
        assert_ne!(
            changed.manifest_digest_sha256.as_deref(),
            Some(first_digest.as_str())
        );
    }

    #[test]
    fn runtime_version_build_and_profile_are_bound_without_local_paths() {
        let (trace, launch) = base_trace();
        let manifest = build(trace, &launch, &sha('9')).unwrap();
        let path = std::env::temp_dir().join(format!(
            "hoi4_portable_binding_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let report = serde_json::json!({
            "schema": "extractor.live_binding_resolution.v1",
            "passed": true,
            "engine_runtime_artifacts": engine_runtime_artifact_declaration(),
            "process": { "exe_sha256": launch.executable_sha256 },
            "runtime": { "version": "Stella Polaris v1.19.2.0.a729" },
            "binding_identity": {
                "profile_id": manifest.source_profile.profile_id,
                "playset_id": manifest.source_profile.playset_id,
            },
        });
        fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
        let result = verify_live_resolution_report(&path, &manifest, &launch);
        result.unwrap();

        let mut undeclared = report.clone();
        undeclared
            .as_object_mut()
            .unwrap()
            .remove("engine_runtime_artifacts");
        fs::write(&path, serde_json::to_vec(&undeclared).unwrap()).unwrap();
        let refused = verify_live_resolution_report(&path, &manifest, &launch);
        portable_test_fs_retry::remove_file(&path).unwrap();
        assert!(refused
            .unwrap_err()
            .contains("engine runtime artifact domain"));
    }
}
