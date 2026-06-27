//! m7_modset_demo — the app-level MOD-AWARENESS path end to end (§K):
//!
//!   1. SELECT an active mod-set + its asset BUNDLE (CLI `--bundle <dir>` /
//!      `REPLAY_BUNDLE` env / default `game_assets`), loading that bundle's
//!      `active_modset.json` manifest (or vanilla when absent) — deliverable 1.
//!   2. AUTO-MATCH a replay against the selected bundle: read the replay snapshot's
//!      recorded `provenance.mod`, compare it to the bundle's mod-set, and print a
//!      CLEAR mismatch WARNING when they differ (incl. version / load-order) —
//!      deliverable 2.
//!
//! READ-ONLY; prints a report, writes nothing. The engine is unchanged — this only
//! decides WHICH bundle it would be handed and whether the replay matches it.
//!
//! Args:
//!   --bundle <dir>     asset bundle dir (default: repo `game_assets`)
//!   --snapshot <file>  a replay snapshot JSON to auto-match (optional)
//!   --store <dir> --game <id>  alternatively, pick the FIRST snapshot of a game

use assets::modset::{match_replay, BundleConfig, MatchOutcome, ModSet};
use std::path::{Path, PathBuf};

fn arg_value(flag: &str) -> Option<String> {
    let mut it = std::env::args();
    while let Some(a) = it.next() {
        if a == flag {
            return it.next();
        }
        if let Some(v) = a.strip_prefix(&format!("{flag}=")) {
            return Some(v.to_string());
        }
    }
    None
}

/// Repo root = two levels up from this crate (replay_native/assets -> repo).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Read `provenance.mod` out of a snapshot JSON file (the value, or None/Null ->
/// the replay's mod identity is unknown).
fn read_provenance_mod(path: &Path) -> Option<ModSet> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let pm = v.get("provenance").and_then(|p| p.get("mod"));
    ModSet::from_provenance(pm)
}

/// Pick a snapshot file: an explicit `--snapshot`, else the first `*.snapshot.json`
/// in `<store>/<game>/` (via its index.json `file` field, or a dir scan fallback).
fn pick_snapshot() -> Option<PathBuf> {
    if let Some(s) = arg_value("--snapshot") {
        return Some(PathBuf::from(s));
    }
    let store = arg_value("--store")?;
    let game = arg_value("--game").unwrap_or_else(|| "campaign".to_string());
    let gdir = PathBuf::from(store).join(&game);
    // try index.json first
    if let Ok(txt) = std::fs::read_to_string(gdir.join("index.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
            if let Some(first) = v
                .get("snapshots")
                .and_then(|s| s.as_array())
                .and_then(|a| a.first())
                .and_then(|e| e.get("file"))
                .and_then(|f| f.as_str())
            {
                return Some(gdir.join(first));
            }
        }
    }
    // fallback: first matching file in the dir
    std::fs::read_dir(&gdir).ok()?.flatten().find_map(|e| {
        let p = e.path();
        if p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".snapshot.json"))
            .unwrap_or(false)
        {
            Some(p)
        } else {
            None
        }
    })
}

fn main() {
    // --- 1. SELECT the active bundle + its mod-set -------------------------
    let default_bundle = repo_root().join("game_assets");
    let cfg = BundleConfig::resolve(arg_value("--bundle").as_deref(), &default_bundle)
        .unwrap_or_else(|e| {
            eprintln!("[m7] BLOCKER: could not load bundle manifest: {e}");
            std::process::exit(2);
        });

    println!("[m7] active bundle dir   = {}", cfg.bundle_dir.display());
    println!(
        "[m7] active mod-set      = {} ({} mod(s), id={}, manifest_present={})",
        if cfg.modset.is_vanilla() {
            "VANILLA".to_string()
        } else {
            cfg.modset
                .mods
                .iter()
                .map(|m| m.label())
                .collect::<Vec<_>>()
                .join(", ")
        },
        cfg.modset.mods.len(),
        cfg.modset.resolved_id(),
        cfg.manifest_present
    );

    // --- 2. AUTO-MATCH a replay against the selected bundle ----------------
    let snap = pick_snapshot();
    let snap = match snap {
        Some(p) => p,
        None => {
            println!(
                "[m7] no replay snapshot given (pass --snapshot <file> or --store <dir> --game <id>); \
                 mod-selection report only."
            );
            return;
        }
    };
    println!("[m7] replay snapshot     = {}", snap.display());

    let replay_mod = read_provenance_mod(&snap);
    match &replay_mod {
        None => println!("[m7] replay provenance.mod = <absent/null> (mod identity unknown)"),
        Some(ms) => println!(
            "[m7] replay mod-set      = {} (id={})",
            if ms.is_vanilla() {
                "VANILLA".to_string()
            } else {
                ms.mods
                    .iter()
                    .map(|m| m.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            ms.resolved_id()
        ),
    }

    let outcome = match_replay(replay_mod.as_ref(), &cfg.modset);
    match &outcome {
        MatchOutcome::Match => {
            println!("[m7] AUTO-MATCH: OK — the selected mod-set matches the replay.");
        }
        MatchOutcome::ReplayUnknown => {
            println!("[m7] AUTO-MATCH: UNKNOWN");
            println!("[m7] ADVISORY: {}", outcome.warning().unwrap());
        }
        MatchOutcome::Mismatch { .. } => {
            println!("[m7] AUTO-MATCH: MISMATCH");
            println!("[m7] WARNING: {}", outcome.warning().unwrap());
            // a real app would refuse to silently render the wrong skin; exit nonzero
            // so a scripted check catches it.
            std::process::exit(1);
        }
    }
}
