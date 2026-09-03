//! Lane PR: run THE production certified-artifact build over a retained spool
//! and report its stage timings, digests and peak memory as one JSON document.
//! usage: pr_finalize_profile <spool> <empty_spill_dir> <report.json> [legacy|identity-v1]
//!        [--game-id <id>] [--fold-cache <dir>]
//! With the sealed recording's own game id the digests it prints are the ones
//! that recording's manifest carries (`host_finalize.fold.hour_index_sha256` /
//! `checkpoint_cache_sha256`), so a run over the retained spool is a
//! byte-identity proof. `--fold-cache` exercises the content-keyed fold cache:
//! an entry is adopted when valid, else the fold runs and publishes one.

use std::io::Read;
use std::path::PathBuf;
use std::time::Instant;

use replay_decoder::StreamIdentityMode;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!(
            "usage: pr_finalize_profile <spool> <empty_spill_dir> <report.json> [legacy|identity-v1] [--game-id <id>] [--fold-cache <dir>]"
        );
        std::process::exit(2);
    }
    let spool = PathBuf::from(&args[1]);
    let spill = PathBuf::from(&args[2]);
    let report_path = PathBuf::from(&args[3]);
    let mut identity_mode = StreamIdentityMode::IdentityV1;
    let mut game_id = "pr_finalize_profile".to_string();
    let mut fold_cache: Option<PathBuf> = None;
    let mut i = 4;
    while i < args.len() {
        match args[i].as_str() {
            "identity-v1" => identity_mode = StreamIdentityMode::IdentityV1,
            "legacy" => identity_mode = StreamIdentityMode::Legacy,
            "--game-id" => {
                i += 1;
                game_id = args.get(i).expect("--game-id requires a value").clone();
            }
            "--fold-cache" => {
                i += 1;
                fold_cache = Some(PathBuf::from(
                    args.get(i).expect("--fold-cache requires a path"),
                ));
            }
            other => {
                eprintln!("unknown argument {other:?}");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let hash_started = Instant::now();
    let (source_len, source_sha256) = hash_file(&spool);
    let source_hash_s = hash_started.elapsed().as_secs_f64();

    std::fs::create_dir_all(&spill).expect("create spill dir");
    let started = Instant::now();
    let mut cache_outcome = serde_json::Value::Null;
    let mut adopted = None;
    let cache_params = replay_decoder::fold_cache::FoldCacheParams::new(
        game_id.as_str(),
        identity_mode,
        snapshot::DEFAULT_CHECKPOINT_INTERVAL_HOURS,
    );
    if let Some(root) = fold_cache.as_deref() {
        let adopt_started = Instant::now();
        let adoption = replay_decoder::fold_cache::adopt(
            root,
            source_len,
            &source_sha256,
            &cache_params,
            &spill,
        );
        let adopt_s = adopt_started.elapsed().as_secs_f64();
        cache_outcome = match &adoption {
            Ok(Some(_)) => serde_json::json!({ "mode": "adopted", "adopt_seconds": adopt_s }),
            Ok(None) => serde_json::json!({ "mode": "miss", "adopt_seconds": adopt_s }),
            Err(reason) => serde_json::json!({
                "mode": "refused",
                "cause": reason,
                "adopt_seconds": adopt_s,
            }),
        };
        if let Ok(Some(artifacts)) = adoption {
            adopted = Some(artifacts);
        }
    }
    let outcome = match adopted {
        Some(artifacts) => Ok(artifacts),
        None => {
            let built = replay_decoder::build_certified_replay_artifacts(
                &spool,
                &spill,
                game_id.as_str(),
                identity_mode,
                source_len,
                &source_sha256,
                snapshot::DEFAULT_CHECKPOINT_INTERVAL_HOURS,
            );
            if let (Some(root), Ok(artifacts)) = (fold_cache.as_deref(), &built) {
                let publish_started = Instant::now();
                let published =
                    replay_decoder::fold_cache::publish(root, &cache_params, artifacts, None);
                cache_outcome["publish_seconds"] =
                    serde_json::json!(publish_started.elapsed().as_secs_f64());
                cache_outcome["published"] = match published {
                    Ok(dir) => serde_json::json!(dir.display().to_string()),
                    Err(error) => serde_json::json!({ "error": error }),
                };
            }
            built
        }
    };
    let wall_s = started.elapsed().as_secs_f64();
    let peak = recorder_host::proc_mem::peak_memory();

    let artifacts = match &outcome {
        Ok(artifacts) => serde_json::json!({
            "hour_count": artifacts.hour_count,
            "checkpoint_count": artifacts.checkpoint_count,
            "max_seek_deltas": artifacts.max_seek_deltas,
            "hour_index_len": artifacts.hour_index_len,
            "hour_index_sha256": artifacts.hour_index_sha256,
            "checkpoint_cache_len": artifacts.checkpoint_cache_len,
            "checkpoint_cache_sha256": artifacts.checkpoint_cache_sha256,
            "legacy_json_bytes_avoided": artifacts.legacy_json_bytes_avoided,
            "durable_decoded_end_offset": artifacts.durable_decoded_end_offset,
        }),
        Err(error) => serde_json::json!({ "error": error.to_string() }),
    };
    let report = serde_json::json!({
        "tool": "pr_finalize_profile",
        "input": spool.display().to_string(),
        "game_id": game_id,
        "identity_mode": match identity_mode {
            StreamIdentityMode::IdentityV1 => "identity-v1",
            StreamIdentityMode::Legacy => "legacy",
        },
        "fold_source_fingerprint": replay_decoder::fold_cache::FOLD_SOURCE_FINGERPRINT,
        "fold_cache": cache_outcome,
        "source_len": source_len,
        "source_sha256": source_sha256,
        "source_hash_seconds": source_hash_s,
        "build_wall_seconds": wall_s,
        "seconds_per_hour": outcome
            .as_ref()
            .ok()
            .filter(|artifacts| artifacts.hour_count > 0)
            .map(|artifacts| wall_s / artifacts.hour_count as f64),
        "peak_private_bytes": peak.as_ref().map(|p| p.peak_private_bytes),
        "peak_working_set_bytes": peak.as_ref().map(|p| p.peak_working_set_bytes),
        "artifacts": artifacts,
        "fold_profile": recorder_host::world_fold::foldprof::report(),
        "cache_profile": snapshot::replay_cache::cacheprof::report(),
        "finalize_profile": replay_decoder::finprof::report(),
    });
    let text = serde_json::to_string_pretty(&report).expect("report serializes") + "\n";
    std::fs::write(&report_path, text.as_bytes()).expect("write report");
    eprintln!("{text}");
    if outcome.is_err() {
        std::process::exit(1);
    }
}

fn hash_file(path: &std::path::Path) -> (u64, String) {
    use sha2::Digest;
    let mut file = std::fs::File::open(path).expect("open spool");
    let mut hasher = sha2::Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer).expect("read spool");
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
    (total, format!("{:x}", hasher.finalize()))
}
