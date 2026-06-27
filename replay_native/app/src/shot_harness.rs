use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use app::host::ViewRegistry;
use app::views::state_panel::STATE_PANEL_VIEW_ID;

pub const SHOT_ALL_MANIFEST: &str = "shot_manifest.json";
pub const DEFAULT_STATE_PANEL_SHOT_STATE: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotTarget {
    pub view_id: &'static str,
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

pub fn shot_filename_for_view_id(id: &str) -> Result<String, String> {
    validate_view_id_for_filename(id)?;
    Ok(format!("native__{id}.png"))
}

fn paired_filenames_for_view_id(id: &str) -> Result<(String, String, String), String> {
    validate_view_id_for_filename(id)?;
    Ok((
        shot_filename_for_view_id(id)?,
        format!("hoi4__{id}.png"),
        format!("diff__{id}.png"),
    ))
}

fn native_args_for_view_id(id: &str, native_filename: &str) -> Vec<String> {
    let mut args = vec![
        "--shot".to_string(),
        native_filename.to_string(),
        "--shot-view".to_string(),
        id.to_string(),
    ];
    if id == STATE_PANEL_VIEW_ID.as_str() {
        args.push("--shot-state".to_string());
        args.push(DEFAULT_STATE_PANEL_SHOT_STATE.to_string());
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
        let (native_filename, hoi4_reference_filename, diff_filename) =
            paired_filenames_for_view_id(view_id)?;
        for filename in [&native_filename, &hoi4_reference_filename, &diff_filename] {
            let collision_key = filename.to_ascii_lowercase();
            if !seen_filenames.insert(collision_key) {
                return Err(format!(
                    "duplicate screenshot target filename '{filename}' from registered view id '{view_id}'"
                ));
            }
        }
        let native_args = native_args_for_view_id(view_id, &native_filename);
        targets.push(ShotTarget {
            view_id,
            native_path: output_dir.join(&native_filename),
            hoi4_reference_path: output_dir.join(&hoi4_reference_filename),
            diff_path: output_dir.join(&diff_filename),
            native_filename,
            hoi4_reference_filename,
            diff_filename,
            native_args,
        });
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
                "native_shot_file": target.native_filename,
                "hoi4_reference_file": target.hoi4_reference_filename,
                "diff_file": target.diff_filename,
                "shot_file": target.native_filename,
                "native_shot_args": target.native_args,
                "shot_args": target.native_args,
            })
        })
        .collect();
    let manifest = serde_json::json!({
        "version": 1,
        "kind": "shot_all_views_manifest",
        "status": "planned_targets_only",
        "note": "--shot-all-views records registry-derived screenshot targets; it does not emit images or prove side-by-side the game parity.",
        "path_base": "manifest_dir",
        "native_args_cwd": "manifest_dir",
        "registry_view_count": targets.len(),
        "manifest_file": SHOT_ALL_MANIFEST,
        "default_state_panel_shot_state": DEFAULT_STATE_PANEL_SHOT_STATE,
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

#[cfg(test)]
mod tests {
    use super::*;
    use app::host::{HostPolicy, ViewDescriptor, ViewId, ViewInstance, ZBand};
    use std::time::{SystemTime, UNIX_EPOCH};

    const TEST_ALPHA: ViewId = ViewId("test.alpha");
    const TEST_BETA: ViewId = ViewId("test.beta");
    const TEST_BAD: ViewId = ViewId("test/bad");
    const TEST_RESERVED: ViewId = ViewId("test.con");

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

    #[test]
    fn shot_all_views_matrix_is_registry_derived_and_sorted() {
        let out = Path::new("shots");
        let matrix = build_registry_shot_matrix(&test_registry(), out).unwrap();
        assert_eq!(matrix.len(), 2);
        assert_eq!(matrix[0].view_id, "test.alpha");
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
        let matrix_ids: Vec<_> = matrix.iter().map(|target| target.view_id).collect();
        let registry_ids: Vec<_> = registry
            .iter_views()
            .map(|descriptor| descriptor.id.as_str())
            .collect();
        assert!(!registry_ids.is_empty());
        assert_eq!(matrix_ids, registry_ids);
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
        assert_eq!(manifest["status"], "planned_targets_only");
        assert_eq!(manifest["path_base"], "manifest_dir");
        assert_eq!(manifest["registry_view_count"], 2);
        assert_eq!(manifest["targets"][0]["view_id"], "test.alpha");
        assert_eq!(
            manifest["targets"][0]["native_shot_file"],
            "native__test.alpha.png"
        );
        assert_eq!(
            manifest["targets"][0]["hoi4_reference_file"],
            "hoi4__test.alpha.png"
        );
        assert_eq!(manifest["targets"][0]["diff_file"], "diff__test.alpha.png");
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
