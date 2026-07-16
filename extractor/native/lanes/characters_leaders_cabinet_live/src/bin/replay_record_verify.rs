use characters_leaders_cabinet_live_lane::domain47_characters_leaders_cabinet_live as compact;
use characters_leaders_cabinet_live_lane::extractor::validate_registry_assignment;
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

    let mut args = std::env::args().skip(1);
    let input = args.next().map(PathBuf::from).ok_or_else(|| {
        "usage: replay_record_verify <domain47_record_or_payload.bin> <out-dir>".to_string()
    })?;
    let out_dir = args.next().map(PathBuf::from).ok_or_else(|| {
        "usage: replay_record_verify <domain47_record_or_payload.bin> <out-dir>".to_string()
    })?;
    if args.next().is_some() {
        return Err(
            "usage: replay_record_verify <domain47_record_or_payload.bin> <out-dir>".to_string(),
        );
    }

    fs::create_dir_all(&out_dir).map_err(|err| format!("create out dir failed: {err}"))?;
    let input_bytes =
        fs::read(&input).map_err(|err| format!("read {} failed: {err}", input.display()))?;
    let source = payload_from_input(&input_bytes)?;
    let summary = compact::decode_payload(0, source.payload);
    if !summary.parse_errors.is_empty() {
        return Err(format!(
            "domain47 record payload parse errors: {:?}",
            summary.parse_errors
        ));
    }

    let proof_hour = source.game_hour_raw.unwrap_or(0);
    let stream = encode_domain47_proof_stream(proof_hour, source.payload)?;
    let stream_gate = verify_domain47_stream(&stream);
    if !stream_gate.parse_errors.is_empty() {
        return Err(format!(
            "domain47 proof stream gate failed: {:?}",
            stream_gate.parse_errors
        ));
    }
    let game_id = "characters_leaders_cabinet_live_domain47_record";
    let stream_path = "collector_raw/collector_stream.bin";
    let manifest_path = "recorder/native_session_manifest.json";
    let replay_path = out_dir.join("native_session.battleplan");
    let stream_fs_path = out_dir.join("collector_stream.bin");
    let manifest_fs_path = out_dir.join("native_session_manifest.json");
    fs::write(&stream_fs_path, &stream).map_err(|err| format!("write stream failed: {err}"))?;

    let manifest = format!(
        "{{\n  \"schema\": \"characters_leaders_cabinet_live.domain47.record_fragment_manifest.v3\",\n  \"artifact_kind\": \"record_fragment_synthetic_envelope\",\n  \"production_certifying\": false,\n  \"source_capture_verified\": false,\n  \"domain\": \"{}\",\n  \"domain_id\": {},\n  \"stream_format\": \"compact\",\n  \"synthetic_record_types\": [\"DOMAIN_PERF\", \"DOMAIN_DATA\", \"FRAME_END\"],\n  \"synthetic_scheduled_domains\": [\"{}\"],\n  \"source_record\": \"{}\",\n  \"wrapped_input_record\": {},\n  \"source_sequence\": {},\n  \"source_game_hour_raw\": {},\n  \"source_ordering_claimed\": false,\n  \"synthetic_proof_game_hour_raw\": {},\n  \"synthetic_envelope\": {{\n    \"payload_bytes\": {},\n    \"stream_bytes\": {},\n    \"record_count\": {},\n    \"domain_perf_record_count\": {},\n    \"domain_data_record_count\": {},\n    \"frame_end_record_count\": {},\n    \"closed_game_hours\": {:?},\n    \"row_count_total\": {},\n    \"character_rows\": {},\n    \"unit_leader_rows\": {},\n    \"unit_leader_trait_rows\": {},\n    \"country_leader_role_rows\": {},\n    \"active_ruler_rows\": {},\n    \"advisor_roster_rows\": {},\n    \"appointed_advisor_rows\": {},\n    \"character_portrait_rows\": {},\n    \"status_flags_or\": {},\n    \"schema_flags_or\": {}\n  }},\n  \"proof_scope\": \"record-fragment decode plus synthetic envelope and container integrity only; source ordering and production capture are not certified\"\n}}\n",
        compact::DOMAIN_NAME,
        compact::DOMAIN_ID,
        compact::DOMAIN_NAME,
        json_path(&input),
        source.wrapped_record,
        json_optional_u64(source.sequence),
        json_optional_i32(source.game_hour_raw),
        proof_hour,
        source.payload.len(),
        stream.len(),
        stream_gate.record_count,
        stream_gate.domain_perf_record_count,
        stream_gate.domain_data_record_count,
        stream_gate.frame_end_record_count,
        stream_gate.closed_game_hours,
        summary.row_count_total,
        summary.character_rows,
        summary.unit_leader_rows,
        summary.trait_rows,
        summary.country_leader_role_rows,
        summary.active_ruler_rows,
        summary.advisor_roster_rows,
        summary.appointed_advisor_rows,
        summary.character_portrait_rows,
        summary.status_flags_or,
        summary.schema_flags_or,
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
        created_utc: Some("domain47_record_verify".to_string()),
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

    let report = format!(
        "{{\n  \"result\": \"{}\",\n  \"artifact_kind\": \"record_fragment_synthetic_envelope\",\n  \"production_certifying\": false,\n  \"source_capture_verified\": false,\n  \"container_integrity_verified\": true,\n  \"stream_entry_byte_identity_verified\": true,\n  \"domain\": \"{}\",\n  \"domain_id\": {},\n  \"source_record\": \"{}\",\n  \"wrapped_input_record\": {},\n  \"source_sequence\": {},\n  \"source_game_hour_raw\": {},\n  \"source_ordering_claimed\": false,\n  \"synthetic_proof_game_hour_raw\": {},\n  \"hoi4replay_path\": \"{}\",\n  \"collector_stream_path\": \"{}\",\n  \"manifest_path\": \"{}\",\n  \"hoi4replay_bytes\": {},\n  \"synthetic_stream_bytes\": {},\n  \"payload_bytes\": {},\n  \"synthetic_record_count\": {},\n  \"synthetic_domain_perf_record_count\": {},\n  \"synthetic_domain_data_record_count\": {},\n  \"synthetic_frame_end_record_count\": {},\n  \"synthetic_closed_game_hours\": {:?},\n  \"row_count_total\": {},\n  \"character_rows\": {},\n  \"unit_leader_rows\": {},\n  \"unit_leader_trait_rows\": {},\n  \"country_leader_role_rows\": {},\n  \"active_ruler_rows\": {},\n  \"advisor_roster_rows\": {},\n  \"appointed_advisor_rows\": {},\n  \"character_portrait_rows\": {},\n  \"schema_flags_or\": {},\n  \"status_flags_or\": {},\n  \"container_game_id\": \"{}\",\n  \"verified_stream_entry\": \"{}\",\n  \"verified_stream_bytes\": {},\n  \"proof_scope\": \"record-fragment decode plus synthetic envelope and container integrity only; source ordering and production capture are not certified\",\n  \"parse_errors\": []\n}}\n",
        NON_CERTIFYING_RESULT,
        compact::DOMAIN_NAME,
        compact::DOMAIN_ID,
        json_path(&input),
        source.wrapped_record,
        json_optional_u64(source.sequence),
        json_optional_i32(source.game_hour_raw),
        proof_hour,
        json_path(&replay_path),
        json_path(&stream_fs_path),
        json_path(&manifest_fs_path),
        packed.len(),
        stream.len(),
        source.payload.len(),
        stream_gate.record_count,
        stream_gate.domain_perf_record_count,
        stream_gate.domain_data_record_count,
        stream_gate.frame_end_record_count,
        stream_gate.closed_game_hours,
        summary.row_count_total,
        summary.character_rows,
        summary.unit_leader_rows,
        summary.trait_rows,
        summary.country_leader_role_rows,
        summary.active_ruler_rows,
        summary.advisor_roster_rows,
        summary.appointed_advisor_rows,
        summary.character_portrait_rows,
        summary.schema_flags_or,
        summary.status_flags_or,
        container.game_id(),
        packed_stream_path,
        verified_stream.len(),
    );
    fs::write(out_dir.join("replay_record_verify.json"), report.as_bytes())
        .map_err(|err| format!("write report failed: {err}"))?;
    print!("{report}");
    Ok(())
}

