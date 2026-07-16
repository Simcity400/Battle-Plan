use serde_json::json;
use session_multiplayer_metadata_live::collector_module::{
    collect_domain57, SessionMetadataInputs,
};
use session_multiplayer_metadata_live::{
    append_compact_record, gate_compact_stream, PlayerCountryRow, SessionEventKind,
    SessionEventRow, EVENT_FLAG_DIRECT_CAPTURE, EVENT_FLAG_HOST_COMMAND, EVENT_FLAG_TEXT_PRESENT,
    EVENT_FLAG_UI_VISIBLE, FRAME_END_PAYLOAD_LEN, PLAYER_FLAG_CONNECTED, PLAYER_FLAG_HOST,
    PLAYER_FLAG_HUMAN, RECORD_TYPE_FRAME_END, SOURCE_FLAG_COMMAND_HOOK_VALIDATED,
    SOURCE_FLAG_DLL_NATIVE, SOURCE_FLAG_MESSAGE_HOOK_VALIDATED,
    SOURCE_FLAG_NOTIFICATION_HOOK_VALIDATED, SOURCE_FLAG_PROOF_FIXTURE,
    SOURCE_FLAG_SESSION_ROOT_VALIDATED,
};
use snapshot::{pack, Container, ExpectedIdentity, PackEntry, PackIdentity};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() -> Result<(), String> {
    let out_dir = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/session_multiplayer_metadata_live_proof"));
    fs::create_dir_all(&out_dir)
        .map_err(|err| format!("failed to create {}: {err}", out_dir.display()))?;

    let frame0 = collect_domain57(sample_inputs(43_800_024, 1));
    if !frame0.gate.ok {
        return Err(format!("payload gate failed: {:?}", frame0.gate.errors));
    }
    let frame1 = collect_domain57(sample_inputs(43_800_025, 2));
    if !frame1.gate.ok {
        return Err(format!("payload gate failed: {:?}", frame1.gate.errors));
    }

    let mut stream = Vec::new();
    append_compact_record(
        &mut stream,
        1,
        frame0.frame.game_hour_raw,
        session_multiplayer_metadata_live::DOMAIN_ID,
        session_multiplayer_metadata_live::RECORD_TYPE_DOMAIN_DATA,
        &frame0.payload,
    );
    append_compact_record(
        &mut stream,
        2,
        frame0.frame.game_hour_raw,
        1000,
        RECORD_TYPE_FRAME_END,
        &[0u8; FRAME_END_PAYLOAD_LEN],
    );
    append_compact_record(
        &mut stream,
        3,
        frame1.frame.game_hour_raw,
        session_multiplayer_metadata_live::DOMAIN_ID,
        session_multiplayer_metadata_live::RECORD_TYPE_DOMAIN_DATA,
        &frame1.payload,
    );
    append_compact_record(
        &mut stream,
        4,
        frame1.frame.game_hour_raw,
        1000,
        RECORD_TYPE_FRAME_END,
        &[0u8; FRAME_END_PAYLOAD_LEN],
    );

    let gate = gate_compact_stream(&stream);
    if !gate.ok {
        return Err(format!("stream gate failed: {:?}", gate.errors));
    }

    let stream_path = out_dir.join("session_multiplayer_metadata_live.compact.bin");
    fs::write(&stream_path, &stream)
        .map_err(|err| format!("failed to write {}: {err}", stream_path.display()))?;

    let manifest = json!({
        "schema": "session_multiplayer_metadata_live.proof_manifest.v1",
        "domain": session_multiplayer_metadata_live::DOMAIN_NAME,
        "domain_id": session_multiplayer_metadata_live::DOMAIN_ID,
        "stream_format": "compact",
        "production_capture_integrated": false,
        "proof_scope": "lane-local compact schema, collector adapter, decoder gate, and hoi4replay container proof",
        "scheduled_domains": [session_multiplayer_metadata_live::DOMAIN_NAME],
        "root_evidence_mask": "fixture-only; live roots remain open",
        "registration_patch_required": true,
        "shared_entrypoints_edited": false,
        "gate": {
            "ok": gate.ok,
            "record_count": gate.record_count,
            "domain_data_records": gate.domain_data_records,
            "frame_end_records": gate.frame_end_records,
            "zero_skipped_hours": gate.zero_skipped_hours,
            "skipped_hours": gate.skipped_hours,
            "partial_data_seen": gate.partial_data_seen,
            "source_unavailable_seen": gate.source_unavailable_seen,
            "worker_panic_seen": gate.worker_panic_seen,
            "bridge_unavailable_seen": gate.bridge_unavailable_seen,
            "player_country_rows": gate.payload_summary.player_country_rows,
            "event_rows": gate.payload_summary.event_rows,
        }
    });
    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest).map_err(|err| format!("json error: {err}"))?;
    let manifest_path = out_dir.join("native_session_manifest.json");
    fs::write(&manifest_path, &manifest_bytes)
        .map_err(|err| format!("failed to write {}: {err}", manifest_path.display()))?;

    let replay_path = out_dir.join("session_multiplayer_metadata_live.battleplan");
    pack_and_verify(&replay_path, &stream, &manifest_bytes)?;

    let proof = json!({
        "schema": "session_multiplayer_metadata_live.proof.v1",
        "result": "verified",
        "domain": session_multiplayer_metadata_live::DOMAIN_NAME,
        "domain_id": session_multiplayer_metadata_live::DOMAIN_ID,
        "compact_stream_path": path_slash(&stream_path),
        "manifest_path": path_slash(&manifest_path),
        "hoi4replay_path": path_slash(&replay_path),
        "compact_stream_bytes": stream.len(),
        "hoi4replay_verified": true,
        "gate": manifest["gate"],
        "rows_covered": [
            "player-country mapping",
            "hotjoin/tag switches",
            "game speed/pause commands",
            "chat/messages",
            "alerts/notifications"
        ],
        "live_root_status": {
            "player_country_mapping": "schema_and_adapter_ready_root_open",
            "hotjoin_tag_switches": "schema_and_adapter_ready_hook_open",
            "game_speed_pause_commands": "schema_and_adapter_ready_command_hook_open",
            "chat_messages": "schema_and_adapter_ready_message_hook_open",
            "alerts_notifications": "schema_and_adapter_ready_notification_hook_open"
        }
    });
    let proof_path = out_dir.join("session_multiplayer_metadata_live_proof.json");
    fs::write(
        &proof_path,
        serde_json::to_vec_pretty(&proof).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("failed to write {}: {err}", proof_path.display()))?;
    println!("{}", proof_path.display());
    Ok(())
}

