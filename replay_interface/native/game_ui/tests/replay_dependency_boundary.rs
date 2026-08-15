use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The acquisition tree's directory name, spelled in halves.
///
/// `replay_tree_names_no_extractor_path_outside_prose` scans this file like
/// every other: it has no exemption, because an exemption for the gate's own
/// source is a hole exactly where a reach would be least visible. Splitting the
/// name is what lets the gate state its rule without breaking it. Nothing else
/// in this tree may do that — a name assembled from pieces is exactly the
/// evasion the scan documents as out of its reach — which is why this constant
/// is the only place it happens and everything downstream derives from it.
const ACQUISITION_TREE: &str = concat!("extr", "actor");

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
    let extractor_prefix = format!(
        "{}/{ACQUISITION_TREE}/",
        normalized(repo).trim_end_matches('/')
    );
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
         {ACQUISITION_TREE}/native: {offenders:?}"
    );
}

/// The third route across the boundary: the FILESYSTEM.
///
/// Both tests above read `cargo metadata`. Neither can see a path a test
/// opens, a `.ps1` a script dot-sources, or a directory a python script pushes
/// onto `sys.path`. Three such reaches existed at once and all three were
/// invisible to the gates:
///
/// - `replay_core/src/source.rs` scanned `../../../extractor/native/snapshot/
///   src` as a root of its pathname-capability audit;
/// - `replay_app/Invoke-HeadlessPerformanceAcceptance.ps1` dot-sourced
///   `extractor\native\scripts\ProductionArtifact.Attestation.ps1`;
/// - `re_evidence/.../XN15L/verify_xn15l_live.py` inserted
///   `extractor/native/scripts` into `sys.path`.
///
/// Two live-tooling reaches survived that first pass because relocation alone
/// could not fix them: the regrader above, and `tools/hoi4_re/runtime_ops.py`
/// composing a path to the acquisition side's `winmem`. Both are now routed
/// through `<repo>/tools/hoi4_re`, a facade at the repository root that is owned
/// by neither tree and that delegates to the helpers where they live. The
/// helpers did not move — a live-process memory reader is acquisition code by
/// nature — so the allowlist below carries no live-tooling entry at all.
///
/// So the rule is stated over the text instead: no file the replay side ships,
/// tests, or runs may NAME a path under the acquisition tree outside a
/// whole-line comment. A doc comment that explains where the producer lives is
/// the point of the comment, not a reach; everything else has to be listed in
/// `is_allowed_extractor_path_mention` with a reason. THIS FILE IS SCANNED TOO —
/// there is no exemption for the gate; `ACQUISITION_TREE` is how it names the
/// rule without breaking it.
///
/// # What it does not catch
///
/// A text scan loses to deliberate obfuscation: a name assembled from pieces
/// (`concat!("ex", "tractor", "/x")`), read out of a data file at runtime, or
/// built from an env var. That is accepted. The gate is aimed at the honest
/// reach — someone wiring the replay side to a real path because it was the
/// convenient thing to do — which is what all four recorded reaches were. It is
/// deliberately conservative everywhere else: python triple-quoted strings and
/// Rust attribute lines are SCANNED, not skipped, because both can carry
/// executable paths, and the handful of genuine prose lines caught that way are
/// listed individually instead.
///
/// # What production change would make this fail?
///
/// Re-adding any of the reaches above, in any mechanism; the allowlist is keyed
/// on the exact surviving line, so a moved or edited reach does not match an old
/// entry.
#[test]
fn replay_tree_names_no_extractor_path_outside_prose() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("repository root")
        .to_path_buf();
    // Every top-level subtree of the replay side, each counted separately. A
    // single total would still pass if `tools/` or `game_assets/` silently
    // dropped out of the walk, and two of the reaches this test records live
    // exactly there.
    let subtrees = ["native", "game_assets", "re_evidence", "tools"];
    let mut offenders = Vec::new();
    for subtree in subtrees {
        let root = repo.join("replay_interface").join(subtree);
        let files = scannable_sources(&root);
        assert!(
            files.len() >= 2,
            "replay_interface/{subtree} contributed {} scannable files; the walk is not \
             reaching it",
            files.len()
        );
        for file in files {
            let relative = normalized(file.strip_prefix(&repo).expect("path under repo"));
            let source = fs::read_to_string(&file)
                .unwrap_or_else(|error| panic!("read {}: {error}", file.display()));
            for line in source.lines() {
                let code = strip_prose(&relative, line);
                if !names_extractor_path(code) {
                    continue;
                }
                if is_allowed_extractor_path_mention(&relative, code) {
                    continue;
                }
                offenders.push(format!("{relative}: {code}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the replay side names a path under the acquisition tree in code, which is a \
         filesystem reach across the boundary that `cargo metadata` cannot see. Move the \
         shared thing to portable_replay/ rather than reaching for it, or add the line to \
         is_allowed_extractor_path_mention with a reason: {offenders:#?}"
    );
}

/// A path under the acquisition tree, in any of the spellings this repo uses.
///
/// The separator forms catch a path continuing into the tree. The terminal forms
/// catch one ENDING at the tree root, which a scan for `<name>/` alone reads as
/// clean even though the next line walks down from it. The quoted-segment form
/// catches path COMPOSITION — `workspace / "<name>" / "memory_reader"` in
/// python, `Join-Path $repo '<name>'` in PowerShell — which is how one of the
/// recorded reaches is written.
fn names_extractor_path(code: &str) -> bool {
    [
        format!("{ACQUISITION_TREE}/"),
        format!("{ACQUISITION_TREE}\\"),
        format!("/{ACQUISITION_TREE}"),
        format!("\\{ACQUISITION_TREE}"),
        format!("\"{ACQUISITION_TREE}\""),
        format!("'{ACQUISITION_TREE}'"),
    ]
    .iter()
    .any(|form| code.contains(form))
}

/// Every reach that is allowed to survive, keyed on the exact surviving text.
fn is_allowed_extractor_path_mention(relative: &str, code: &str) -> bool {
    // XUI27 is the external producer dependency ledger's checker, not a Replay
    // product runtime, and it carries one negative assertion that refuses a
    // ledger binding pointing into the acquisition tree.
    //
    // Lane KK (2026-08-15) narrowed this from five admitted spellings to one.
    // The other four -- the validation-wrapper path, a `startswith` prefix
    // test, a nested-path test, and a refusal fixture -- all lived in the
    // replay-owned freeze chain that lane KK deleted, so admitting them now
    // would only hold the door open for a reach nothing needs. Keep it lexical
    // and keep it at one: this exception must shrink with its subject, never
    // outlive it.
    // Keyed on the one file and the one exact line, not on the directory and a
    // substring. `contains` over a whole subtree would admit a real reach that
    // happened to share a line with the negative assertion, and it would admit
    // it from any file added to XUI27 later.
    if relative == "replay_interface/re_evidence/rendering_laws/xui27/external_dependency_ledger.py"
    {
        let exclusion_check = format!("require(\"{ACQUISITION_TREE}/\" not in relative.lower(),");
        return code.trim() == exclusion_check;
    }
    match relative {
        // Module-docstring prose. Python docstrings are scanned rather than
        // skipped (a triple-quoted string can hold executable text), so the
        // handful of genuine prose lines are listed instead.
        "replay_interface/game_assets/portraits.py" => {
            code.trim().starts_with("Companion to extract.py.")
        }
        "replay_interface/game_assets/tests/test_asset_resolution.py" => {
            code.trim().ends_with("against this same module.")
        }
        // A negative assertion: the portable content manifest must NOT carry an
        // acquisition path. Naming the string is how it checks.
        "replay_interface/native/replay_core/src/source.rs" => {
            code.trim() == format!("assert!(!manifest.contains(\"{ACQUISITION_TREE}/\"), \"{{manifest}}\");")
        }
        _ => false,
    }
}

/// Drops whole-line comments, by the comment syntax of the file's own language.
///
/// Rust drops `//` and `///` only — NOT `#`, which begins an attribute, and a
/// `#[path = "../../../<acquisition tree>/..."]` is executable. Nor `*`, which
/// begins a dereference as readily as a block-comment continuation; a block
/// comment that mentions the producer costs one allowlist entry, and getting
/// that wrong the other way costs a missed reach.
///
/// Trailing comments are deliberately NOT stripped: a line that opens a file or
/// dot-sources a script has code before any trailing comment, so nothing can
/// hide behind one.
fn strip_prose<'a>(relative: &str, line: &'a str) -> &'a str {
    let trimmed = line.trim();
    let comment_prefix = if relative.ends_with(".rs") { "//" } else { "#" };
    if trimmed.starts_with(comment_prefix) {
        return "";
    }
    trimmed
}

fn scannable_sources(root: &Path) -> Vec<PathBuf> {
    fn walk(directory: &Path, found: &mut Vec<PathBuf>) {
        // A traversal that swallows its own errors reports "no reaches found"
        // for a directory it could not open.
        let entries = fs::read_dir(directory)
            .unwrap_or_else(|error| panic!("read_dir {}: {error}", directory.display()));
        let mut entries = entries
            .map(|entry| {
                entry.unwrap_or_else(|error| {
                    panic!("enumerate {}: {error}", directory.display())
                })
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(std::fs::DirEntry::path);
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                // Build outputs are not authored source.
                if name == "target" || name.starts_with(".cargo-target") || name == "out" {
                    continue;
                }
                walk(&path, found);
                continue;
            }
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if matches!(extension, "rs" | "ps1" | "py" | "toml") {
                found.push(path);
            }
        }
    }

    let mut found = Vec::new();
    walk(root, &mut found);
    found
}