struct PayloadSource<'a> {
    payload: &'a [u8],
    wrapped_record: bool,
    sequence: Option<u64>,
    game_hour_raw: Option<i32>,
}

fn payload_from_input(bytes: &[u8]) -> Result<PayloadSource<'_>, String> {
    if bytes.starts_with(compact::COMPACT_RECORD_MAGIC) {
        let header = compact_contract::parse_compact_record_header(bytes)
            .map_err(|err| format!("invalid compact record header: {err}"))?;
        if header.domain_id != compact::DOMAIN_ID {
            return Err(format!(
                "domain_id {} != {}",
                header.domain_id,
                compact::DOMAIN_ID
            ));
        }
        if header.record_type != compact::COMPACT_RECORD_TYPE_DOMAIN_DATA {
            return Err(format!(
                "record_type {} != {}",
                header.record_type,
                compact::COMPACT_RECORD_TYPE_DOMAIN_DATA
            ));
        }
        if header.sequence == 0 {
            return Err("sequence 0 is not canonical H4CR record authority".to_string());
        }
        if bytes.len() != header.record_len() {
            return Err(format!(
                "record payload length {} plus header {} != file bytes {}",
                header.payload_len,
                compact::COMPACT_RECORD_HEADER_LEN,
                bytes.len()
            ));
        }
        compact_contract::validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            &bytes[compact::COMPACT_RECORD_HEADER_LEN..],
        )
        .map_err(|error| format!("invalid compact record semantics: {error}"))?;
        return Ok(PayloadSource {
            payload: &bytes[compact::COMPACT_RECORD_HEADER_LEN..],
            wrapped_record: true,
            sequence: Some(header.sequence),
            game_hour_raw: Some(header.game_hour_raw),
        });
    }

    Ok(PayloadSource {
        payload: bytes,
        wrapped_record: false,
        sequence: None,
        game_hour_raw: None,
    })
}