fn sample_inputs(game_hour_raw: i32, sequence_base: u64) -> SessionMetadataInputs {
    SessionMetadataInputs {
        game_hour_raw,
        source_flags: SOURCE_FLAG_DLL_NATIVE
            | SOURCE_FLAG_SESSION_ROOT_VALIDATED
            | SOURCE_FLAG_COMMAND_HOOK_VALIDATED
            | SOURCE_FLAG_MESSAGE_HOOK_VALIDATED
            | SOURCE_FLAG_NOTIFICATION_HOOK_VALIDATED
            | SOURCE_FLAG_PROOF_FIXTURE,
        root_evidence_mask: 0x1F,
        player_country_rows: vec![
            PlayerCountryRow {
                player_key_hash: 0xA11C_E001,
                player_name: "host".to_string(),
                country_tag: "GER".to_string(),
                tag_db_id: 1,
                session_slot: 0,
                flags: PLAYER_FLAG_HOST | PLAYER_FLAG_HUMAN | PLAYER_FLAG_CONNECTED,
                valid_from_hour_raw: game_hour_raw,
            },
            PlayerCountryRow {
                player_key_hash: 0xB22D_E002,
                player_name: "ally".to_string(),
                country_tag: "ITA".to_string(),
                tag_db_id: 4,
                session_slot: 1,
                flags: PLAYER_FLAG_HUMAN | PLAYER_FLAG_CONNECTED,
                valid_from_hour_raw: game_hour_raw,
            },
        ],
        event_rows: vec![
            event(SessionEventKind::Hotjoin, game_hour_raw, sequence_base, ""),
            event(
                SessionEventKind::TagSwitch,
                game_hour_raw,
                sequence_base + 1,
                "",
            ),
            SessionEventRow {
                kind: SessionEventKind::GameSpeed,
                value_i32: 4,
                flags: EVENT_FLAG_DIRECT_CAPTURE | EVENT_FLAG_UI_VISIBLE | EVENT_FLAG_HOST_COMMAND,
                ..event(
                    SessionEventKind::GameSpeed,
                    game_hour_raw,
                    sequence_base + 2,
                    "",
                )
            },
            SessionEventRow {
                kind: SessionEventKind::Pause,
                value_i32: 1,
                flags: EVENT_FLAG_DIRECT_CAPTURE | EVENT_FLAG_UI_VISIBLE | EVENT_FLAG_HOST_COMMAND,
                ..event(
                    SessionEventKind::Pause,
                    game_hour_raw,
                    sequence_base + 3,
                    "",
                )
            },
            event(
                SessionEventKind::Chat,
                game_hour_raw,
                sequence_base + 4,
                "front ready?",
            ),
            event(
                SessionEventKind::SystemMessage,
                game_hour_raw,
                sequence_base + 5,
                "player joined",
            ),
            event(
                SessionEventKind::Alert,
                game_hour_raw,
                sequence_base + 6,
                "low manpower",
            ),
            event(
                SessionEventKind::Notification,
                game_hour_raw,
                sequence_base + 7,
                "war goal available",
            ),
        ],
    }
}

