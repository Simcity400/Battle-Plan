use serde_json::json;
use snapshot::{pack, Container, ExpectedIdentity, PackEntry, PackIdentity};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "../../domain44_supply_logistics_attrition.rs"]
mod domain44;

const REPLAY_SCHEMA_VERSION: &str = "native_collector_session_v1";
const DEFAULT_GAME_ID: &str = "supply_logistics_attrition_live_lane_local";
const STREAM_ENTRY_PATH: &str = "collector_raw/collector_stream.bin";
const MANIFEST_ENTRY_PATH: &str = "recorder/native_session_manifest.json";
const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
const COMPACT_RECORD_HEADER_LEN: usize = 32;
const COMPACT_RECORD_VERSION: u16 = 1;
const RECORD_TYPE_FRAME_END: u16 = 2;
const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
const BUILD_SHA256: &str = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let options = Options::parse()?;
    let payload = fs::read(&options.payload)
        .map_err(|err| format!("read {} failed: {err}", options.payload.display()))?;
    let payload_summary = domain44::decode_payload(0, &payload);
    if !payload_summary.parse_errors.is_empty() {
        return Err(format!(
            "domain44 payload gate failed before stream pack: {:?}",
            payload_summary.parse_errors
        ));
    }
    if payload_summary.csupply_input_rows_total == 0
        && payload_summary.pressure_candidate_rows_total == 0
    {
        return Err("domain44 payload has zero csupply input rows and zero pressure candidate rows".to_string());
    }

    fs::create_dir_all(&options.out_dir).map_err(|err| format!("create out dir failed: {err}"))?;
    let stream = build_stream(&payload);
    let stream_gate = gate_stream(&stream)?;
    if !stream_gate.ok {
        return Err(format!(
            "domain44 compact stream gate failed before replay pack: {:?}",
            stream_gate.problems
        ));
    }

    let manifest = manifest_json(&options, &stream, &stream_gate, &payload_summary)?;
    let replay = pack_replay(&options.game_id, &stream, &manifest)?;
    let verification = verify_replay(&options.game_id, &replay, &stream)?;

    let stream_out = options.out_dir.join("collector_stream.bin");
    let manifest_out = options.out_dir.join("native_session_manifest.json");
    let replay_out = options.out_dir.join("native_session.battleplan");
    let proof_out = options.out_dir.join("replay_pack_proof.json");
    let proof_md = options.out_dir.join("REPLAY_PACK_PROOF.md");
    fs::write(&stream_out, &stream).map_err(|err| format!("write stream failed: {err}"))?;
    fs::write(&manifest_out, &manifest).map_err(|err| format!("write manifest failed: {err}"))?;
    write_atomic(&replay_out, &replay)?;

    let proof = json!({
        "result": "verified",
        "schema": "supply_logistics_attrition_live.replay_pack_proof.v1",
        "domain": domain44::DOMAIN_NAME,
        "domain_id": domain44::DOMAIN_ID,
        "source_payload": options.payload.display().to_string(),
        "out_dir": options.out_dir.display().to_string(),
        "game_id": options.game_id,
        "collector_stream_path": stream_out.display().to_string(),
        "manifest_path": manifest_out.display().to_string(),
        "hoi4replay_path": replay_out.display().to_string(),
        "collector_stream_bytes": stream.len(),
        "hoi4replay_bytes": replay.len(),
        "verified_stream_entry": verification.stream_entry,
        "verified_stream_bytes": verification.stream_bytes,
        "collector_compact_stream_entries": verification.collector_stream_entries,
        "recorder_manifest_entries": verification.recorder_manifest_entries,
        "payload_gate": payload_summary_json(&payload_summary),
        "stream_gate": stream_gate.to_json(),
    });
    let proof_bytes = serde_json::to_vec_pretty(&proof).map_err(|err| err.to_string())?;
    fs::write(&proof_out, &proof_bytes).map_err(|err| format!("write proof failed: {err}"))?;
    fs::write(&proof_md, proof_markdown(&proof)).map_err(|err| format!("write proof md failed: {err}"))?;
    println!("{}", String::from_utf8_lossy(&proof_bytes));
    Ok(())
}

fn build_stream(payload: &[u8]) -> Vec<u8> {
    let mut stream = Vec::new();
    append_compact_record(
        &mut stream,
        1,
        1,
        domain44::DOMAIN_ID,
        RECORD_TYPE_DOMAIN_DATA,
        payload,
    );
    append_compact_record(&mut stream, 2, 1, 0, RECORD_TYPE_FRAME_END, &[]);
    stream
}

fn append_compact_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    payload: &[u8],
) {
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
}