fn json_optional_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "null".to_string(), |value| value.to_string())
}

fn json_optional_i32(value: Option<i32>) -> String {
    value.map_or_else(|| "null".to_string(), |value| value.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn wrapped_record() -> Vec<u8> {
        let payload = compact::encode_payload(0, 0, compact::SCHEMA_ROOTS_VALIDATED, &[]);
        compact::encode_compact_data_record(7, 123, &payload)
    }

    #[test]
    fn wrapped_fragment_requires_canonical_data_record_semantics() {
        let valid = wrapped_record();
        let source = payload_from_input(&valid).unwrap();
        assert_eq!(source.sequence, Some(7));
        assert_eq!(source.game_hour_raw, Some(123));

        let mut flags = valid.clone();
        flags[28..32].copy_from_slice(&1u32.to_le_bytes());
        assert!(payload_from_input(&flags).is_err());

        let mut record_type = valid;
        record_type[22..24].copy_from_slice(&99u16.to_le_bytes());
        assert!(payload_from_input(&record_type).is_err());

        let mut zero_sequence = wrapped_record();
        zero_sequence[8..16].copy_from_slice(&0u64.to_le_bytes());
        assert!(payload_from_input(&zero_sequence).is_err());
    }

    #[test]
    fn result_vocabulary_is_explicitly_non_certifying() {
        assert_eq!(
            NON_CERTIFYING_RESULT,
            "container_integrity_verified_non_certifying"
        );
    }
}
