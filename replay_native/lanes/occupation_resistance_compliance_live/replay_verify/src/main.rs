use serde_json::{json, Value};
use snapshot::{pack, Container, ExpectedIdentity, PackEntry, PackIdentity};
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const GAME_ID_DEFAULT: &str = "orc52";
const SCHEMA_VERSION: &str = "native_collector_session_v1";
const STREAM_FILE: &str = "collector_stream.bin";
const MANIFEST_FILE: &str = "native_session_manifest.json";
const REPLAY_FILE: &str = "native_session.battleplan";

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse(env::args().skip(1))?;
    let report = pack_and_verify(&args.run_dir, &args.game_id)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

struct Args {
    run_dir: PathBuf,
    game_id: String,
}

impl Args {
    fn parse<I>(mut values: I) -> Result<Self, String>
    where
        I: Iterator<Item = String>,
    {
        let mut run_dir = None;
        let mut game_id = GAME_ID_DEFAULT.to_string();
        while let Some(arg) = values.next() {
            match arg.as_str() {
                "--run-dir" => {
                    let value = values
                        .next()
                        .ok_or_else(|| "--run-dir requires a value".to_string())?;
                    run_dir = Some(PathBuf::from(value));
                }
                "--game-id" => {
                    game_id = values
                        .next()
                        .ok_or_else(|| "--game-id requires a value".to_string())?;
                    validate_game_id(&game_id)?;
                }
                "--help" | "-h" => return Err(usage()),
                _ => return Err(format!("unknown argument {arg:?}\n{}", usage())),
            }
        }
        Ok(Self {
            run_dir: run_dir.ok_or_else(usage)?,
            game_id,
        })
    }
}

fn usage() -> String {
    "usage: replay_verify --run-dir <lane-proof-run> [--game-id orc52]".to_string()
}

fn validate_game_id(game_id: &str) -> Result<(), String> {
    if game_id.is_empty()
        || !game_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err("game id must be non-empty ASCII alnum, '_' or '-'".to_string());
    }
    Ok(())
}

fn pack_and_verify(run_dir: &Path, game_id: &str) -> Result<Value, Box<dyn Error>> {
    validate_game_id(game_id)?;
    let summary_path = run_dir.join("summary.json");
    let stream_path = run_dir.join(STREAM_FILE);
    let manifest_path = run_dir.join(MANIFEST_FILE);
    let replay_path = run_dir.join(REPLAY_FILE);
    let summary_bytes = fs::read(&summary_path)?;
    let summary: Value = serde_json::from_slice(&summary_bytes)?;
    let domain_id = summary
        .pointer("/domain/id")
        .and_then(Value::as_u64)
        .ok_or("summary missing /domain/id")?;
    if domain_id != 52 {
        return Err(format!("expected domain id 52, got {domain_id}").into());
    }
    let stream_bytes = fs::read(&stream_path)?;
    let build_sha = summary
        .pointer("/process/sha256")
        .and_then(Value::as_str)
        .map(str::to_string);
    let stream_payload_path = format!("collector_raw/{STREAM_FILE}");
    let manifest_payload_path = format!("recorder/{MANIFEST_FILE}");
    let manifest = json!({
        "schema": "occupation_resistance_compliance_live_lane_replay_manifest_v1",
        "lane": "occupation_resistance_compliance_live",
        "domain_id": 52,
        "game_id": game_id,
        "build_sha256": build_sha,
        "summary": {
            "rows": summary.get("rows").cloned().unwrap_or(Value::Null),
            "compact": summary.get("compact").cloned().unwrap_or(Value::Null),
            "roots": summary.get("roots").cloned().unwrap_or(Value::Null),
        },
        "replay_entries": {
            "index": "index.json",
            "timeline": "timeline.json",
            "stream": stream_payload_path,
            "stream_role": "collector_compact_stream",
            "session_manifest": manifest_payload_path,
            "session_manifest_role": "recorder_manifest"
        },
        "unresolved_badges": [
            "garrison_need_unavailable",
            "garrison_equilibrium_unavailable",
            "garrison_progress_unavailable",
            "collaboration_detail_unavailable",
            "government_in_exile_detail_unavailable"
        ]
    });
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;

    let index_bytes = serde_json::to_vec_pretty(&json!({
        "game_id": game_id,
        "schema_version": SCHEMA_VERSION,
        "snapshots": [],
    }))?;
    let timeline_bytes = serde_json::to_vec_pretty(&json!({
        "format": "hoi4_compact_stream_timeline_v1",
        "stream_format": "compact",
        "stream": format!("collector_raw/{STREAM_FILE}"),
        "session_manifest": format!("recorder/{MANIFEST_FILE}"),
        "raw_stream_role": "collector_compact_stream",
        "session_manifest_role": "recorder_manifest",
        "domain_id": 52,
        "lane": "occupation_resistance_compliance_live",
    }))?;
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
            path: format!("payload/{game_id}/collector_raw/{STREAM_FILE}"),
            bytes: stream_bytes.clone(),
            role: "collector_compact_stream",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/recorder/{MANIFEST_FILE}"),
            bytes: fs::read(&manifest_path)?,
            role: "recorder_manifest",
            hour_index: None,
        },
    ];
    let identity = PackIdentity {
        schema_version: Some(SCHEMA_VERSION.to_string()),
        build_exe_sha256: build_sha.clone(),
        build_pinned: build_sha.is_some(),
        created_utc: Some(created_label()),
        ..Default::default()
    };
    let replay_bytes = pack(game_id, &entries, &identity, true)?;
    let tmp_path = replay_path.with_extension("hoi4replay.tmp");
    fs::write(&tmp_path, replay_bytes)?;
    if replay_path.exists() {
        fs::remove_file(&replay_path)?;
    }
    fs::rename(&tmp_path, &replay_path)?;

    let expected = ExpectedIdentity {
        build_exe_sha256: build_sha.clone(),
        schema_version: Some(SCHEMA_VERSION.to_string()),
        ..Default::default()
    };
    let container = Container::open(&replay_path, &expected)?;
    container.verify_all()?;
    let stream_entry = format!("payload/{game_id}/collector_raw/{STREAM_FILE}");
    let verified_stream = container.read_entry_bytes(&stream_entry)?;
    if verified_stream != stream_bytes {
        return Err("verified replay compact stream bytes changed".into());
    }
    let roles = container
        .entries()
        .into_iter()
        .map(|entry| {
            json!({
                "path": entry.path,
                "role": entry.role,
                "bytes": entry.bytes,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "verified": true,
        "replay": replay_path,
        "replay_bytes": fs::metadata(&replay_path)?.len(),
        "game_id": container.game_id(),
        "schema_version": SCHEMA_VERSION,
        "build_sha256": build_sha,
        "collector_compact_stream_entry": stream_entry,
        "collector_compact_stream_bytes": verified_stream.len(),
        "manifest_entries": roles,
        "index_snapshot_count": container.index().snapshots.len(),
    }))
}

fn created_label() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("unix_seconds:{secs}")
}

#[cfg(test)]
mod tests {
    use super::validate_game_id;

    #[test]
    fn game_id_validation_accepts_short_safe_ids() {
        validate_game_id("orc52").unwrap();
        validate_game_id("orc-52_test").unwrap();
    }

    #[test]
    fn game_id_validation_rejects_paths() {
        assert!(validate_game_id("../orc52").is_err());
        assert!(validate_game_id("").is_err());
    }
}
