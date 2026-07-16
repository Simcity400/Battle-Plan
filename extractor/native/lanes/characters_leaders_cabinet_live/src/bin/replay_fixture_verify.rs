use characters_leaders_cabinet_live_lane::domain47_characters_leaders_cabinet_live as compact;
use characters_leaders_cabinet_live_lane::extractor::{
    appointed_advisors_from_active_ideas, encode_frame_payload, validate_registry_assignment,
    AdvisorRecord, LaneFrame,
};
use characters_leaders_cabinet_live_lane::stream_gate::{
    encode_domain47_proof_stream, verify_domain47_stream,
};
use snapshot::{pack, Container, ExpectedIdentity, PackEntry, PackIdentity};
use std::fs;
use std::path::{Path, PathBuf};

const NON_CERTIFYING_RESULT: &str = "container_integrity_verified_non_certifying";

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    validate_registry_assignment(include_str!("../../../../domain_registry.json"))
        .map_err(|err| err.to_string())?;

    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                "extractor/native/runs/characters_leaders_cabinet_live_20260701_replay_fixture_v1",
            )
        });
    fs::create_dir_all(&out_dir).map_err(|err| format!("create out dir failed: {err}"))?;

    let mut frame = LaneFrame::new(0);
    frame.advisor_roster = eng_fixture_roster();
    frame.appointed_advisors = appointed_advisors_from_active_ideas(
        &frame.advisor_roster,
        &[
            "ENG_leslie_hore_belisha",
            "ENG_maxwell_aitken",
            "ENG_philip_kerr",
        ],
    );
    let payload = encode_frame_payload(&frame);
    let decoded = compact::decode_payload(0, &payload);
    if !decoded.parse_errors.is_empty() {
        return Err(format!(
            "domain47 fixture payload parse errors: {:?}",
            decoded.parse_errors
        ));
    }
    let stream = encode_domain47_proof_stream(1946051909, &payload)?;
    let stream_gate = verify_domain47_stream(&stream);
    if !stream_gate.parse_errors.is_empty() {
        return Err(format!(
            "domain47 fixture stream gate failed: {:?}",
            stream_gate.parse_errors
        ));
    }

    let game_id = "characters_leaders_cabinet_live_domain47_fixture";
    let stream_path = "collector_raw/collector_stream.bin";
    let manifest_path = "recorder/native_session_manifest.json";
    let replay_path = out_dir.join("native_session.battleplan");
    let stream_fs_path = out_dir.join("collector_stream.bin");
    let manifest_fs_path = out_dir.join("native_session_manifest.json");
    fs::write(&stream_fs_path, &stream).map_err(|err| format!("write stream failed: {err}"))?;

    let manifest = format!(
        "{{\n  \"schema\": \"characters_leaders_cabinet_live.domain47.synthetic_fixture_manifest.v3\",\n  \"artifact_kind\": \"lane_local_synthetic_fixture_stream\",\n  \"production_certifying\": false,\n  \"source_capture_verified\": false,\n  \"domain\": \"{}\",\n  \"domain_id\": {},\n  \"stream_format\": \"compact\",\n  \"synthetic_record_types\": [\"DOMAIN_PERF\", \"DOMAIN_DATA\", \"FRAME_END\"],\n  \"synthetic_scheduled_domains\": [\"{}\"],\n  \"synthetic_stream\": {{\n    \"payload_bytes\": {},\n    \"stream_bytes\": {},\n    \"record_count\": {},\n    \"domain_perf_record_count\": {},\n    \"domain_data_record_count\": {},\n    \"frame_end_record_count\": {},\n    \"closed_game_hours\": {:?},\n    \"row_count_total\": {},\n    \"advisor_roster_rows\": {},\n    \"appointed_advisor_rows\": {},\n    \"character_portrait_rows\": {},\n    \"status_flags_or\": {},\n    \"schema_flags_or\": {}\n  }},\n  \"proof_scope\": \"synthetic fixture stream and container integrity only; not source capture or production certification\"\n}}\n",
        compact::DOMAIN_NAME,
        compact::DOMAIN_ID,
        compact::DOMAIN_NAME,
        payload.len(),
        stream.len(),
        stream_gate.record_count,
        stream_gate.domain_perf_record_count,
        stream_gate.domain_data_record_count,
        stream_gate.frame_end_record_count,
        stream_gate.closed_game_hours,
        decoded.row_count_total,
        decoded.advisor_roster_rows,
        decoded.appointed_advisor_rows,
        decoded.character_portrait_rows,
        decoded.status_flags_or,
        decoded.schema_flags_or,
    );
    fs::write(&manifest_fs_path, manifest.as_bytes())
        .map_err(|err| format!("write manifest failed: {err}"))?;

    let index_bytes = format!(
        "{{\n  \"game_id\": \"{}\",\n  \"schema_version\": \"native_collector_session_v1\",\n  \"snapshots\": []\n}}\n",
        game_id
    )
    .into_bytes();
    let timeline_bytes = format!(
        "{{\n  \"format\": \"hoi4_compact_stream_timeline_v1\",\n  \"stream_format\": \"compact\",\n  \"stream\": \"{}\",\n  \"session_manifest\": \"{}\",\n  \"raw_stream_role\": \"collector_compact_stream\",\n  \"session_manifest_role\": \"recorder_manifest\"\n}}\n",
        stream_path, manifest_path
    )
    .into_bytes();

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
            path: format!("payload/{game_id}/{stream_path}"),
            bytes: stream.clone(),
            role: "collector_compact_stream",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/{manifest_path}"),
            bytes: manifest.into_bytes(),
            role: "recorder_manifest",
            hour_index: None,
        },
    ];
    let identity = PackIdentity {
        schema_version: Some("native_collector_session_v1".to_string()),
        created_utc: Some("domain47_fixture".to_string()),
        ..PackIdentity::default()
    };
    let packed = pack(game_id, &entries, &identity, true)
        .map_err(|err| format!("pack replay container failed: {err}"))?;
    let tmp = replay_path.with_extension("hoi4replay.tmp");
    fs::write(&tmp, &packed).map_err(|err| format!("write replay tmp failed: {err}"))?;
    if replay_path.exists() {
        fs::remove_file(&replay_path).map_err(|err| format!("replace replay failed: {err}"))?;
    }
    fs::rename(&tmp, &replay_path).map_err(|err| format!("finalize replay failed: {err}"))?;

    let container = Container::open(&replay_path, &ExpectedIdentity::default())
        .map_err(|err| format!("verify replay open failed: {err}"))?;
    container
        .verify_all()
        .map_err(|err| format!("verify replay entries failed: {err}"))?;
    let packed_stream_path = format!("payload/{game_id}/{stream_path}");
    let verified_stream = container
        .read_entry_bytes(&packed_stream_path)
        .map_err(|err| format!("verify stream entry failed: {err}"))?;
    if verified_stream != stream {
        return Err("verify stream entry failed: compact bytes changed".to_string());
    }

    let summary = format!(
        "{{\n  \"result\": \"{}\",\n  \"artifact_kind\": \"lane_local_synthetic_fixture_stream\",\n  \"production_certifying\": false,\n  \"source_capture_verified\": false,\n  \"container_integrity_verified\": true,\n  \"stream_entry_byte_identity_verified\": true,\n  \"domain\": \"{}\",\n  \"domain_id\": {},\n  \"hoi4replay_path\": \"{}\",\n  \"collector_stream_path\": \"{}\",\n  \"manifest_path\": \"{}\",\n  \"hoi4replay_bytes\": {},\n  \"synthetic_stream_bytes\": {},\n  \"payload_bytes\": {},\n  \"synthetic_record_count\": {},\n  \"synthetic_domain_perf_record_count\": {},\n  \"synthetic_domain_data_record_count\": {},\n  \"synthetic_frame_end_record_count\": {},\n  \"synthetic_closed_game_hours\": {:?},\n  \"row_count_total\": {},\n  \"advisor_roster_rows\": {},\n  \"appointed_advisor_rows\": {},\n  \"character_portrait_rows\": {},\n  \"schema_flags_or\": {},\n  \"status_flags_or\": {},\n  \"container_game_id\": \"{}\",\n  \"verified_stream_entry\": \"{}\",\n  \"verified_stream_bytes\": {},\n  \"proof_scope\": \"synthetic fixture stream and container integrity only; not source capture or production certification\",\n  \"parse_errors\": []\n}}\n",
        NON_CERTIFYING_RESULT,
        compact::DOMAIN_NAME,
        compact::DOMAIN_ID,
        json_path(&replay_path),
        json_path(&stream_fs_path),
        json_path(&manifest_fs_path),
        packed.len(),
        stream.len(),
        payload.len(),
        stream_gate.record_count,
        stream_gate.domain_perf_record_count,
        stream_gate.domain_data_record_count,
        stream_gate.frame_end_record_count,
        stream_gate.closed_game_hours,
        decoded.row_count_total,
        decoded.advisor_roster_rows,
        decoded.appointed_advisor_rows,
        decoded.character_portrait_rows,
        decoded.schema_flags_or,
        decoded.status_flags_or,
        container.game_id(),
        packed_stream_path,
        verified_stream.len(),
    );
    let summary_path = out_dir.join("replay_fixture_verify.json");
    fs::write(&summary_path, summary.as_bytes())
        .map_err(|err| format!("write summary failed: {err}"))?;
    print!("{summary}");
    Ok(())
}

fn eng_fixture_roster() -> Vec<AdvisorRecord> {
    [
        (1459, "ENG_leslie_hore_belisha"),
        (1458, "ENG_maxwell_aitken"),
        (1456, "ENG_philip_kerr"),
    ]
    .into_iter()
    .map(|(char_id, token)| AdvisorRecord {
        country_tag_db_id: 2,
        char_id,
        slot: Some("political_advisor".to_string()),
        ledger: Some("political_advisor".to_string()),
        idea_token: token.to_string(),
    })
    .collect()
}

fn json_path(path: &Path) -> String {
    json_escape(&path.to_string_lossy())
}

fn json_escape(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}
