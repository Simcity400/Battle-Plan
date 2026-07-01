use characters_leaders_cabinet_live_lane::domain47_characters_leaders_cabinet_live as compact;
use characters_leaders_cabinet_live_lane::extractor::validate_registry_assignment;
use snapshot::{pack, Container, ExpectedIdentity, PackEntry, PackIdentity};
use std::fs;
use std::path::{Path, PathBuf};

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

    let stream = if source.wrapped_record {
        input_bytes.clone()
    } else {
        compact::encode_compact_data_record(1, source.game_hour_raw, source.payload)
    };
    let game_id = "characters_leaders_cabinet_live_domain47_record";
    let stream_path = "collector_raw/collector_stream.bin";
    let manifest_path = "recorder/native_session_manifest.json";
    let replay_path = out_dir.join("native_session.battleplan");
    let stream_fs_path = out_dir.join("collector_stream.bin");
    let manifest_fs_path = out_dir.join("native_session_manifest.json");
    fs::write(&stream_fs_path, &stream).map_err(|err| format!("write stream failed: {err}"))?;

    let manifest = format!(
        "{{\n  \"schema\": \"characters_leaders_cabinet_live.domain47.record_manifest.v1\",\n  \"domain\": \"{}\",\n  \"domain_id\": {},\n  \"stream_format\": \"compact\",\n  \"record_type\": \"DOMAIN_DATA\",\n  \"scheduled_domains\": [\"{}\"],\n  \"source_record\": \"{}\",\n  \"wrapped_input_record\": {},\n  \"sequence\": {},\n  \"game_hour_raw\": {},\n  \"compact\": {{\n    \"payload_bytes\": {},\n    \"record_bytes\": {},\n    \"row_count_total\": {},\n    \"character_rows\": {},\n    \"unit_leader_rows\": {},\n    \"unit_leader_trait_rows\": {},\n    \"country_leader_role_rows\": {},\n    \"active_ruler_rows\": {},\n    \"advisor_roster_rows\": {},\n    \"appointed_advisor_rows\": {},\n    \"character_portrait_rows\": {},\n    \"status_flags_or\": {},\n    \"schema_flags_or\": {}\n  }},\n  \"source\": \"lane-local existing domain-47 compact record, not shared recorder capture\"\n}}\n",
        compact::DOMAIN_NAME,
        compact::DOMAIN_ID,
        compact::DOMAIN_NAME,
        json_path(&input),
        source.wrapped_record,
        source.sequence,
        source.game_hour_raw,
        source.payload.len(),
        stream.len(),
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
        "{{\n  \"result\": \"verified\",\n  \"domain\": \"{}\",\n  \"domain_id\": {},\n  \"source_record\": \"{}\",\n  \"wrapped_input_record\": {},\n  \"sequence\": {},\n  \"game_hour_raw\": {},\n  \"hoi4replay_path\": \"{}\",\n  \"collector_stream_path\": \"{}\",\n  \"manifest_path\": \"{}\",\n  \"hoi4replay_bytes\": {},\n  \"collector_stream_bytes\": {},\n  \"payload_bytes\": {},\n  \"row_count_total\": {},\n  \"character_rows\": {},\n  \"unit_leader_rows\": {},\n  \"unit_leader_trait_rows\": {},\n  \"country_leader_role_rows\": {},\n  \"active_ruler_rows\": {},\n  \"advisor_roster_rows\": {},\n  \"appointed_advisor_rows\": {},\n  \"character_portrait_rows\": {},\n  \"schema_flags_or\": {},\n  \"status_flags_or\": {},\n  \"container_game_id\": \"{}\",\n  \"verified_stream_entry\": \"{}\",\n  \"verified_stream_bytes\": {},\n  \"parse_errors\": []\n}}\n",
        compact::DOMAIN_NAME,
        compact::DOMAIN_ID,
        json_path(&input),
        source.wrapped_record,
        source.sequence,
        source.game_hour_raw,
        json_path(&replay_path),
        json_path(&stream_fs_path),
        json_path(&manifest_fs_path),
        packed.len(),
        stream.len(),
        source.payload.len(),
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
    sequence: u64,
    game_hour_raw: i32,
}

fn payload_from_input(bytes: &[u8]) -> Result<PayloadSource<'_>, String> {
    if bytes.len() >= compact::COMPACT_RECORD_HEADER_LEN
        && &bytes[0..4] == compact::COMPACT_RECORD_MAGIC
    {
        let version = read_u16(bytes, 4);
        let header_len = read_u16(bytes, 6) as usize;
        let sequence = read_u64(bytes, 8);
        let game_hour_raw = read_i32(bytes, 16);
        let domain_id = read_u16(bytes, 20);
        let record_type = read_u16(bytes, 22);
        let payload_len = read_u32(bytes, 24) as usize;
        if version != compact::COMPACT_RECORD_VERSION {
            return Err(format!("unsupported compact record version {version}"));
        }
        if header_len != compact::COMPACT_RECORD_HEADER_LEN {
            return Err(format!(
                "header_len {header_len} != {}",
                compact::COMPACT_RECORD_HEADER_LEN
            ));
        }
        if domain_id != compact::DOMAIN_ID {
            return Err(format!("domain_id {domain_id} != {}", compact::DOMAIN_ID));
        }
        if record_type != compact::COMPACT_RECORD_TYPE_DOMAIN_DATA {
            return Err(format!(
                "record_type {record_type} != {}",
                compact::COMPACT_RECORD_TYPE_DOMAIN_DATA
            ));
        }
        if bytes.len() != header_len + payload_len {
            return Err(format!(
                "record payload length {payload_len} plus header {header_len} != file bytes {}",
                bytes.len()
            ));
        }
        return Ok(PayloadSource {
            payload: &bytes[header_len..],
            wrapped_record: true,
            sequence,
            game_hour_raw,
        });
    }

    Ok(PayloadSource {
        payload: bytes,
        wrapped_record: false,
        sequence: 1,
        game_hour_raw: 0,
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
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
