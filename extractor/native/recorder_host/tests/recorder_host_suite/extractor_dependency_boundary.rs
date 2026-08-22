use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn normalized(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

#[test]
fn extractor_workspace_has_no_replay_interface_path_dependency() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_manifest = manifest_dir.parent().unwrap().join("Cargo.toml");
    let repo = manifest_dir.ancestors().nth(3).unwrap();
    let forbidden = normalized(&repo.join("replay_interface"));
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--locked",
            "--offline",
            "--manifest-path",
        ])
        .arg(&workspace_manifest)
        .output()
        .expect("run cargo metadata for extractor dependency boundary");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    for package in metadata["packages"].as_array().unwrap() {
        let manifest = PathBuf::from(package["manifest_path"].as_str().unwrap());
        assert!(
            !normalized(&manifest).starts_with(&forbidden),
            "extractor workspace includes replay package {}",
            manifest.display()
        );
        for dependency in package["dependencies"].as_array().unwrap() {
            if let Some(path) = dependency["path"].as_str() {
                assert!(
                    !normalized(Path::new(path)).starts_with(&forbidden),
                    "extractor package {} depends on replay path {path}",
                    package["name"].as_str().unwrap()
                );
            }
        }
    }
}
