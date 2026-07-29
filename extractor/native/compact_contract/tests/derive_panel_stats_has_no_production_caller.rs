//! `division_combat_stats::derive_panel_stats` is consumer-side only.
//!
//! The owner's ruling is that the extractor emits RAW engine fields and never
//! computes the four displayed land-combat values: the extractor stays a pure
//! reader, and a capture recorded today stays re-derivable if a game update
//! changes the formula. The derivation lives beside the carrier so the formula
//! and the fields cannot drift apart, which puts it one `use` away from the
//! recording path. This is the executable gate on the claim in its doc comment.
//!
//! # Why the scope is derived rather than listed
//!
//! Two earlier versions of this gate were too narrow, each time in a way that
//! looked complete. The first scanned only `collector/src` and
//! `recorder_host/src`, missing the dozens of `lanes/*` crates the injected
//! collector links. The second added `lanes/*` but still missed
//! `scientists_live_lane` and `wire_product`, which `recorder_host` depends on
//! directly from outside `lanes/` — and `wire_product` is itself the
//! extractor-side production packaging boundary, so that was a live hole.
//!
//! So the scope is no longer a hand-written list of directories. This test reads
//! the local path dependencies of the two recording roots out of their
//! `Cargo.toml` files and DEFAULTS EVERY ONE TO SCANNED. That default is the
//! safety property: an unrecognised dependency is treated as recording-side, so
//! the failure mode of not knowing about a crate is scanning it unnecessarily,
//! never leaving it unscanned. A new local dependency is therefore covered the
//! moment it is added, with no list to update.
//!
//! The single escape hatch is [`CONSUMER_ONLY`], which must name a crate that is
//! actually reachable — a stale exemption fails the test rather than quietly
//! excusing nothing.
//!
//! An earlier revision claimed instead that an unclassified dependency FAILS
//! until someone classifies it, and asserted as much. That assertion was
//! tautological: the code inserts each discovered dependency into the scanned set
//! before computing the "unclassified" difference, so the difference is empty by
//! construction and removing the assertion changed nothing. Default-to-scanned is
//! what the code actually does, it is the fail-safe direction for this rule, and
//! an explicit-classification policy would add a list to maintain without adding
//! any safety — so the claim is corrected rather than the behaviour.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The two roots that define the recording surface: the injected collector and
/// the host that drives it.
const RECORDING_ROOTS: [&str; 2] = ["collector", "recorder_host"];

/// The ONLY escape hatch from the default-to-scanned rule: local crates reachable
/// from a recording root that are nevertheless consumer side, each with its
/// reason. A crate here is not scanned, because calling the helper there is
/// legitimate.
///
/// Keep this short and justified. Every entry must be reachable from a recording
/// root or the test fails, so the list cannot accumulate dead exemptions. Empty
/// today: nothing reachable from the recording roots is a consumer surface.
const CONSUMER_ONLY: &[(&str, &str)] = &[];

/// Crates the dependency walk MUST find, as a positive control on the parse
/// itself. Both are local dependencies of `recorder_host` from outside `lanes/`,
/// and both were invisible to an earlier version of this gate.
const DISCOVERY_CONTROL: &[&str] = &["wire_product", "scientists_live_lane"];

const NEEDLE: &str = "derive_panel_stats";

/// A symbol the scanned crates demonstrably DO reference. Without this the test
/// would pass just as happily if the walk stopped matching files.
const POSITIVE_CONTROL: &str = "compact_contract";

/// Guards against the lane glob silently shrinking.
const MIN_EXPECTED_SCANNED_CRATES: usize = 25;

#[test]
fn no_production_caller_derives_panel_stats() {
    let native = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compact_contract sits under extractor/native")
        .to_path_buf();

    // 1. Collect the recording-side crate set: the two roots, every lanes/*
    //    crate, and every local path dependency of the roots that is not
    //    classified consumer-only.
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

    // Every local path dependency of a recording root is scanned unless it is
    // explicitly CONSUMER_ONLY. Defaulting to scanned is deliberate: not
    // recognising a crate then costs an unnecessary scan rather than leaving a
    // hole. (An earlier revision claimed to fail on "unclassified" dependencies
    // and asserted it, but inserted them into the scanned set first, so the
    // assertion was empty by construction.)
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
                // Defines the helper; scanning it would match the definition.
                continue;
            }
            if consumer_only.contains(name.as_str()) {
                continue;
            }
            crate_dirs.insert(name, dir);
        }
    }

    // POSITIVE CONTROL ON THE DISCOVERY MECHANISM. Without this the whole
    // dependency walk could return nothing and the test would still pass, because
    // the two roots plus 40-odd lanes already exceed the crate-count floor. These
    // two are exactly the crates an earlier version of this gate missed: they are
    // local dependencies of recorder_host from OUTSIDE lanes/, and wire_product is
    // the extractor-side production packaging boundary.
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

    // Every CONSUMER_ONLY entry must actually be reachable, so the list cannot
    // accumulate stale exemptions that quietly excuse nothing.
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

    // 2. Scan every recording-side source file.
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
                // Strip line comments: a comment naming the function is
                // documentation, not a call.
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

    // Positive control: proves the walk and the substring search both work, so a
    // clean result below means "no caller" rather than "nothing was read".
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

/// Pull `path = "..."` values out of a Cargo manifest.
///
/// Deliberately simple line scanning rather than a TOML parse: it only has to
/// find local crate directories. The safety net is not this function being
/// exhaustive — it is [`DISCOVERY_CONTROL`], which fails loudly if the parse
/// stops finding the two crates it is known to need. (An earlier comment here
/// claimed a broken parse would show up as a dropped crate count. That was
/// false: the roots plus the lanes already exceed the floor on their own.)
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