fn gate_stream(raw: &[u8]) -> Result<StreamGate, String> {
    let mut gate = StreamGate {
        total_bytes: raw.len(),
        ..StreamGate::default()
    };
    let mut offset = 0usize;
    let mut previous_sequence = None;
    let mut data_hour = None;
    let mut frame_hour = None;
    while offset < raw.len() {
        if raw.len().saturating_sub(offset) < COMPACT_RECORD_HEADER_LEN {
            gate.problems
                .push(format!("offset {offset}: truncated compact record header"));
            break;
        }
        let header = &raw[offset..offset + COMPACT_RECORD_HEADER_LEN];
        if &header[0..4] != COMPACT_RECORD_MAGIC {
            gate.problems
                .push(format!("offset {offset}: invalid compact record magic"));
            break;
        }
        let version = read_u16_le(header, 4);
        let header_len = read_u16_le(header, 6) as usize;
        if version != COMPACT_RECORD_VERSION || header_len != COMPACT_RECORD_HEADER_LEN {
            gate.problems.push(format!(
                "offset {offset}: unsupported compact record version {version} header_len {header_len}"
            ));
            break;
        }
        let sequence = read_u64_le(header, 8);
        let game_hour_raw = read_i32_le(header, 16);
        let domain_id = read_u16_le(header, 20);
        let record_type = read_u16_le(header, 22);
        let payload_len = read_u32_le(header, 24) as usize;
        let reserved = read_u32_le(header, 28);
        if reserved != 0 {
            gate.problems
                .push(format!("offset {offset}: compact record reserved field is nonzero"));
        }
        if let Some(previous) = previous_sequence {
            if sequence <= previous {
                gate.problems.push(format!(
                    "offset {offset}: sequence {sequence} did not advance past {previous}"
                ));
            }
        }
        previous_sequence = Some(sequence);
        let record_len = COMPACT_RECORD_HEADER_LEN.saturating_add(payload_len);
        if record_len > raw.len().saturating_sub(offset) {
            gate.problems.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining stream bytes"
            ));
            break;
        }
        let payload_start = offset + COMPACT_RECORD_HEADER_LEN;
        let payload_end = payload_start + payload_len;
        let payload = &raw[payload_start..payload_end];
        gate.record_count = gate.record_count.saturating_add(1);
        match record_type {
            RECORD_TYPE_DOMAIN_DATA if domain_id == domain44::DOMAIN_ID => {
                gate.domain_data_records = gate.domain_data_records.saturating_add(1);
                data_hour = Some(game_hour_raw);
                let summary = domain44::decode_payload(offset, payload);
                if !summary.parse_errors.is_empty() {
                    gate.problems.extend(summary.parse_errors.clone());
                }
                gate.province_rows_total = gate
                    .province_rows_total
                    .saturating_add(summary.province_rows_total);
                gate.division_rows_total = gate
                    .division_rows_total
                    .saturating_add(summary.division_rows_total);
                gate.badge_rows_total = gate.badge_rows_total.saturating_add(summary.badge_rows_total);
                gate.csupply_input_rows_total = gate
                    .csupply_input_rows_total
                    .saturating_add(summary.csupply_input_rows_total);
                gate.pressure_candidate_rows_total = gate
                    .pressure_candidate_rows_total
                    .saturating_add(summary.pressure_candidate_rows_total);
                gate.data_rows_total = gate.data_rows_total.saturating_add(summary.data_rows_total);
                gate.status_flags_or |= summary.status_flags_or;
            }
            RECORD_TYPE_FRAME_END => {
                gate.frame_end_records = gate.frame_end_records.saturating_add(1);
                frame_hour = Some(game_hour_raw);
            }
            other => gate.problems.push(format!(
                "offset {offset}: unexpected compact record type {other} domain {domain_id}"
            )),
        }
        offset += record_len;
    }
    if gate.record_count != 2 {
        gate.problems
            .push(format!("record_count {} != 2", gate.record_count));
    }
    if gate.domain_data_records != 1 {
        gate.problems.push(format!(
            "domain_data_records {} != 1",
            gate.domain_data_records
        ));
    }
    if gate.frame_end_records != 1 {
        gate.problems
            .push(format!("frame_end_records {} != 1", gate.frame_end_records));
    }
    if data_hour != frame_hour {
        gate.problems
            .push(format!("data_hour {:?} != frame_hour {:?}", data_hour, frame_hour));
    }
    if gate.csupply_input_rows_total == 0 && gate.pressure_candidate_rows_total == 0 {
        gate.problems
            .push("csupply_input_rows_total and pressure_candidate_rows_total are both zero".to_string());
    }
    gate.unique_game_hours = if data_hour.is_some() && data_hour == frame_hour {
        1
    } else {
        0
    };
    gate.skipped_game_hours = 0;
    gate.ok = gate.problems.is_empty();
    Ok(gate)
}

