use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn normalized(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn replay_workspace_metadata() -> Value {
    let game_ui = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = game_ui.parent().expect("replay workspace");
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
    serde_json::from_slice(&output.stdout).expect("parse cargo metadata")
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
    let metadata = replay_workspace_metadata();
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

/// The positive statement of the same boundary: no crate here holds a live-code
/// patching primitive.
///
/// The path check above is necessary but not sufficient, because it only says
/// where a manifest lives. `terrain_capture` — the injected D3D11 DLL that
/// captures the game's map-frame bundles — sat in this workspace until it was moved
/// to `extractor/native/terrain_capture`, and it reached its hooking primitive
/// two ways at once: `inline_detour` by path (which the check above catches) and
/// `minhook` straight from crates.io (which it does not, because a registry
/// dependency has no path to inspect). A crate re-added here that took only the
/// registry dependency would satisfy the path check and still be patching live
/// game code from the replay product.
///
/// The replay side consumes sealed captures. Nothing it ships needs to rewrite
/// another process's instructions, so the honest form of the rule is that the
/// primitive is absent, not that one route to it is.
///
/// Checked twice on purpose. The manifest scan names the offending member, but
/// `--no-deps` only reports DECLARED dependencies, so a patching crate arriving
/// transitively through some innocuous wrapper would not appear in it. The
/// lockfile is the resolved closure, so it catches that — at the cost of not
/// being able to say which member pulled it. Neither alone is the rule.
#[test]
fn replay_workspace_declares_no_live_code_patching_dependency() {
    // Crates whose entire purpose is patching a running image. Matched on the
    // stem, so the raw binding layer underneath each one — `minhook-sys`,
    // `frida-gum-sys`, and friends — is denied by the same entry. Naming only
    // the safe wrappers would leave the obvious bypass open: the `-sys` crate
    // exposes the same patching entry points and takes no extra dependency to
    // reach.
    const PATCHING_CRATES: [&str; 4] = ["minhook", "detour", "retour", "frida-gum"];
    let is_patching = |name: &str| {
        let stem = name.strip_suffix("-sys").unwrap_or(name);
        PATCHING_CRATES.contains(&stem)
    };
    let metadata = replay_workspace_metadata();
    let packages = metadata["packages"].as_array().expect("metadata packages");
    let mut offenders = Vec::new();
    for package in packages {
        let member = package["name"].as_str().unwrap_or("<unnamed>");
        for dependency in package["dependencies"].as_array().into_iter().flatten() {
            // `name` is the real crate name even when the dependency is
            // renamed with `package = "..."`, so an alias does not slip past.
            let Some(name) = dependency["name"].as_str() else {
                continue;
            };
            if is_patching(name) {
                offenders.push(format!("declared: {member} -> {name}"));
            }
        }
    }

    let lock = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("replay workspace")
        .join("Cargo.lock");
    let locked = std::fs::read_to_string(&lock).expect("read replay workspace Cargo.lock");
    for line in locked.lines() {
        let Some(name) = line
            .trim()
            .strip_prefix("name = \"")
            .and_then(|rest| rest.strip_suffix('"'))
        else {
            continue;
        };
        if is_patching(name) {
            offenders.push(format!("resolved in Cargo.lock: {name}"));
        }
    }

    assert!(
        offenders.is_empty(),
        "the replay workspace consumes sealed captures and must not carry a \
         live-code patching primitive; acquisition crates belong in \
         extractor/native: {offenders:?}"
    );
}
