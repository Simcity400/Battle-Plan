use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use app::host::ViewRegistry;
use app::views::map::MAP_VIEW_ID;
use app::views::state_panel::STATE_PANEL_VIEW_ID;

pub const SHOT_ALL_MANIFEST: &str = "shot_manifest.json";
pub const SHOT_MANIFEST_VERSION: u64 = 2;
pub const DEFAULT_STATE_PANEL_SHOT_STATE: u32 = 1;
pub const BASE_SHOT_CASE_ID: &str = "base";
pub const MAP_MODE_SHOT_CASES: &[(&str, &str, &str)] = &[
    ("map_mode_control", "Map mode: Control", "control"),
    ("map_mode_weather", "Map mode: Weather", "weather"),
    ("map_mode_supply", "Map mode: Supply", "supply"),
];
pub const STATE_PANEL_SELECTED_STATE_SHOT_CASES: &[(&str, &str, u32)] =
    &[("state_panel_state_64", "State panel: State 64", 64)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShotCaseSpec {
    id: &'static str,
    family: &'static str,
    label: &'static str,
    map_mode_slug: Option<&'static str>,
    shot_state_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotTarget {
    pub view_id: &'static str,
    pub case_id: &'static str,
    pub case_family: &'static str,
    pub case_label: &'static str,
    pub native_filename: String,
    pub hoi4_reference_filename: String,
    pub diff_filename: String,
    pub native_path: PathBuf,
    pub hoi4_reference_path: PathBuf,
    pub diff_path: PathBuf,
    pub native_args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotManifestWrite {
    pub path: PathBuf,
    pub target_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestShotTarget {
    pub view_id: String,
    pub case_id: String,
    pub case_family: String,
    pub case_label: String,
    pub native_filename: String,
    pub hoi4_reference_filename: String,
    pub diff_filename: String,
    pub native_args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotManifestRead {
    pub path: PathBuf,
    pub target_count: usize,
    pub targets: Vec<ManifestShotTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeShotBatchConfig {
    pub command: PathBuf,
    pub common_args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeShotInvocation {
    pub view_id: String,
    pub command: PathBuf,
    pub current_dir: PathBuf,
    pub args: Vec<String>,
    pub native_path: PathBuf,
    pub hoi4_reference_path: PathBuf,
    pub diff_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeShotBatchRun {
    pub manifest_path: PathBuf,
    pub captured_count: usize,
    pub pending_hoi4_reference_count: usize,
    pub pending_diff_count: usize,
}

fn target_owned_shot_arg(arg: &str) -> Option<&'static str> {
    for flag in ["--shot", "--shot-view", "--shot-state", "--map-mode"] {
        if arg == flag
            || arg
                .strip_prefix(flag)
                .is_some_and(|rest| rest.starts_with('='))
        {
            return Some(flag);
        }
    }
    None
}

fn validate_view_id_for_filename(id: &str) -> Result<(), String> {
    if id.is_empty() || id.trim() != id || id == "." || id == ".." {
        return Err(format!(
            "invalid registered view id '{id}' for screenshot target"
        ));
    }
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(format!(
            "invalid registered view id '{id}' for screenshot target: unsupported path character"
        ));
    }

    for segment in id.split('.') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(format!(
                "invalid registered view id '{id}' for screenshot target: empty path segment"
            ));
        }
        let upper = segment.to_ascii_uppercase();
        if matches!(
            upper.as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        ) {
            return Err(format!(
                "invalid registered view id '{id}' for screenshot target: reserved Windows device name"
            ));
        }
    }
    Ok(())
}

fn validate_case_id_for_filename(id: &str) -> Result<(), String> {
    if id.is_empty() || id.trim() != id || id == "." || id == ".." {
        return Err(format!("invalid screenshot case id '{id}'"));
    }
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err(format!(
            "invalid screenshot case id '{id}': unsupported path character"
        ));
    }
    Ok(())
}

fn base_shot_case() -> ShotCaseSpec {
    ShotCaseSpec {
        id: BASE_SHOT_CASE_ID,
        family: "base",
        label: "Base registered view",
        map_mode_slug: None,
        shot_state_id: None,
    }
}

fn shot_cases_for_view_id(id: &str) -> Vec<ShotCaseSpec> {
    let mut cases = vec![base_shot_case()];
    if id == MAP_VIEW_ID.as_str() {
        cases.extend(
            MAP_MODE_SHOT_CASES
                .iter()
                .map(|&(case_id, label, slug)| ShotCaseSpec {
                    id: case_id,
                    family: "map_modes",
                    label,
                    map_mode_slug: Some(slug),
                    shot_state_id: None,
                }),
        );
    }
    if id == STATE_PANEL_VIEW_ID.as_str() {
        cases.extend(STATE_PANEL_SELECTED_STATE_SHOT_CASES.iter().map(
            |&(case_id, label, state_id)| ShotCaseSpec {
                id: case_id,
                family: "state_panel_selected_states",
                label,
                map_mode_slug: None,
                shot_state_id: Some(state_id),
            },
        ));
    }
    cases
}

fn shot_case_for_view_id(id: &str, case_id: &str) -> Option<ShotCaseSpec> {
    shot_cases_for_view_id(id)
        .into_iter()
        .find(|case| case.id == case_id)
}

pub fn shot_filename_for_view_id(id: &str) -> Result<String, String> {
    validate_view_id_for_filename(id)?;
    Ok(format!("native__{id}.png"))
}

fn paired_filenames_for_shot_case(
    id: &str,
    case_id: &str,
) -> Result<(String, String, String), String> {
    validate_view_id_for_filename(id)?;
    validate_case_id_for_filename(case_id)?;
    if case_id == BASE_SHOT_CASE_ID {
        Ok((
            shot_filename_for_view_id(id)?,
            format!("hoi4__{id}.png"),
            format!("diff__{id}.png"),
        ))
    } else {
        Ok((
            format!("native__{id}__{case_id}.png"),
            format!("hoi4__{id}__{case_id}.png"),
            format!("diff__{id}__{case_id}.png"),
        ))
    }
}

fn native_args_for_shot_case(id: &str, case: ShotCaseSpec, native_filename: &str) -> Vec<String> {
    let mut args = vec![
        "--shot".to_string(),
        native_filename.to_string(),
        "--shot-view".to_string(),
        id.to_string(),
    ];
    if id == STATE_PANEL_VIEW_ID.as_str() {
        args.push("--shot-state".to_string());
        args.push(
            case.shot_state_id
                .unwrap_or(DEFAULT_STATE_PANEL_SHOT_STATE)
                .to_string(),
        );
    }
    if let Some(slug) = case.map_mode_slug {
        args.push("--map-mode".to_string());
        args.push(slug.to_string());
    }
    args
}

pub fn build_registry_shot_matrix(
    registry: &ViewRegistry,
    output_dir: &Path,
) -> Result<Vec<ShotTarget>, String> {
    let mut seen_filenames = BTreeSet::new();
    let mut targets = Vec::new();
    for descriptor in registry.iter_views() {
        let view_id = descriptor.id.as_str();
        for case in shot_cases_for_view_id(view_id) {
            let (native_filename, hoi4_reference_filename, diff_filename) =
                paired_filenames_for_shot_case(view_id, case.id)?;
            for filename in [&native_filename, &hoi4_reference_filename, &diff_filename] {
                let collision_key = filename.to_ascii_lowercase();
                if !seen_filenames.insert(collision_key) {
                    return Err(format!(
                        "duplicate screenshot target filename '{filename}' from registered view id '{view_id}' case '{}'",
                        case.id
                    ));
                }
            }
            let native_args = native_args_for_shot_case(view_id, case, &native_filename);
            targets.push(ShotTarget {
                view_id,
                case_id: case.id,
                case_family: case.family,
                case_label: case.label,
                native_path: output_dir.join(&native_filename),
                hoi4_reference_path: output_dir.join(&hoi4_reference_filename),
                diff_path: output_dir.join(&diff_filename),
                native_filename,
                hoi4_reference_filename,
                diff_filename,
                native_args,
            });
        }
    }
    if targets.is_empty() {
        return Err("--shot-all-views found no registered views".to_string());
    }
    Ok(targets)
}

pub fn write_registry_shot_manifest(
    registry: &ViewRegistry,
    output_dir: &Path,
) -> Result<ShotManifestWrite, String> {
    if output_dir.as_os_str().is_empty() {
        return Err("--shot-all-views requires an existing output directory".to_string());
    }
    let metadata = std::fs::metadata(output_dir).map_err(|e| {
        format!(
            "--shot-all-views output '{}' must be an existing directory: {e}",
            output_dir.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "--shot-all-views output '{}' must be an existing directory",
            output_dir.display()
        ));
    }

    let targets = build_registry_shot_matrix(registry, output_dir)?;
    for target in &targets {
        for path in [
            &target.native_path,
            &target.hoi4_reference_path,
            &target.diff_path,
        ] {
            if path.is_dir() {
                return Err(format!(
                    "--shot-all-views target '{}' is an existing directory",
                    path.display()
                ));
            }
        }
    }
    let manifest_path = output_dir.join(SHOT_ALL_MANIFEST);
    let json_targets: Vec<_> = targets
        .iter()
        .map(|target| {
            serde_json::json!({
                "view_id": target.view_id,
                "case_id": target.case_id,
                "case_family": target.case_family,
                "case_label": target.case_label,
                "native_shot_file": target.native_filename,
                "hoi4_reference_file": target.hoi4_reference_filename,
                "diff_file": target.diff_filename,
                "native_status": "pending_native_capture",
                "hoi4_reference_status": "pending_external_reference_capture",
                "diff_status": "pending_external_comparison",
                "shot_file": target.native_filename,
                "native_shot_args": target.native_args,
                "shot_args": target.native_args,
            })
        })
        .collect();
    let base_view_count = registry.len();
    let extra_case_count = targets.len().saturating_sub(base_view_count);
    let manifest = serde_json::json!({
        "version": SHOT_MANIFEST_VERSION,
        "kind": "shot_all_views_manifest",
        "status": "native_targets_planned",
        "note": "--shot-all-views records registry-derived base screenshot targets plus deterministic native-only case expansions. --shot-all-views-run-native can emit native__ shots from this manifest; the game reference capture and diff review remain external and pending.",
        "path_base": "manifest_dir",
        "native_args_cwd": "manifest_dir",
        "registry_view_count": base_view_count,
        "target_count": targets.len(),
        "extra_case_count": extra_case_count,
        "manifest_file": SHOT_ALL_MANIFEST,
        "default_state_panel_shot_state": DEFAULT_STATE_PANEL_SHOT_STATE,
        "case_expansion": {
            "status": "deterministic",
            "families": [{
                "family": "map_modes",
                "view_id": MAP_VIEW_ID.as_str(),
                "cases": MAP_MODE_SHOT_CASES.iter().map(|&(case_id, label, slug)| {
                    serde_json::json!({
                        "case_id": case_id,
                        "case_label": label,
                        "map_mode": slug,
                    })
                }).collect::<Vec<_>>(),
            }, {
                "family": "state_panel_selected_states",
                "view_id": STATE_PANEL_VIEW_ID.as_str(),
                "cases": STATE_PANEL_SELECTED_STATE_SHOT_CASES.iter().map(|&(case_id, label, state_id)| {
                    serde_json::json!({
                        "case_id": case_id,
                        "case_label": label,
                        "shot_state": state_id,
                    })
                }).collect::<Vec<_>>(),
            }],
        },
        "native_batch_runner": {
            "status": "available_with_runtime_command",
            "run_flag": "--shot-all-views-run-native",
            "command_flag": "--shot-all-views-command",
            "emits": "native__*.png only",
            "does_not_emit": ["hoi4__*.png", "diff__*.png"],
        },
        "targets": json_targets,
    });
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| format!("serialize shot-all-views manifest failed: {e}"))?;
    std::fs::write(&manifest_path, bytes).map_err(|e| {
        format!(
            "write shot-all-views manifest '{}' failed: {e}",
            manifest_path.display()
        )
    })?;
    Ok(ShotManifestWrite {
        path: manifest_path,
        target_count: targets.len(),
    })
}

fn manifest_string_field(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
    index: usize,
) -> Result<String, String> {
    object
        .get(field)
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .ok_or_else(|| format!("shot manifest target {index} missing string field '{field}'"))
}

fn manifest_string_array_field(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
    index: usize,
) -> Result<Vec<String>, String> {
    let raw = object
        .get(field)
        .and_then(|value| value.as_array())
        .ok_or_else(|| {
            format!("shot manifest target {index} missing string array field '{field}'")
        })?;
    raw.iter()
        .map(|value| {
            value.as_str().map(ToString::to_string).ok_or_else(|| {
                format!("shot manifest target {index} field '{field}' contains a non-string arg")
            })
        })
        .collect()
}

fn manifest_usize_field(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
    path: &Path,
) -> Result<usize, String> {
    let raw = object
        .get(field)
        .and_then(|value| value.as_u64())
        .ok_or_else(|| {
            format!(
                "shot-all-views manifest '{}' missing non-negative integer field '{field}'",
                path.display()
            )
        })?;
    usize::try_from(raw).map_err(|_| {
        format!(
            "shot-all-views manifest '{}' field '{field}' is too large",
            path.display()
        )
    })
}

pub fn read_shot_manifest(path: &Path) -> Result<ShotManifestRead, String> {
    let raw = std::fs::read(path).map_err(|e| {
        format!(
            "read shot-all-views manifest '{}' failed: {e}",
            path.display()
        )
    })?;
    let manifest: serde_json::Value = serde_json::from_slice(&raw).map_err(|e| {
        format!(
            "parse shot-all-views manifest '{}' failed: {e}",
            path.display()
        )
    })?;
    let object = manifest.as_object().ok_or_else(|| {
        format!(
            "shot-all-views manifest '{}' must be a JSON object",
            path.display()
        )
    })?;
    let version = object
        .get("version")
        .and_then(|value| value.as_u64())
        .ok_or_else(|| {
            format!(
                "shot-all-views manifest '{}' missing numeric version",
                path.display()
            )
        })?;
    if version != SHOT_MANIFEST_VERSION {
        return Err(format!(
            "shot-all-views manifest '{}' has unsupported version {version} (expected {SHOT_MANIFEST_VERSION})",
            path.display()
        ));
    }
    if object.get("kind").and_then(|value| value.as_str()) != Some("shot_all_views_manifest") {
        return Err(format!(
            "shot-all-views manifest '{}' has unsupported kind",
            path.display()
        ));
    }
    if object.get("path_base").and_then(|value| value.as_str()) != Some("manifest_dir") {
        return Err(format!(
            "shot-all-views manifest '{}' must use path_base=manifest_dir",
            path.display()
        ));
    }
    if object
        .get("native_args_cwd")
        .and_then(|value| value.as_str())
        != Some("manifest_dir")
    {
        return Err(format!(
            "shot-all-views manifest '{}' must use native_args_cwd=manifest_dir",
            path.display()
        ));
    }
    let raw_targets = object
        .get("targets")
        .and_then(|value| value.as_array())
        .ok_or_else(|| {
            format!(
                "shot-all-views manifest '{}' missing targets array",
                path.display()
            )
        })?;
    if raw_targets.is_empty() {
        return Err(format!(
            "shot-all-views manifest '{}' has no targets",
            path.display()
        ));
    }
    let declared_target_count = manifest_usize_field(object, "target_count", path)?;
    if declared_target_count != raw_targets.len() {
        return Err(format!(
            "shot-all-views manifest '{}' target_count={declared_target_count} does not match targets array length {}",
            path.display(),
            raw_targets.len()
        ));
    }
    let declared_registry_view_count = manifest_usize_field(object, "registry_view_count", path)?;
    let declared_extra_case_count = manifest_usize_field(object, "extra_case_count", path)?;

    let mut seen_filenames = BTreeSet::new();
    let mut targets = Vec::new();
    for (index, raw_target) in raw_targets.iter().enumerate() {
        let target = raw_target.as_object().ok_or_else(|| {
            format!("shot-all-views manifest target {index} must be a JSON object")
        })?;
        let view_id = manifest_string_field(target, "view_id", index)?;
        let case_id = manifest_string_field(target, "case_id", index)?;
        let case_family = manifest_string_field(target, "case_family", index)?;
        let case_label = manifest_string_field(target, "case_label", index)?;
        let native_filename = manifest_string_field(target, "native_shot_file", index)?;
        let hoi4_reference_filename = manifest_string_field(target, "hoi4_reference_file", index)?;
        let diff_filename = manifest_string_field(target, "diff_file", index)?;
        let native_args = manifest_string_array_field(target, "native_shot_args", index)?;

        let case = shot_case_for_view_id(&view_id, &case_id).ok_or_else(|| {
            format!(
                "shot-all-views manifest target {index} has unsupported case '{case_id}' for view id '{view_id}'"
            )
        })?;
        if case_family != case.family || case_label != case.label {
            return Err(format!(
                "shot-all-views manifest target {index} case metadata does not match view id '{view_id}' case '{case_id}'"
            ));
        }
        let (expected_native, expected_hoi4, expected_diff) =
            paired_filenames_for_shot_case(&view_id, &case_id)?;
        if native_filename != expected_native
            || hoi4_reference_filename != expected_hoi4
            || diff_filename != expected_diff
        {
            return Err(format!(
                "shot-all-views manifest target {index} filenames do not match view id '{view_id}' case '{case_id}'"
            ));
        }
        let expected_args = native_args_for_shot_case(&view_id, case, &native_filename);
        if native_args != expected_args {
            return Err(format!(
                "shot-all-views manifest target {index} native_shot_args do not match view id '{view_id}' case '{case_id}'"
            ));
        }
        if let Some(alias) = target.get("shot_file").and_then(|value| value.as_str()) {
            if alias != native_filename {
                return Err(format!(
                    "shot-all-views manifest target {index} shot_file alias does not match native_shot_file"
                ));
            }
        }
        if let Some(alias_args) = target.get("shot_args") {
            let alias_args = alias_args.as_array().ok_or_else(|| {
                format!("shot-all-views manifest target {index} shot_args alias must be an array")
            })?;
            let alias_args: Vec<String> = alias_args
                .iter()
                .map(|value| {
                    value.as_str().map(ToString::to_string).ok_or_else(|| {
                        format!(
                            "shot-all-views manifest target {index} shot_args alias contains a non-string arg"
                        )
                    })
                })
                .collect::<Result<_, _>>()?;
            if alias_args != native_args {
                return Err(format!(
                    "shot-all-views manifest target {index} shot_args alias does not match native_shot_args"
                ));
            }
        }

        for filename in [&native_filename, &hoi4_reference_filename, &diff_filename] {
            let collision_key = filename.to_ascii_lowercase();
            if !seen_filenames.insert(collision_key) {
                return Err(format!(
                    "shot-all-views manifest duplicate filename '{filename}'"
                ));
            }
        }

        targets.push(ManifestShotTarget {
            view_id,
            case_id,
            case_family,
            case_label,
            native_filename,
            hoi4_reference_filename,
            diff_filename,
            native_args,
        });
    }
    let base_case_count = targets
        .iter()
        .filter(|target| target.case_id == BASE_SHOT_CASE_ID)
        .count();
    let extra_case_count = targets.len().saturating_sub(base_case_count);
    if declared_registry_view_count != base_case_count {
        return Err(format!(
            "shot-all-views manifest '{}' registry_view_count={declared_registry_view_count} does not match base case count {base_case_count}",
            path.display()
        ));
    }
    if declared_extra_case_count != extra_case_count {
        return Err(format!(
            "shot-all-views manifest '{}' extra_case_count={declared_extra_case_count} does not match expanded case count {extra_case_count}",
            path.display()
        ));
    }
    let target_cases: BTreeSet<(String, String)> = targets
        .iter()
        .map(|target| (target.view_id.clone(), target.case_id.clone()))
        .collect();
    for target in &targets {
        if target.case_id != BASE_SHOT_CASE_ID
            && !target_cases.contains(&(target.view_id.clone(), BASE_SHOT_CASE_ID.to_string()))
        {
            return Err(format!(
                "shot-all-views manifest '{}' expanded case '{}' for view id '{}' is missing its base case",
                path.display(),
                target.case_id,
                target.view_id
            ));
        }
    }
    for target in targets
        .iter()
        .filter(|target| target.case_id == BASE_SHOT_CASE_ID)
    {
        for expected_case in shot_cases_for_view_id(&target.view_id) {
            if !target_cases.contains(&(target.view_id.clone(), expected_case.id.to_string())) {
                return Err(format!(
                    "shot-all-views manifest '{}' missing deterministic case '{}' for view id '{}'",
                    path.display(),
                    expected_case.id,
                    target.view_id
                ));
            }
        }
    }

    Ok(ShotManifestRead {
        path: path.to_path_buf(),
        target_count: targets.len(),
        targets,
    })
}

pub fn build_native_shot_invocations(
    manifest: &ShotManifestRead,
    config: &NativeShotBatchConfig,
) -> Result<Vec<NativeShotInvocation>, String> {
    if config.command.as_os_str().is_empty() {
        return Err("--shot-all-views-command requires a non-empty command path".to_string());
    }
    let manifest_dir = manifest.path.parent().ok_or_else(|| {
        format!(
            "shot-all-views manifest '{}' has no parent directory",
            manifest.path.display()
        )
    })?;
    if !manifest_dir.is_dir() {
        return Err(format!(
            "shot-all-views manifest directory '{}' must exist",
            manifest_dir.display()
        ));
    }
    if manifest.targets.is_empty() {
        return Err("shot-all-views native batch has no manifest targets".to_string());
    }
    if let Some(flag) = config
        .common_args
        .iter()
        .find_map(|arg| target_owned_shot_arg(arg))
    {
        return Err(format!(
            "shot-all-views common args cannot include target-owned {flag}"
        ));
    }

    Ok(manifest
        .targets
        .iter()
        .map(|target| {
            let mut args = config.common_args.clone();
            args.extend(target.native_args.clone());
            NativeShotInvocation {
                view_id: target.view_id.clone(),
                command: config.command.clone(),
                current_dir: manifest_dir.to_path_buf(),
                args,
                native_path: manifest_dir.join(&target.native_filename),
                hoi4_reference_path: manifest_dir.join(&target.hoi4_reference_filename),
                diff_path: manifest_dir.join(&target.diff_filename),
            }
        })
        .collect())
}

pub fn run_native_shot_batch_from_manifest(
    manifest_path: &Path,
    config: &NativeShotBatchConfig,
) -> Result<NativeShotBatchRun, String> {
    let manifest = read_shot_manifest(manifest_path)?;
    let invocations = build_native_shot_invocations(&manifest, config)?;
    let mut captured_count = 0usize;

    for invocation in &invocations {
        if invocation.native_path.is_dir() {
            return Err(format!(
                "shot-all-views native target '{}' is an existing directory",
                invocation.native_path.display()
            ));
        }
        let status = Command::new(&invocation.command)
            .current_dir(&invocation.current_dir)
            .args(&invocation.args)
            .status()
            .map_err(|e| {
                format!(
                    "run native shot for view '{}' with command '{}' failed: {e}",
                    invocation.view_id,
                    invocation.command.display()
                )
            })?;
        if !status.success() {
            let code = status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "terminated".to_string());
            return Err(format!(
                "native shot for view '{}' failed with status {code}",
                invocation.view_id
            ));
        }
        if !invocation.native_path.is_file() {
            return Err(format!(
                "native shot for view '{}' completed but did not write '{}'",
                invocation.view_id,
                invocation.native_path.display()
            ));
        }
        captured_count += 1;
    }

    Ok(NativeShotBatchRun {
        manifest_path: manifest.path,
        captured_count,
        pending_hoi4_reference_count: invocations.len(),
        pending_diff_count: invocations.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::host::{HostPolicy, ViewDescriptor, ViewId, ViewInstance, ZBand};
    use std::time::{SystemTime, UNIX_EPOCH};

    const TEST_ALPHA: ViewId = ViewId("test.alpha");
    const TEST_BETA: ViewId = ViewId("test.beta");
    const TEST_BAD: ViewId = ViewId("test/bad");
    const TEST_RESERVED: ViewId = ViewId("test.con");
    const EXPECTED_BASE_REGISTERED_VIEW_COUNT: usize = 114;

    fn unused_open(_arg: Option<i64>) -> Box<dyn ViewInstance> {
        panic!("shot harness tests must not instantiate views")
    }

    static ALPHA_DESC: ViewDescriptor = ViewDescriptor {
        id: TEST_ALPHA,
        gui_root: None,
        required_domains: &[],
        route: None,
        shortcut: None,
        open: unused_open,
        policy: HostPolicy::window(ZBand::Panel),
    };

    static BETA_DESC: ViewDescriptor = ViewDescriptor {
        id: TEST_BETA,
        gui_root: None,
        required_domains: &[],
        route: None,
        shortcut: None,
        open: unused_open,
        policy: HostPolicy::window(ZBand::Panel),
    };

    static BAD_DESC: ViewDescriptor = ViewDescriptor {
        id: TEST_BAD,
        gui_root: None,
        required_domains: &[],
        route: None,
        shortcut: None,
        open: unused_open,
        policy: HostPolicy::window(ZBand::Panel),
    };

    static RESERVED_DESC: ViewDescriptor = ViewDescriptor {
        id: TEST_RESERVED,
        gui_root: None,
        required_domains: &[],
        route: None,
        shortcut: None,
        open: unused_open,
        policy: HostPolicy::window(ZBand::Panel),
    };

    fn test_registry() -> ViewRegistry {
        let mut registry = ViewRegistry::empty();
        registry.insert_view(&BETA_DESC);
        registry.insert_view(&ALPHA_DESC);
        registry
    }

    fn unique_temp_dir_name(test_name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "battle_plan_{test_name}_{}_{}",
            std::process::id(),
            nanos
        ))
    }

    fn manifest_read_error_after_registry_mutation(
        test_name: &str,
        registry: ViewRegistry,
        mutate: impl FnOnce(&mut serde_json::Value),
    ) -> String {
        let out = unique_temp_dir_name(test_name);
        std::fs::create_dir(&out).unwrap();
        let write = write_registry_shot_manifest(&registry, &out).unwrap();
        let raw = std::fs::read(&write.path).unwrap();
        let mut manifest: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        mutate(&mut manifest);
        std::fs::write(&write.path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
        let err = read_shot_manifest(&write.path).unwrap_err();
        std::fs::remove_dir_all(&out).unwrap();
        err
    }

    fn manifest_read_error_after_mutation(
        test_name: &str,
        mutate: impl FnOnce(&mut serde_json::Value),
    ) -> String {
        manifest_read_error_after_registry_mutation(test_name, test_registry(), mutate)
    }

    #[test]
    fn shot_all_views_matrix_is_registry_derived_and_sorted() {
        let out = Path::new("shots");
        let matrix = build_registry_shot_matrix(&test_registry(), out).unwrap();
        assert_eq!(matrix.len(), 2);
        assert_eq!(matrix[0].view_id, "test.alpha");
        assert_eq!(matrix[0].case_id, BASE_SHOT_CASE_ID);
        assert_eq!(matrix[0].case_family, "base");
        assert_eq!(matrix[0].native_filename, "native__test.alpha.png");
        assert_eq!(matrix[0].hoi4_reference_filename, "hoi4__test.alpha.png");
        assert_eq!(matrix[0].diff_filename, "diff__test.alpha.png");
        assert_eq!(matrix[0].native_path, out.join("native__test.alpha.png"));
        assert_eq!(matrix[1].view_id, "test.beta");
        assert_eq!(matrix[1].native_filename, "native__test.beta.png");
        assert_eq!(matrix[1].native_path, out.join("native__test.beta.png"));
    }

    #[test]
    fn shot_all_views_matrix_covers_inventory_without_static_view_list() {
        let registry = ViewRegistry::from_inventory();
        let matrix = build_registry_shot_matrix(&registry, Path::new("shots")).unwrap();
        let base_targets: Vec<_> = matrix
            .iter()
            .filter(|target| target.case_id == BASE_SHOT_CASE_ID)
            .collect();
        let matrix_ids: Vec<_> = base_targets.iter().map(|target| target.view_id).collect();
        let registry_ids: Vec<_> = registry
            .iter_views()
            .map(|descriptor| descriptor.id.as_str())
            .collect();
        assert_eq!(registry.len(), EXPECTED_BASE_REGISTERED_VIEW_COUNT);
        assert_eq!(base_targets.len(), EXPECTED_BASE_REGISTERED_VIEW_COUNT);
        assert_eq!(matrix_ids, registry_ids);
        assert_eq!(
            matrix.len(),
            EXPECTED_BASE_REGISTERED_VIEW_COUNT
                + MAP_MODE_SHOT_CASES.len()
                + STATE_PANEL_SELECTED_STATE_SHOT_CASES.len()
        );
    }

    #[test]
    fn shot_all_views_expands_named_map_mode_cases_deterministically() {
        let registry = ViewRegistry::from_inventory();
        let matrix = build_registry_shot_matrix(&registry, Path::new("shots")).unwrap();
        let map_targets: Vec<_> = matrix
            .iter()
            .filter(|target| target.view_id == MAP_VIEW_ID.as_str())
            .collect();
        let case_ids: Vec<_> = map_targets.iter().map(|target| target.case_id).collect();
        assert_eq!(
            case_ids,
            vec![
                BASE_SHOT_CASE_ID,
                "map_mode_control",
                "map_mode_weather",
                "map_mode_supply"
            ]
        );

        let control = map_targets[1];
        assert_eq!(control.case_family, "map_modes");
        assert_eq!(control.case_label, "Map mode: Control");
        assert_eq!(
            control.native_filename,
            "native__core.map__map_mode_control.png"
        );
        assert_eq!(
            control.hoi4_reference_filename,
            "hoi4__core.map__map_mode_control.png"
        );
        assert_eq!(
            control.diff_filename,
            "diff__core.map__map_mode_control.png"
        );
        assert_eq!(
            control.native_args,
            vec![
                "--shot".to_string(),
                "native__core.map__map_mode_control.png".to_string(),
                "--shot-view".to_string(),
                MAP_VIEW_ID.as_str().to_string(),
                "--map-mode".to_string(),
                "control".to_string(),
            ]
        );
    }

    #[test]
    fn shot_all_views_expands_state_panel_selected_state_case_deterministically() {
        let registry = ViewRegistry::from_inventory();
        let matrix = build_registry_shot_matrix(&registry, Path::new("shots")).unwrap();
        let state_targets: Vec<_> = matrix
            .iter()
            .filter(|target| target.view_id == STATE_PANEL_VIEW_ID.as_str())
            .collect();
        let case_ids: Vec<_> = state_targets.iter().map(|target| target.case_id).collect();
        assert_eq!(case_ids, vec![BASE_SHOT_CASE_ID, "state_panel_state_64"]);

        let selected = state_targets[1];
        assert_eq!(selected.case_family, "state_panel_selected_states");
        assert_eq!(selected.case_label, "State panel: State 64");
        assert_eq!(
            selected.native_filename,
            "native__core.state_panel__state_panel_state_64.png"
        );
        assert_eq!(
            selected.hoi4_reference_filename,
            "hoi4__core.state_panel__state_panel_state_64.png"
        );
        assert_eq!(
            selected.diff_filename,
            "diff__core.state_panel__state_panel_state_64.png"
        );
        assert_eq!(
            selected.native_args,
            vec![
                "--shot".to_string(),
                "native__core.state_panel__state_panel_state_64.png".to_string(),
                "--shot-view".to_string(),
                STATE_PANEL_VIEW_ID.as_str().to_string(),
                "--shot-state".to_string(),
                "64".to_string(),
            ]
        );
    }

    #[test]
    fn shot_all_views_rejects_path_unsafe_view_ids() {
        let mut registry = ViewRegistry::empty();
        registry.insert_view(&BAD_DESC);
        let err = build_registry_shot_matrix(&registry, Path::new("shots")).unwrap_err();
        assert!(err.contains("invalid registered view id 'test/bad'"));
        assert!(err.contains("unsupported path character"));
    }

    #[test]
    fn shot_all_views_rejects_windows_reserved_view_id_segments() {
        let mut registry = ViewRegistry::empty();
        registry.insert_view(&RESERVED_DESC);
        let err = build_registry_shot_matrix(&registry, Path::new("shots")).unwrap_err();
        assert!(err.contains("invalid registered view id 'test.con'"));
        assert!(err.contains("reserved Windows device name"));
    }

    #[test]
    fn shot_all_views_rejects_missing_output_dir() {
        let out = unique_temp_dir_name("missing_shot_dir");
        let err = write_registry_shot_manifest(&test_registry(), &out).unwrap_err();
        assert!(err.contains("--shot-all-views output"));
        assert!(err.contains("must be an existing directory"));
    }

    #[test]
    fn shot_all_views_writes_manifest_for_existing_dir() {
        let out = unique_temp_dir_name("manifest");
        std::fs::create_dir(&out).unwrap();
        let write = write_registry_shot_manifest(&test_registry(), &out).unwrap();
        assert_eq!(write.target_count, 2);
        assert_eq!(write.path, out.join(SHOT_ALL_MANIFEST));

        let raw = std::fs::read(&write.path).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(manifest["version"], 2);
        assert_eq!(manifest["status"], "native_targets_planned");
        assert_eq!(manifest["path_base"], "manifest_dir");
        assert_eq!(manifest["native_args_cwd"], "manifest_dir");
        assert_eq!(
            manifest["native_batch_runner"]["emits"],
            "native__*.png only"
        );
        assert_eq!(manifest["registry_view_count"], 2);
        assert_eq!(manifest["target_count"], 2);
        assert_eq!(manifest["extra_case_count"], 0);
        assert_eq!(manifest["case_expansion"]["status"], "deterministic");
        assert_eq!(
            manifest["case_expansion"]["families"][0]["family"],
            "map_modes"
        );
        assert_eq!(
            manifest["case_expansion"]["families"][1]["family"],
            "state_panel_selected_states"
        );
        assert_eq!(manifest["targets"][0]["view_id"], "test.alpha");
        assert_eq!(manifest["targets"][0]["case_id"], BASE_SHOT_CASE_ID);
        assert_eq!(manifest["targets"][0]["case_family"], "base");
        assert_eq!(manifest["targets"][0]["case_label"], "Base registered view");
        assert_eq!(
            manifest["targets"][0]["native_shot_file"],
            "native__test.alpha.png"
        );
        assert_eq!(
            manifest["targets"][0]["native_status"],
            "pending_native_capture"
        );
        assert_eq!(
            manifest["targets"][0]["hoi4_reference_file"],
            "hoi4__test.alpha.png"
        );
        assert_eq!(
            manifest["targets"][0]["hoi4_reference_status"],
            "pending_external_reference_capture"
        );
        assert_eq!(manifest["targets"][0]["diff_file"], "diff__test.alpha.png");
        assert_eq!(
            manifest["targets"][0]["diff_status"],
            "pending_external_comparison"
        );
        assert_eq!(
            manifest["targets"][0]["native_shot_args"],
            serde_json::json!([
                "--shot",
                "native__test.alpha.png",
                "--shot-view",
                "test.alpha"
            ])
        );
        assert_eq!(manifest["targets"][1]["view_id"], "test.beta");
        assert_eq!(manifest["targets"][1]["shot_file"], "native__test.beta.png");

        std::fs::remove_dir_all(&out).unwrap();
    }

    #[test]
    fn shot_all_views_manifest_reader_rejects_wrong_version_and_counts() {
        let err = manifest_read_error_after_mutation("manifest_bad_version", |manifest| {
            manifest["version"] = serde_json::json!(1);
        });
        assert!(err.contains("unsupported version 1"));

        let err = manifest_read_error_after_mutation("manifest_bad_target_count", |manifest| {
            manifest["target_count"] = serde_json::json!(999);
        });
        assert!(err.contains("target_count=999 does not match targets array length 2"));

        let err = manifest_read_error_after_mutation("manifest_bad_registry_count", |manifest| {
            manifest["registry_view_count"] = serde_json::json!(1);
        });
        assert!(err.contains("registry_view_count=1 does not match base case count 2"));

        let err = manifest_read_error_after_mutation("manifest_bad_extra_count", |manifest| {
            manifest["extra_case_count"] = serde_json::json!(1);
        });
        assert!(err.contains("extra_case_count=1 does not match expanded case count 0"));
    }

    #[test]
    fn shot_all_views_manifest_reader_rejects_bad_case_metadata_and_args() {
        let err = manifest_read_error_after_mutation("manifest_bad_case", |manifest| {
            manifest["targets"][0]["case_id"] = serde_json::json!("map_mode_control");
        });
        assert!(err.contains("unsupported case 'map_mode_control' for view id 'test.alpha'"));

        let err = manifest_read_error_after_mutation("manifest_bad_case_meta", |manifest| {
            manifest["targets"][0]["case_label"] = serde_json::json!("Wrong label");
        });
        assert!(err.contains("case metadata does not match view id 'test.alpha' case 'base'"));

        let err = manifest_read_error_after_mutation("manifest_bad_case_args", |manifest| {
            manifest["targets"][0]["native_shot_args"] = serde_json::json!([
                "--shot",
                "native__test.alpha.png",
                "--shot-view",
                "test.alpha",
                "--map-mode",
                "control"
            ]);
        });
        assert!(err.contains("native_shot_args do not match view id 'test.alpha' case 'base'"));
    }

    #[test]
    fn shot_all_views_manifest_reader_rejects_incomplete_or_tampered_state_panel_cases() {
        let err = manifest_read_error_after_registry_mutation(
            "manifest_missing_state_panel_case",
            ViewRegistry::from_inventory(),
            |manifest| {
                let targets = manifest["targets"].as_array_mut().unwrap();
                let index = targets
                    .iter()
                    .position(|target| target["case_id"] == "state_panel_state_64")
                    .expect("state panel selected-state case");
                targets.remove(index);
                manifest["target_count"] = serde_json::json!(targets.len());
                manifest["extra_case_count"] = serde_json::json!(
                    MAP_MODE_SHOT_CASES.len() + STATE_PANEL_SELECTED_STATE_SHOT_CASES.len() - 1
                );
            },
        );
        assert!(err.contains("missing deterministic case 'state_panel_state_64'"));

        let err = manifest_read_error_after_registry_mutation(
            "manifest_bad_state_panel_case_args",
            ViewRegistry::from_inventory(),
            |manifest| {
                let targets = manifest["targets"].as_array_mut().unwrap();
                let target = targets
                    .iter_mut()
                    .find(|target| target["case_id"] == "state_panel_state_64")
                    .expect("state panel selected-state case");
                target["native_shot_args"] = serde_json::json!([
                    "--shot",
                    "native__core.state_panel__state_panel_state_64.png",
                    "--shot-view",
                    "core.state_panel",
                    "--shot-state",
                    "1"
                ]);
            },
        );
        assert!(err.contains(
            "native_shot_args do not match view id 'core.state_panel' case 'state_panel_state_64'"
        ));
    }

    #[test]
    fn shot_all_views_native_batch_rejects_target_owned_common_args() {
        let out = unique_temp_dir_name("batch_conflict");
        std::fs::create_dir(&out).unwrap();
        let write = write_registry_shot_manifest(&test_registry(), &out).unwrap();
        let manifest = read_shot_manifest(&write.path).unwrap();
        let config = NativeShotBatchConfig {
            command: PathBuf::from("replay_app"),
            common_args: vec!["--map-mode".to_string(), "supply".to_string()],
        };

        let err = build_native_shot_invocations(&manifest, &config).unwrap_err();
        assert_eq!(
            err,
            "shot-all-views common args cannot include target-owned --map-mode"
        );

        let config = NativeShotBatchConfig {
            command: PathBuf::from("replay_app"),
            common_args: vec!["--shot-state=64".to_string()],
        };
        let err = build_native_shot_invocations(&manifest, &config).unwrap_err();
        assert_eq!(
            err,
            "shot-all-views common args cannot include target-owned --shot-state"
        );

        std::fs::remove_dir_all(&out).unwrap();
    }

    #[test]
    fn shot_all_views_builds_native_batch_invocations_from_manifest() {
        let out = unique_temp_dir_name("batch_plan");
        std::fs::create_dir(&out).unwrap();
        let write = write_registry_shot_manifest(&test_registry(), &out).unwrap();
        let manifest = read_shot_manifest(&write.path).unwrap();
        let config = NativeShotBatchConfig {
            command: PathBuf::from("replay_app"),
            common_args: vec![
                "--country".to_string(),
                "GER".to_string(),
                "--game".to_string(),
                "campaign".to_string(),
            ],
        };

        let invocations = build_native_shot_invocations(&manifest, &config).unwrap();

        assert_eq!(manifest.target_count, 2);
        assert_eq!(invocations.len(), 2);
        assert_eq!(invocations[0].view_id, "test.alpha");
        assert_eq!(invocations[0].command, PathBuf::from("replay_app"));
        assert_eq!(invocations[0].current_dir, out);
        assert_eq!(
            invocations[0].native_path,
            out.join("native__test.alpha.png")
        );
        assert_eq!(
            invocations[0].hoi4_reference_path,
            out.join("hoi4__test.alpha.png")
        );
        assert_eq!(invocations[0].diff_path, out.join("diff__test.alpha.png"));
        assert_eq!(
            invocations[0].args,
            vec![
                "--country".to_string(),
                "GER".to_string(),
                "--game".to_string(),
                "campaign".to_string(),
                "--shot".to_string(),
                "native__test.alpha.png".to_string(),
                "--shot-view".to_string(),
                "test.alpha".to_string(),
            ]
        );
        assert!(
            !invocations[0]
                .args
                .iter()
                .any(|arg| arg.starts_with("hoi4__") || arg.starts_with("diff__")),
            "native batch must preserve HOI4/diff names as metadata, not child outputs"
        );

        std::fs::remove_dir_all(&out).unwrap();
    }

    #[test]
    fn shot_all_views_state_panel_target_has_default_state_arg() {
        let registry = ViewRegistry::from_inventory();
        let matrix = build_registry_shot_matrix(&registry, Path::new("shots")).unwrap();
        let target = matrix
            .iter()
            .find(|target| target.view_id == STATE_PANEL_VIEW_ID.as_str())
            .expect("state panel target");
        assert_eq!(
            target.native_args,
            vec![
                "--shot".to_string(),
                target.native_filename.clone(),
                "--shot-view".to_string(),
                STATE_PANEL_VIEW_ID.as_str().to_string(),
                "--shot-state".to_string(),
                DEFAULT_STATE_PANEL_SHOT_STATE.to_string(),
            ]
        );
    }
}
