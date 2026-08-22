use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const RECORDING_ROOTS: [&str; 2] = ["collector", "recorder_host"];

const CONSUMER_ONLY: &[(&str, &str)] = &[];

const DISCOVERY_CONTROL: &[&str] = &["wire_product", "scientists_live_lane"];

const NEEDLE: &str = "derive_panel_stats";

const POSITIVE_CONTROL: &str = "compact_contract";

const MIN_EXPECTED_SCANNED_CRATES: usize = 25;

#[test]
fn no_production_caller_derives_panel_stats() {
    let native = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compact_contract sits under extractor/native")
        .to_path_buf();

    let consumer_only: BTreeSet<&str> = CONSUMER_ONLY.iter().map(|(name, _)| *name).collect();
    let mut crate_dirs: BTreeMap<String, PathBuf> = BTreeMap::new();

    for root in RECORDING_ROOTS {
        let dir = native.join(root);
        assert!(dir.is_dir(), "recording root {} not found", dir.display());
        crate_dirs.insert(root.to_string(), dir);
    }

    let lane_root = native.join("lanes");
    assert!(lane_root.is_dir(), "lanes/ not found — scan target moved");
    let mut lanes = 0usize;
    for entry in fs::read_dir(&lane_root).expect("lanes/ must be readable") {
        let lane = entry.expect("lane entry must be readable").path();
        if lane.join("src").is_dir() {
            let name = lane
                .file_name()
                .and_then(|n| n.to_str())
                .expect("lane name")
                .to_string();
            crate_dirs.insert(name, lane);
            lanes += 1;
        }
    }
    assert!(
        lanes >= 20,
        "found only {lanes} lane crates — the lane glob is not matching"
    );

    let mut discovered: BTreeSet<String> = BTreeSet::new();
    for root in RECORDING_ROOTS {
        let manifest = native.join(root).join("Cargo.toml");
        let text = fs::read_to_string(&manifest).expect("recording root manifest must be readable");
        for dep in local_path_dependencies(&text) {
            let Ok(dir) = native.join(root).join(&dep).canonicalize() else {
                continue;
            };
            if !dir.join("src").is_dir() {
                continue;
            }
            let Some(name) = dir.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
                continue;
            };
            discovered.insert(name.clone());
            if name == "compact_contract" {
                continue;
            }
            if consumer_only.contains(name.as_str()) {
                continue;
            }
            crate_dirs.insert(name, dir);
        }
    }

    for required in DISCOVERY_CONTROL {
        assert!(
            discovered.contains(*required),
            "dependency discovery did not find {required} — the Cargo.toml parse is broken,              so the scanned surface is smaller than it appears"
        );
        assert!(
            crate_dirs.contains_key(*required) || consumer_only.contains(*required),
            "{required} was discovered but is neither scanned nor classified consumer-only"
        );
    }

    for (name, _) in CONSUMER_ONLY {
        assert!(
            discovered.contains(*name),
            "CONSUMER_ONLY lists {name}, which is not a local dependency of any              recording root — remove the stale exemption"
        );
    }

    assert!(
        crate_dirs.len() >= MIN_EXPECTED_SCANNED_CRATES,
        "only {} crates in the recording surface — the discovery is not working",
        crate_dirs.len()
    );

    let mut scanned = 0usize;
    let mut control_hits = 0usize;
    let mut violations = Vec::new();

    for (name, dir) in &crate_dirs {
        let src = dir.join("src");
        if !src.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        collect_rust_sources(&src, &mut files);
        assert!(
            !files.is_empty(),
            "no Rust sources under {} ({name}) — the walk is not matching",
            src.display()
        );
        for path in files {
            let source = fs::read_to_string(&path).expect("Rust source must be readable");
            scanned += 1;
            if source.contains(POSITIVE_CONTROL) {
                control_hits += 1;
            }
            for (index, line) in source.lines().enumerate() {
                let code = line.split_once("//").map_or(line, |(code, _)| code);
                if code.contains(NEEDLE) {
                    violations.push(format!(
                        "{}:{}: {}",
                        path.strip_prefix(&native).unwrap_or(&path).display(),
                        index + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        scanned > 0 && control_hits > 0,
        "scan is not functioning: {scanned} files read, {control_hits} matched the positive control"
    );

    assert!(
        violations.is_empty(),
        "the recording path must not derive the displayed land-combat values; \
         emit the raw engine fields and let the replay side derive:\n{}",
        violations.join("\n")
    );
}

fn local_path_dependencies(manifest: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in manifest.lines() {
        let line = line.split_once('#').map_or(line, |(code, _)| code);
        let Some(rest) = line.split_once("path").map(|(_, rest)| rest) else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('"') else {
            continue;
        };
        if let Some((value, _)) = rest.split_once('"') {
            if !value.is_empty() {
                out.push(value.to_string());
            }
        }
    }
    out
}

fn collect_rust_sources(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("source directory must be readable") {
        let path = entry.expect("directory entry must be readable").path();
        if path.is_dir() {
            collect_rust_sources(&path, files);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}
