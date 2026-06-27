//! `pack_replay` — reference CLI to PACK a directory snapshot store into a
//! `.battleplan`, and to VERIFY a `.battleplan` round-trips byte-identically.
//!
//! This is the REFERENCE (and a real, usable) implementation of the finalize
//! step; the production streaming writer is the Python hand-off
//! (`replay_ui/WRITER_INTEGRATION_PLAN.md`). It is loss-less by construction:
//! payload files are read + stored as RAW BYTES, never re-serialized.
//!
//! Usage:
//!   pack_replay pack   <store_root> <game_id> <out.battleplan>
//!   pack_replay verify <in.battleplan> [<store_root> <game_id>]   # byte-identity check
//!
//! `pack` reads the store's `index.json`, packs every `index.snapshots[*].file`
//! (+ index + timeline if present), and stamps build/mod identity FROM THE FIRST
//! SNAPSHOT's provenance (so the container's identity == what was captured).

use serde_json::Value;
use snapshot::{pack, Container, ExpectedIdentity, PackEntry, PackIdentity};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rc = match args.get(1).map(String::as_str) {
        Some("pack") => cmd_pack(&args),
        Some("verify") => cmd_verify(&args),
        _ => {
            eprintln!(
                "usage:\n  pack_replay pack   <store_root> <game_id> <out.battleplan>\n  \
                 pack_replay verify <in.battleplan> [<store_root> <game_id>]"
            );
            2
        }
    };
    exit(rc);
}

fn cmd_pack(args: &[String]) -> i32 {
    let (root, game, out) = match (args.get(2), args.get(3), args.get(4)) {
        (Some(a), Some(b), Some(c)) => (PathBuf::from(a), b.clone(), PathBuf::from(c)),
        _ => {
            eprintln!("pack: need <store_root> <game_id> <out.battleplan>");
            return 2;
        }
    };
    let gdir = root.join(&game);
    let index_path = gdir.join("index.json");
    let index_bytes = match fs::read(&index_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("pack: cannot read {}: {e}", index_path.display());
            return 1;
        }
    };
    let index: Value = serde_json::from_slice(&index_bytes).expect("index json");
    let snaps = index.get("snapshots").and_then(Value::as_array).cloned().unwrap_or_default();

    let mut entries: Vec<PackEntry> = Vec::new();
    entries.push(PackEntry {
        path: format!("payload/{game}/index.json"),
        bytes: index_bytes,
        role: "index",
        hour_index: None,
    });

    let mut identity = PackIdentity {
        created_utc: Some(now_utc()),
        ..Default::default()
    };
    let mut took_identity = false;

    for s in &snaps {
        let file = match s.get("file").and_then(Value::as_str) {
            Some(f) => f,
            None => continue,
        };
        let hour = s.get("hour_index").and_then(Value::as_i64);
        let p = gdir.join(file);
        let bytes = match fs::read(&p) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("pack: missing snapshot {}: {e}", p.display());
                return 1;
            }
        };
        // pull identity from the FIRST snapshot's provenance/header (once)
        if !took_identity {
            if let Ok(snap) = serde_json::from_slice::<Value>(&bytes) {
                identity.schema_version = snap
                    .get("schema_version")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                identity.mod_identity = snap
                    .get("provenance")
                    .and_then(|pr| pr.get("mod"))
                    .cloned()
                    .filter(|v| !v.is_null());
                // build exe hash, if the capture recorded it in provenance.memory
                identity.build_exe_sha256 = snap
                    .get("provenance")
                    .and_then(|pr| pr.get("memory"))
                    .and_then(|m| m.get("exe_sha256"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            took_identity = true;
        }
        entries.push(PackEntry {
            path: format!("payload/{game}/{file}"),
            bytes,
            role: "snapshot",
            hour_index: hour,
        });
    }

    // optional timeline.json
    let tpath = gdir.join("timeline.json");
    if let Ok(tb) = fs::read(&tpath) {
        entries.push(PackEntry {
            path: format!("payload/{game}/timeline.json"),
            bytes: tb,
            role: "timeline",
            hour_index: None,
        });
    }

    let container = match pack(&game, &entries, &identity, true) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("pack: {e}");
            return 1;
        }
    };
    // atomic finalize: write to .tmp then rename (spec §6)
    let tmp = out.with_extension("hoi4replay.tmp");
    if let Err(e) = fs::write(&tmp, &container) {
        eprintln!("pack: write {}: {e}", tmp.display());
        return 1;
    }
    if let Err(e) = fs::rename(&tmp, &out) {
        eprintln!("pack: finalize rename: {e}");
        return 1;
    }
    eprintln!(
        "packed {} snapshots -> {} ({} bytes)",
        entries.iter().filter(|e| e.role == "snapshot").count(),
        out.display(),
        container.len()
    );
    0
}

fn cmd_verify(args: &[String]) -> i32 {
    let infile = match args.get(2) {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("verify: need <in.battleplan>");
            return 2;
        }
    };
    let c = match Container::open(&infile, &ExpectedIdentity::default()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("verify: open failed: {e}");
            return 1;
        }
    };
    if let Err(e) = c.verify_all() {
        eprintln!("verify: integrity FAILED: {e}");
        return 1;
    }
    eprintln!(
        "verify: OK — game={} snapshots={} signed={} unsigned={}",
        c.game_id(),
        c.index().snapshots.len(),
        c.signed,
        c.unsigned
    );

    // optional byte-identity check against the source store
    if let (Some(root), Some(game)) = (args.get(3), args.get(4)) {
        let gdir = Path::new(root).join(game);
        for s in &c.index().snapshots {
            let orig = gdir.join(&s.file);
            let orig_bytes = match fs::read(&orig) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("verify: cannot read source {}: {e}", orig.display());
                    return 1;
                }
            };
            // re-derive the container's stored bytes by hashing both sides; the
            // reader already guarantees the served bytes == manifest hash, so equal
            // sha over the source proves byte-identity.
            let want = snapshot::sha256::Sha256::hex(&orig_bytes);
            // find the manifest entry hash for this file
            let path = format!("payload/{game}/{}", s.file);
            let got = c
                .manifest()
                .get("entries")
                .and_then(Value::as_array)
                .and_then(|es| {
                    es.iter().find(|e| e.get("path").and_then(Value::as_str) == Some(&path))
                })
                .and_then(|e| e.get("sha256").and_then(Value::as_str))
                .unwrap_or("<none>");
            if got != want {
                eprintln!(
                    "verify: BYTE-IDENTITY MISMATCH for {}: source {} != container {}",
                    s.file, want, got
                );
                return 1;
            }
        }
        eprintln!("verify: byte-identity OK for all {} snapshots", c.index().snapshots.len());
    }
    0
}

fn now_utc() -> String {
    // dependency-free UTC stamp: seconds since epoch -> RFC3339-ish. We avoid
    // pulling chrono into this crate; the exact wall time is informational.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("epoch:{secs}")
}