fn manifest_json(
    options: &Options,
    stream: &[u8],
    gate: &StreamGate,
    payload_summary: &domain44::Domain44GateSummary,
) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(&json!({
        "schema": "native_collector_session_v1",
        "session_id": options.game_id,
        "replay_game_id": options.game_id,
        "stream_format": "compact",
        "stream_file": "collector_stream.bin",
        "stream_role": "collector_compact_stream",
        "scheduled_domains": [domain44::DOMAIN_NAME],
        "source": "lane-local supply_logistics_attrition_live compact stream replay pack proof",
        "hourly_requirement": {
            "required": true,
            "requested_game_hours": 1,
            "met": gate.unique_game_hours == 1 && gate.skipped_game_hours == 0,
            "problems": gate.problems,
        },
        "unique_game_hour_count": gate.unique_game_hours,
        "skipped_game_hours": gate.skipped_game_hours,
        "parse_errors": gate.problems,
        "compact_stream": {
            "record_magic": "H4CR",
            "record_version": 1,
            "record_header_len": 32,
            "decodable": gate.ok,
            "ordered": true,
            "total_bytes": stream.len(),
            "record_count": gate.record_count,
            "domain_data_record_count": gate.domain_data_records,
            "frame_end_record_count": gate.frame_end_records,
            "status_flags_or": gate.status_flags_or,
            "worker_panic_seen": false,
            "bridge_unavailable_seen": false,
            "source_unavailable_seen": false,
            "partial_data_seen": false,
            "domains_seen": [domain44::DOMAIN_NAME],
            "expected_domains": [domain44::DOMAIN_NAME],
            "per_domain": {
                domain44::DOMAIN_NAME: {
                    "domain_id": domain44::DOMAIN_ID,
                    "data_records": gate.domain_data_records,
                    "data_rows_total": payload_summary.data_rows_total,
                    "province_rows_total": payload_summary.province_rows_total,
                    "division_rows_total": payload_summary.division_rows_total,
                    "badge_rows_total": payload_summary.badge_rows_total,
                    "csupply_input_rows_total": payload_summary.csupply_input_rows_total,
                    "observed_province_roots": payload_summary.observed_province_roots,
                    "observed_division_roots": payload_summary.observed_division_roots,
                }
            }
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
        "domain_id": domain44::DOMAIN_ID,
        "lane": domain44::DOMAIN_NAME,
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
        build_exe_sha256: Some(BUILD_SHA256.to_string()),
        build_pinned: true,
        created_utc: Some(created_label()),
        ..Default::default()
    };
    pack(game_id, &entries, &identity, true).map_err(|err| err.to_string())
}

fn verify_replay(
    game_id: &str,
    replay: &[u8],
    expected_stream: &[u8],
) -> Result<Verification, String> {
    let expected = ExpectedIdentity {
        build_exe_sha256: Some(BUILD_SHA256.to_string()),
        schema_version: Some(REPLAY_SCHEMA_VERSION.to_string()),
        ..Default::default()
    };
    let container = Container::from_bytes(replay.to_vec(), &expected).map_err(|err| err.to_string())?;
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
    let replay_stream_gate = gate_stream(&stored_stream)?;
    if !replay_stream_gate.ok {
        return Err(format!(
            "replay readback stream gate failed: {:?}",
            replay_stream_gate.problems
        ));
    }
    Ok(Verification {
        stream_entry: stream_entries[0].path.clone(),
        stream_bytes: stored_stream.len(),
        collector_stream_entries: stream_entries.len(),
        recorder_manifest_entries: manifest_entries.len(),
    })
}

fn payload_summary_json(summary: &domain44::Domain44GateSummary) -> serde_json::Value {
    json!({
        "data_rows_total": summary.data_rows_total,
        "province_rows_total": summary.province_rows_total,
        "division_rows_total": summary.division_rows_total,
        "badge_rows_total": summary.badge_rows_total,
        "csupply_input_rows_total": summary.csupply_input_rows_total,
        "pressure_candidate_rows_total": summary.pressure_candidate_rows_total,
        "observed_province_roots": summary.observed_province_roots,
        "observed_division_roots": summary.observed_division_roots,
        "status_flags_or": summary.status_flags_or,
        "data_units_total": summary.data_units_total,
        "parse_errors": summary.parse_errors,
    })
}

fn proof_markdown(proof: &serde_json::Value) -> String {
    let gate = &proof["stream_gate"];
    let payload = &proof["payload_gate"];
    format!(
        "# supply_logistics_attrition_live replay pack proof\n\n\
         - result: `{}`\n\
         - domain_id: `{}`\n\
         - hoi4replay_path: `{}`\n\
         - hoi4replay_bytes: `{}`\n\
         - collector_stream_bytes: `{}`\n\
         - domain_data_records: `{}`\n\
         - frame_end_records: `{}`\n\
         - unique_game_hours: `{}`\n\
         - skipped_game_hours: `{}`\n\
         - data_rows_total: `{}`\n\
         - csupply_input_rows_total: `{}`\n\
         - pressure_candidate_rows_total: `{}`\n\
         - parse_errors: `{}`\n\n\
         ## Verdict\n\n\
         - The lane-local compact stream was packed into `.battleplan`, reopened with `snapshot::Container`, byte-verified, and domain 44 decoded from the embedded stream.\n\
         - This proves replay packing for lane-local domain 44 compact rows. Candidate pressure rows are not a production capture and do not remove the mandatory convoy-pressure badge.\n",
        proof["result"].as_str().unwrap_or("unknown"),
        proof["domain_id"].as_u64().unwrap_or(0),
        proof["hoi4replay_path"].as_str().unwrap_or("none"),
        proof["hoi4replay_bytes"].as_u64().unwrap_or(0),
        proof["collector_stream_bytes"].as_u64().unwrap_or(0),
        gate["domain_data_records"].as_u64().unwrap_or(0),
        gate["frame_end_records"].as_u64().unwrap_or(0),
        gate["unique_game_hours"].as_u64().unwrap_or(0),
        gate["skipped_game_hours"].as_u64().unwrap_or(0),
        payload["data_rows_total"].as_u64().unwrap_or(0),
        payload["csupply_input_rows_total"].as_u64().unwrap_or(0),
        payload["pressure_candidate_rows_total"].as_u64().unwrap_or(0),
        payload["parse_errors"].as_array().map_or(0, Vec::len),
    )
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("hoi4replay.tmp");
    fs::write(&tmp, bytes).map_err(|err| format!("write replay tmp failed: {err}"))?;
    if path.exists() {
        fs::remove_file(path).map_err(|err| format!("replace replay failed: {err}"))?;
    }
    fs::rename(&tmp, path).map_err(|err| format!("finalize replay failed: {err}"))
}

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn read_i32_le(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn created_label() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("unix_seconds:{secs}")
}

#[derive(Default)]
struct StreamGate {
    ok: bool,
    total_bytes: usize,
    record_count: u64,
    domain_data_records: u64,
    frame_end_records: u64,
    unique_game_hours: u64,
    skipped_game_hours: u64,
    data_rows_total: u64,
    province_rows_total: u64,
    division_rows_total: u64,
    badge_rows_total: u64,
    csupply_input_rows_total: u64,
    pressure_candidate_rows_total: u64,
    status_flags_or: u32,
    problems: Vec<String>,
}

impl StreamGate {
    fn to_json(&self) -> serde_json::Value {
        json!({
            "ok": self.ok,
            "total_bytes": self.total_bytes,
            "record_count": self.record_count,
            "domain_data_records": self.domain_data_records,
            "frame_end_records": self.frame_end_records,
            "unique_game_hours": self.unique_game_hours,
            "skipped_game_hours": self.skipped_game_hours,
            "data_rows_total": self.data_rows_total,
            "province_rows_total": self.province_rows_total,
            "division_rows_total": self.division_rows_total,
            "badge_rows_total": self.badge_rows_total,
            "csupply_input_rows_total": self.csupply_input_rows_total,
            "pressure_candidate_rows_total": self.pressure_candidate_rows_total,
            "status_flags_or": self.status_flags_or,
            "problems": self.problems,
        })
    }
}

struct Verification {
    stream_entry: String,
    stream_bytes: usize,
    collector_stream_entries: usize,
    recorder_manifest_entries: usize,
}

struct Options {
    payload: PathBuf,
    out_dir: PathBuf,
    game_id: String,
}

impl Options {
    fn parse() -> Result<Self, String> {
        let mut payload = None;
        let mut out_dir = None;
        let mut game_id = DEFAULT_GAME_ID.to_string();
        let args = env::args().skip(1).collect::<Vec<_>>();
        let mut i = 0usize;
        while i < args.len() {
            match args[i].as_str() {
                "--payload" => {
                    i += 1;
                    payload = Some(PathBuf::from(required(args.get(i), "--payload")?));
                }
                "--out-dir" => {
                    i += 1;
                    out_dir = Some(PathBuf::from(required(args.get(i), "--out-dir")?));
                }
                "--game-id" => {
                    i += 1;
                    game_id = required(args.get(i), "--game-id")?;
                    validate_game_id(&game_id)?;
                }
                "--help" | "-h" => return Err(usage()),
                other => return Err(format!("unknown argument {other}\n{}", usage())),
            }
            i += 1;
        }
        Ok(Self {
            payload: payload.ok_or_else(usage)?,
            out_dir: out_dir.ok_or_else(usage)?,
            game_id,
        })
    }
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

fn required(value: Option<&String>, flag: &str) -> Result<String, String> {
    value
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn usage() -> String {
    "usage: replay_pack_proof --payload <domain44_payload.bin> --out-dir <proof-dir> [--game-id <id>]".to_string()
}