fn event(
    kind: SessionEventKind,
    game_hour_raw: i32,
    command_sequence: u64,
    text: &str,
) -> SessionEventRow {
    let text_flag = if text.is_empty() {
        0
    } else {
        EVENT_FLAG_TEXT_PRESENT
    };
    SessionEventRow {
        kind,
        flags: EVENT_FLAG_DIRECT_CAPTURE | EVENT_FLAG_UI_VISIBLE | text_flag,
        actor_key_hash: 0xA11C_E001,
        actor_name: "host".to_string(),
        text: text.to_string(),
        tag_db_id: 1,
        value_i32: 0,
        related_value_i32: 4,
        command_sequence,
        event_hour_raw: game_hour_raw,
        event_source_id: kind as u32,
        event_state: 0,
    }
}

fn pack_and_verify(path: &Path, stream: &[u8], manifest: &[u8]) -> Result<(), String> {
    let game_id = "session_multiplayer_metadata_live_proof";
    let index_bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "game_id": game_id,
        "schema_version": "session_multiplayer_metadata_live.proof.v1",
        "snapshots": []
    }))
    .map_err(|err| format!("index json failed: {err}"))?;
    let timeline_bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "schema": "session_multiplayer_metadata_live.timeline.v1",
        "stream_format": "compact",
        "stream": "collector_raw/session_multiplayer_metadata_live.compact.bin",
        "session_manifest": "recorder/native_session_manifest.json",
        "raw_stream_role": "collector_compact_stream",
        "session_manifest_role": "recorder_manifest"
    }))
    .map_err(|err| format!("timeline json failed: {err}"))?;
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
            path: format!(
                "payload/{game_id}/collector_raw/session_multiplayer_metadata_live.compact.bin"
            ),
            bytes: stream.to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/recorder/native_session_manifest.json"),
            bytes: manifest.to_vec(),
            role: "recorder_manifest",
            hour_index: None,
        },
    ];
    let bytes = pack(
        game_id,
        &entries,
        &PackIdentity {
            schema_version: Some("native_collector_session_v1".to_string()),
            build_exe_sha256: None,
            build_pinned: false,
            mod_identity: None,
            created_utc: Some("session_multiplayer_metadata_live_proof".to_string()),
        },
        false,
    )
    .map_err(|err| format!("pack failed: {err}"))?;
    fs::write(path, bytes).map_err(|err| format!("failed to write {}: {err}", path.display()))?;
    let container = Container::open(path, &ExpectedIdentity::default())
        .map_err(|err| format!("container verify failed: {err}"))?;
    container
        .verify_all()
        .map_err(|err| format!("container verify_all failed: {err}"))?;
    if container.entries_by_role("collector_compact_stream").len() != 1 {
        return Err("collector_compact_stream entry count != 1".to_string());
    }
    if container.entries_by_role("recorder_manifest").len() != 1 {
        return Err("recorder_manifest entry count != 1".to_string());
    }
    Ok(())
}

fn path_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
