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
//!   stream_fold <collector_stream.bin> <out_snapshot_dir> [--game-id ID] [--json report.json] [--discard-snapshots]

/// mimalloc as the global allocator: the post-capture fold is allocation-bound
/// (multi-GB serde_json `Value` trees built and dropped per game-hour), and the
/// Windows system heap serializes badly under that load across fold workers.
/// Allocator choice cannot change any produced byte (G1-neutral).
#[global_allocator]
static GLOBAL_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use recorder_host::proc_mem::peak_memory;
use recorder_host::stream_fold::{
    default_fold_workers, fold_compact_stream_file_parallel, hash_folded_snapshot_files,
    snapshot_chain_hash,
};
use serde_json::{json, Value};

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
    let mut discard_snapshots = false;
    let mut workers = default_fold_workers();
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
            "--discard-snapshots" => {
                discard_snapshots = true;
            }
            "--workers" => {
                i += 1;
                workers = args
                    .get(i)
                    .ok_or("--workers requires a count")?
                    .parse::<usize>()
                    .map_err(|e| format!("--workers expects a positive integer: {e}"))?;
                if workers == 0 {
                    return Err("--workers must be >= 1".into());
                }
            }
            "-h" | "--help" => {
                eprintln!(
                    "stream_fold <collector_stream.bin> <out_snapshot_dir> \
                     [--game-id ID] [--json report.json] [--discard-snapshots]"
                );
                return Ok(());
            }
            other if other.starts_with("--") => return Err(format!("unknown flag {other}")),
            other => positional.push(other.to_string()),
        }
        i += 1;
    }
    if positional.len() != 2 {
        return Err("usage: stream_fold <stream.bin> <out_snapshot_dir> [--game-id ID] [--json report.json] [--discard-snapshots] [--workers N]".into());
    }
    let stream_path = PathBuf::from(&positional[0]);
    let out_dir = PathBuf::from(&positional[1]);

    let start = std::time::Instant::now();
    let (files, stats) =
        fold_compact_stream_file_parallel(&stream_path, &game_id, &out_dir, workers)
            .map_err(|e| format!("streaming fold {}: {e}", stream_path.display()))?;
    let elapsed = start.elapsed();
    if let Some(breakdown) = recorder_host::world_fold::foldprof::report() {
        eprintln!("stream_fold: {breakdown}");
    }

    let hour_rows = hash_folded_snapshot_files(&files, discard_snapshots)
        .map_err(|e| format!("snapshot report: {e}"))?;
    let chain_hash = snapshot_chain_hash(&hour_rows);
    let hours: Vec<Value> = hour_rows
        .iter()
        .map(|row| {
            json!({
                "hour_index": row.hour_index,
                "date": row.date,
                "snapshot_hash": row.snapshot_hash,
                "snapshot_bytes": row.snapshot_bytes,
            })
        })
        .collect();

    let peak = peak_memory();
    let report = json!({
        "tool": "stream_fold",
        "input": stream_path.display().to_string(),
        "workers": workers,
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
