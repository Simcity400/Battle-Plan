//! `stream_fold` — offline bounded-memory streaming fold of a compact record
//! stream into per-game-hour snapshot files.
//!
//! This is the same streaming fold the recorder host runs during finalization
//! ([`recorder_host::stream_fold::fold_compact_stream_file`]), exposed as a CLI so
//! it can (a) PROVE bounded memory over a real multi-hundred-MB stream (it reports
//! its own peak private/working-set bytes at exit), (b) PROVE G1 fold-equivalence
//! — its per-hour `snapshot_hash`es match `fold_check hash` on the same stream
//! when run with the same `--game-id` (default `fold_check`), because both drive
//! the one production fold — and (c) re-fold an old recording's raw stream.
//!
//! Usage:
//!   stream_fold <collector_stream.bin> <out_snapshot_dir> [--game-id ID] [--json report.json]

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use recorder_host::proc_mem::peak_memory;
use recorder_host::stream_fold::fold_compact_stream_file;
use serde_json::{json, Value};
use snapshot::sha256::{to_hex, Sha256};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("stream_fold: error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut positional: Vec<String> = Vec::new();
    let mut game_id = "fold_check".to_string();
    let mut json_out: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--game-id" => {
                i += 1;
                game_id = args.get(i).ok_or("--game-id requires a value")?.clone();
            }
            "--json" => {
                i += 1;
                json_out = Some(args.get(i).ok_or("--json requires a path")?.clone());
            }
            "-h" | "--help" => {
                eprintln!(
                    "stream_fold <collector_stream.bin> <out_snapshot_dir> \
                     [--game-id ID] [--json report.json]"
                );
                return Ok(());
            }
            other if other.starts_with("--") => return Err(format!("unknown flag {other}")),
            other => positional.push(other.to_string()),
        }
        i += 1;
    }
    if positional.len() != 2 {
        return Err("usage: stream_fold <stream.bin> <out_snapshot_dir> [--game-id ID] [--json report.json]".into());
    }
    let stream_path = PathBuf::from(&positional[0]);
    let out_dir = PathBuf::from(&positional[1]);

    let start = std::time::Instant::now();
    let (files, stats) = fold_compact_stream_file(&stream_path, &game_id, &out_dir)
        .map_err(|e| format!("streaming fold {}: {e}", stream_path.display()))?;
    let elapsed = start.elapsed();

    // Hash each spilled snapshot file (streaming) to produce the per-hour hashes
    // that mirror `fold_check hash`'s `snapshot_hash` (identical when game_id matches).
    let mut hours: Vec<Value> = Vec::with_capacity(files.len());
    let mut chain = Sha256::new();
    for f in &files {
        let sha = sha256_file(&f.path).map_err(|e| format!("hash {}: {e}", f.path.display()))?;
        chain.update(f.hour_index.to_le_bytes().as_slice());
        chain.update(sha.as_bytes());
        chain.update(b"\n");
        hours.push(json!({
            "hour_index": f.hour_index,
            "date": f.date,
            "snapshot_hash": sha,
            "snapshot_bytes": f.byte_len,
        }));
    }
    let chain_hash = to_hex(&chain.finalize());

    let peak = peak_memory();
    let report = json!({
        "tool": "stream_fold",
        "input": stream_path.display().to_string(),
        "game_id": game_id,
        "totals": {
            "stream_bytes": stats.stream_bytes,
            "bytes_folded": stats.bytes_folded,
            "truncated_tail_bytes": stats.truncated_tail_bytes,
            "records_total": stats.records_total,
            "data_records": stats.data_records,
            "frame_end_records": stats.frame_end_records,
            "perf_records_ignored": stats.perf_records,
            "other_records_ignored": stats.other_records,
            "hours_emitted": stats.hours_emitted,
            "framing_stopped": stats.stop_reason,
        },
        "hour_range": match (files.first(), files.last()) {
            (Some(a), Some(b)) => json!([a.hour_index, b.hour_index]),
            _ => Value::Null,
        },
        "chain_hash": chain_hash,
        "fold_seconds": elapsed.as_secs_f64(),
        "peak_private_bytes": peak.map(|p| p.peak_private_bytes),
        "peak_working_set_bytes": peak.map(|p| p.peak_working_set_bytes),
        "hours": hours,
    });

    let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())? + "\n";
    match &json_out {
        Some(path) => File::create(path)
            .and_then(|mut f| f.write_all(text.as_bytes()))
            .map_err(|e| format!("write {path}: {e}"))?,
        None => print!("{text}"),
    }
    Ok(())
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(&hasher.finalize()))
}
