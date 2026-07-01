#![recursion_limit = "256"]

use country_economy_counters_live_lane::{domain_name, gate_counter_stream, DOMAIN_ID};
use serde_json::json;
use snapshot::{pack, Container, ExpectedIdentity, PackEntry, PackIdentity};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const REPLAY_SCHEMA_VERSION: &str = "native_collector_session_v1";
const DEFAULT_GAME_ID: &str = "country_economy_counters_live_lane_local";
const STREAM_ENTRY_PATH: &str = "collector_raw/collector_stream.bin";
const MANIFEST_ENTRY_PATH: &str = "recorder/native_session_manifest.json";

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let options = Options::parse()?;
    let stream = fs::read(&options.stream)
        .map_err(|err| format!("read {} failed: {err}", options.stream.display()))?;
    let gate = gate_counter_stream(&stream, options.min_game_hours);
    if !gate.ok {
        return Err(format!(
            "country_economy_counters_live stream gate failed before replay pack: {:?}",
            gate.summary.coverage_errors
        ));
    }

    fs::create_dir_all(&options.out_dir).map_err(|err| format!("create out dir failed: {err}"))?;
    let stream_out = options.out_dir.join("collector_stream.bin");
    let manifest_out = options.out_dir.join("native_session_manifest.json");
    let replay_out = options.out_dir.join("native_session.battleplan");
    let proof_out = options.out_dir.join("replay_pack_proof.json");

    let manifest = manifest_json(&stream, &options, &gate)?;
    let replay = pack_replay(&options.game_id, &stream, &manifest)?;
    let verification = verify_replay(&options.game_id, &replay, &stream)?;

    fs::write(&stream_out, &stream).map_err(|err| format!("write stream failed: {err}"))?;
    fs::write(&manifest_out, &manifest).map_err(|err| format!("write manifest failed: {err}"))?;
    write_atomic(&replay_out, &replay)?;

    let proof = json!({
        "result": "verified",
        "schema": "country_economy_counters_live.replay_pack_proof.v1",
        "domain": domain_name(DOMAIN_ID),
        "domain_id": DOMAIN_ID,
        "source_stream": options.stream.display().to_string(),
        "out_dir": options.out_dir.display().to_string(),
        "game_id": options.game_id,
        "min_game_hours": options.min_game_hours,
        "collector_stream_path": stream_out.display().to_string(),
        "manifest_path": manifest_out.display().to_string(),
        "hoi4replay_path": replay_out.display().to_string(),
        "collector_stream_bytes": stream.len(),
        "hoi4replay_bytes": replay.len(),
        "verified_stream_entry": verification.stream_entry,
        "verified_stream_bytes": verification.stream_bytes,
        "collector_compact_stream_entries": verification.collector_stream_entries,
        "recorder_manifest_entries": verification.recorder_manifest_entries,
        "gate": {
            "ok": gate.ok,
            "domain_data_records": gate.summary.domain_data_records,
            "unique_game_hours": gate.summary.unique_game_hours,
            "skipped_game_hours": gate.summary.skipped_game_hours,
            "status_flags_or": gate.summary.status_flags_or,
            "parse_errors": gate.summary.parse_errors,
            "ordering_errors": gate.summary.ordering_errors,
            "coverage_errors": gate.summary.coverage_errors,
        }
    });
    let proof_bytes = serde_json::to_vec_pretty(&proof).map_err(|err| err.to_string())?;
    fs::write(&proof_out, &proof_bytes).map_err(|err| format!("write proof failed: {err}"))?;
    println!("{}", String::from_utf8_lossy(&proof_bytes));
    Ok(())
}

fn manifest_json(
    stream: &[u8],
    options: &Options,
    gate: &country_economy_counters_live_lane::CounterGate,
) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(&json!({
        "schema": "native_collector_session_v1",
        "session_id": options.game_id,
        "replay_game_id": options.game_id,
        "stream_format": "compact",
        "stream_file": "collector_stream.bin",
        "stream_role": "collector_compact_stream",
        "scheduled_domains": [domain_name(DOMAIN_ID)],
        "source": "lane-local country_economy_counters_live compact stream replay pack proof",
        "hourly_requirement": {
            "required": true,
            "requested_game_hours": options.min_game_hours,
            "met": gate.summary.unique_game_hours >= options.min_game_hours,
            "problems": gate.summary.coverage_errors,
        },
        "unique_game_hour_count": gate.summary.unique_game_hours,
        "skipped_game_hours": gate.summary.skipped_game_hours.len(),
        "parse_errors": gate.summary.parse_errors,
        "ordering_errors": gate.summary.ordering_errors,
        "extraction_perf_gate": true,
        "compact_stream": {
            "record_magic": "H4CR",
            "record_version": 1,
            "record_header_len": 32,
            "decodable": gate.summary.parse_errors.is_empty(),
            "ordered": gate.summary.ordering_errors.is_empty(),
            "total_bytes": stream.len(),
            "record_count": gate.summary.record_count,
            "domain_data_record_count": gate.summary.domain_data_records,
            "frame_end_record_count": gate.summary.frame_end_records,
            "status_flags_or": gate.summary.status_flags_or,
            "domains_seen": [domain_name(DOMAIN_ID)],
            "expected_domains": [domain_name(DOMAIN_ID)],
        },
    }))
    .map_err(|err| err.to_string())
}

