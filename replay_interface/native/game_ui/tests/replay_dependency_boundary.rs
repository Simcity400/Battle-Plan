use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn normalized(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

#[test]
fn replay_workspace_cargo_metadata_has_zero_extractor_dependencies() {
    let game_ui = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = game_ui.parent().expect("replay workspace");
    let repo = workspace
        .parent()
        .and_then(Path::parent)
        .expect("repository root");
    let extractor_prefix = format!("{}/extractor/", normalized(repo).trim_end_matches('/'));
    let output = Command::new(env!("CARGO"))
        .current_dir(workspace)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--locked",
            "--offline",
        ])
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).expect("parse cargo metadata");
    let packages = metadata["packages"].as_array().expect("metadata packages");
    let mut offenders = packages
        .iter()
        .filter_map(|package| package["manifest_path"].as_str())
        .filter(|manifest| normalized(Path::new(manifest)).starts_with(&extractor_prefix))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for package in packages {
        for dependency in package["dependencies"].as_array().into_iter().flatten() {
            if let Some(path) = dependency["path"].as_str() {
                if normalized(Path::new(path)).starts_with(&extractor_prefix) {
                    offenders.push(path.to_owned());
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "replay workspace depends on extractor manifests: {offenders:?}"
    );
}