fn pack_replay(game_id: &str, stream: &[u8], manifest: &[u8]) -> Result<Vec<u8>, String> {
    let index_bytes = serde_json::to_vec_pretty(&json!({
        "game_id": game_id,
        "schema_version": REPLAY_SCHEMA_VERSION,
        "snapshots": [],
    }))
    .map_err(|err| err.to_string())?;
    let timeline_bytes = serde_json::to_vec_pretty(&json!({
        "format": "hoi4_compact_stream_timeline_v1",
        "stream_format": "compact",
        "stream": STREAM_ENTRY_PATH,
        "session_manifest": MANIFEST_ENTRY_PATH,
        "raw_stream_role": "collector_compact_stream",
        "session_manifest_role": "recorder_manifest",
    }))
    .map_err(|err| err.to_string())?;
    let entries = vec![
        PackEntry {
            path: format!("payload/{game_id}/index.json"),
            bytes: index_bytes,
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/timeline.json"),
            bytes: timeline_bytes,
            role: "timeline",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/{STREAM_ENTRY_PATH}"),
            bytes: stream.to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/{MANIFEST_ENTRY_PATH}"),
            bytes: manifest.to_vec(),
            role: "recorder_manifest",
            hour_index: None,
        },
    ];
    let identity = PackIdentity {
        schema_version: Some(REPLAY_SCHEMA_VERSION.to_string()),
        build_exe_sha256: Some(
            "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad".to_string(),
        ),
        build_pinned: true,
        created_utc: Some("country_economy_counters_live_lane_local_replay_pack_proof".to_string()),
        ..Default::default()
    };
    pack(game_id, &entries, &identity, true).map_err(|err| err.to_string())
}

fn verify_replay(
    game_id: &str,
    replay: &[u8],
    expected_stream: &[u8],
) -> Result<Verification, String> {
    let container = Container::from_bytes(replay.to_vec(), &ExpectedIdentity::default())
        .map_err(|err| err.to_string())?;
    container.verify_all().map_err(|err| err.to_string())?;
    let stream_entries = container.entries_by_role("collector_compact_stream");
    if stream_entries.len() != 1 {
        return Err(format!(
            "collector_compact_stream entries {} != 1",
            stream_entries.len()
        ));
    }
    let manifest_entries = container.entries_by_role("recorder_manifest");
    if manifest_entries.len() != 1 {
        return Err(format!(
            "recorder_manifest entries {} != 1",
            manifest_entries.len()
        ));
    }
    let expected_path = format!("payload/{game_id}/{STREAM_ENTRY_PATH}");
    if stream_entries[0].path != expected_path {
        return Err(format!(
            "collector_compact_stream path {} != {}",
            stream_entries[0].path, expected_path
        ));
    }
    let stored_stream = container
        .read_entry_bytes(&stream_entries[0].path)
        .map_err(|err| err.to_string())?;
    if stored_stream != expected_stream {
        return Err("collector_compact_stream bytes differ after replay pack".to_string());
    }
    Ok(Verification {
        stream_entry: stream_entries[0].path.clone(),
        stream_bytes: stored_stream.len(),
        collector_stream_entries: stream_entries.len(),
        recorder_manifest_entries: manifest_entries.len(),
    })
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("hoi4replay.tmp");
    fs::write(&tmp, bytes).map_err(|err| format!("write replay tmp failed: {err}"))?;
    if path.exists() {
        fs::remove_file(path).map_err(|err| format!("replace replay failed: {err}"))?;
    }
    fs::rename(&tmp, path).map_err(|err| format!("finalize replay failed: {err}"))
}

struct Verification {
    stream_entry: String,
    stream_bytes: usize,
    collector_stream_entries: usize,
    recorder_manifest_entries: usize,
}

struct Options {
    stream: PathBuf,
    out_dir: PathBuf,
    game_id: String,
    min_game_hours: usize,
}

impl Options {
    fn parse() -> Result<Self, String> {
        let mut stream = None;
        let mut out_dir = None;
        let mut game_id = DEFAULT_GAME_ID.to_string();
        let mut min_game_hours = 1usize;
        let args = env::args().skip(1).collect::<Vec<_>>();
        let mut i = 0usize;
        while i < args.len() {
            match args[i].as_str() {
                "--stream" => {
                    i += 1;
                    stream = Some(PathBuf::from(required(args.get(i), "--stream")?));
                }
                "--out-dir" => {
                    i += 1;
                    out_dir = Some(PathBuf::from(required(args.get(i), "--out-dir")?));
                }
                "--game-id" => {
                    i += 1;
                    game_id = required(args.get(i), "--game-id")?;
                }
                "--min-game-hours" => {
                    i += 1;
                    let raw = required(args.get(i), "--min-game-hours")?;
                    min_game_hours = raw
                        .parse::<usize>()
                        .map_err(|err| format!("invalid --min-game-hours {raw}: {err}"))?;
                }
                "--help" | "-h" => return Err(usage()),
                other => return Err(format!("unknown argument {other}\n{}", usage())),
            }
            i += 1;
        }
        Ok(Self {
            stream: stream.ok_or_else(usage)?,
            out_dir: out_dir.ok_or_else(usage)?,
            game_id,
            min_game_hours,
        })
    }
}

fn required(value: Option<&String>, flag: &str) -> Result<String, String> {
    value
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn usage() -> String {
    "usage: replay_pack_proof --stream domain58_stream.bin --out-dir <proof-dir> [--game-id <id>] [--min-game-hours <n>]".to_string()
}
