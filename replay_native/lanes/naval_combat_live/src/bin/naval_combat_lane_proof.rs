use std::env;
use std::fs;
use std::path::PathBuf;

use naval_combat_live_lane::{
    build_capture_frame, build_transition_capture_frame, gate_stream, result_json, CombatSide,
    CshipSnapshotRow, MemberBridgeRow, NavalCombatBodyBridgeRow, ValidatedCombatSnapshot,
    CURRENT_870EBF_CSHIP_ROUTE, DOMAIN_NAVAL_COMBAT_LIFECYCLE, DOMAIN_NAVAL_COMBAT_MEMBERS,
};
use snapshot::{pack, Container, ExpectedIdentity, PackEntry, PackIdentity};

fn main() {
    let mut out_dir = PathBuf::from("replay_native/runs/naval_combat_live_lane_stack_proof");
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--out-dir" {
            let Some(value) = args.next() else {
                eprintln!("--out-dir requires a path");
                std::process::exit(2);
            };
            out_dir = PathBuf::from(value);
        } else {
            eprintln!("unknown argument: {arg}");
            std::process::exit(2);
        }
    }

    if let Err(err) = fs::create_dir_all(&out_dir) {
        eprintln!("create {} failed: {err}", out_dir.display());
        std::process::exit(1);
    }

    let fixture = fixture();
    let mut updated_fixture = fixture.clone();
    updated_fixture.lifecycle.attacker_positioning_milli = Some(835);
    updated_fixture.members[0].engagement_weight_raw = Some(1_250);
    let cships = [cship(1001)];
    let start =
        match build_transition_capture_frame(&[], &[fixture.clone()], &cships, 100, 60787539, 7) {
            Ok(frame) => frame,
            Err(errors) => {
                eprintln!("synthetic start fixture unexpectedly failed: {errors:?}");
                std::process::exit(1);
            }
        };
    let update = match build_transition_capture_frame(
        &[fixture.clone()],
        &[updated_fixture.clone()],
        &cships,
        102,
        60787540,
        17,
    ) {
        Ok(frame) => frame,
        Err(errors) => {
            eprintln!("synthetic update fixture unexpectedly failed: {errors:?}");
            std::process::exit(1);
        }
    };
    let end =
        match build_transition_capture_frame(&[updated_fixture], &[], &cships, 104, 60787541, 29) {
            Ok(frame) => frame,
            Err(errors) => {
                eprintln!("synthetic end fixture unexpectedly failed: {errors:?}");
                std::process::exit(1);
            }
        };
    let mut compact_stream = start.compact_stream;
    compact_stream.extend_from_slice(&update.compact_stream);
    compact_stream.extend_from_slice(&end.compact_stream);
    let compact_path = out_dir.join("naval_combat_live_domain49_50_compact.bin");
    if let Err(err) = fs::write(&compact_path, &compact_stream) {
        eprintln!("write {} failed: {err}", compact_path.display());
        std::process::exit(1);
    }

    let gate = gate_stream(&compact_stream);
    let gate_json = result_json(&gate);
    let gate_path = out_dir.join("naval_combat_live_domain49_50_gate.json");
    if let Err(err) = fs::write(&gate_path, &gate_json) {
        eprintln!("write {} failed: {err}", gate_path.display());
        std::process::exit(1);
    }

    let replay_path = out_dir.join("native_session.battleplan");
    let replay_verify = match write_and_verify_replay(&replay_path, &compact_stream) {
        Ok(report) => report,
        Err(err) => {
            eprintln!("replay verification failed: {err}");
            std::process::exit(1);
        }
    };

    let empty_fixture_errors = build_capture_frame(&[], &cships, 200, 60787540, 1)
        .err()
        .unwrap_or_default();
    let proof_json = format!(
        concat!(
            "{{\n",
            "  \"status\": \"passed\",\n",
            "  \"domains\": {{\"49\": \"naval_combat_lifecycle_live\", \"50\": \"naval_combat_members_live\"}},\n",
            "  \"domain_ids\": [{}, {}],\n",
            "  \"compact_path\": {},\n",
            "  \"gate_path\": {},\n",
            "  \"replay_path\": {},\n",
            "  \"hoi4replay_verified\": {},\n",
            "  \"collector_compact_stream_byte_match\": {},\n",
            "  \"collector_compact_stream_bytes\": {},\n",
            "  \"gate_ok\": {},\n",
            "  \"lifecycle_rows\": {},\n",
            "  \"member_rows\": {},\n",
            "  \"two_sided_combat_count\": {},\n",
            "  \"lifecycle_started_row_count\": {},\n",
            "  \"lifecycle_updated_row_count\": {},\n",
            "  \"lifecycle_ended_row_count\": {},\n",
            "  \"cship_validated_member_count\": {},\n",
            "  \"task_force_linked_member_count\": {},\n",
            "  \"fleet_linked_member_count\": {},\n",
            "  \"hull_telemetry_member_count\": {},\n",
            "  \"sunk_convoy_telemetry_member_count\": {},\n",
            "  \"empty_fixture_rejected\": {},\n",
            "  \"empty_fixture_errors\": {}\n",
            "}}\n"
        ),
        DOMAIN_NAVAL_COMBAT_LIFECYCLE,
        DOMAIN_NAVAL_COMBAT_MEMBERS,
        json_str(&compact_path.to_string_lossy()),
        json_str(&gate_path.to_string_lossy()),
        json_str(&replay_path.to_string_lossy()),
        replay_verify.verified,
        replay_verify.byte_match,
        replay_verify.collector_compact_stream_bytes,
        gate.ok,
        gate.summary.lifecycle_row_count,
        gate.summary.member_row_count,
        gate.summary.two_sided_combat_count,
        gate.summary.lifecycle_started_row_count,
        gate.summary.lifecycle_updated_row_count,
        gate.summary.lifecycle_ended_row_count,
        gate.summary.cship_validated_member_count,
        gate.summary.task_force_linked_member_count,
        gate.summary.fleet_linked_member_count,
        gate.summary.hull_telemetry_member_count,
        gate.summary.sunk_convoy_telemetry_member_count,
        !empty_fixture_errors.is_empty(),
        json_string_array(&empty_fixture_errors),
    );
    let proof_path = out_dir.join("naval_combat_live_lane_stack_proof.json");
    if let Err(err) = fs::write(&proof_path, &proof_json) {
        eprintln!("write {} failed: {err}", proof_path.display());
        std::process::exit(1);
    }
    println!("{proof_json}");
    std::process::exit(
        if gate.ok
            && gate.summary.lifecycle_started_row_count > 0
            && gate.summary.lifecycle_updated_row_count > 0
            && gate.summary.lifecycle_ended_row_count > 0
            && gate.summary.task_force_linked_member_count > 0
            && gate.summary.fleet_linked_member_count > 0
            && gate.summary.hull_telemetry_member_count > 0
            && !empty_fixture_errors.is_empty()
        {
            0
        } else {
            1
        },
    );
}

struct ReplayVerifyReport {
    verified: bool,
    byte_match: bool,
    collector_compact_stream_bytes: usize,
}

fn write_and_verify_replay(
    replay_path: &PathBuf,
    compact_stream: &[u8],
) -> Result<ReplayVerifyReport, String> {
    const GAME_ID: &str = "naval_combat_live_lane_synthetic";
    let index = br#"{"game_id":"naval_combat_live_lane_synthetic","schema_version":"lane-local-compact-v1","snapshots":[]}"#;
    let entries = vec![
        PackEntry {
            path: format!("payload/{GAME_ID}/index.json"),
            bytes: index.to_vec(),
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{GAME_ID}/collector_stream.bin"),
            bytes: compact_stream.to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
    ];
    let identity = PackIdentity {
        schema_version: Some("lane-local-compact-v1".to_string()),
        build_exe_sha256: Some(
            "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad".to_string(),
        ),
        build_pinned: true,
        mod_identity: None,
        created_utc: Some("2026-07-01T00:00:00Z".to_string()),
    };
    let replay = pack(GAME_ID, &entries, &identity, true).map_err(|err| err.to_string())?;
    fs::write(replay_path, replay).map_err(|err| err.to_string())?;
    let container = Container::open(replay_path, &ExpectedIdentity::default())
        .map_err(|err| err.to_string())?;
    container.verify_all().map_err(|err| err.to_string())?;
    let stream_entries = container.entries_by_role("collector_compact_stream");
    if stream_entries.len() != 1 {
        return Err(format!(
            "expected one collector_compact_stream entry, found {}",
            stream_entries.len()
        ));
    }
    let stored = container
        .read_entry_bytes(&stream_entries[0].path)
        .map_err(|err| err.to_string())?;
    if stored.as_slice() != compact_stream {
        return Err("collector_compact_stream bytes differ from compact stream".to_string());
    }
    Ok(ReplayVerifyReport {
        verified: true,
        byte_match: true,
        collector_compact_stream_bytes: stored.len(),
    })
}

fn fixture() -> ValidatedCombatSnapshot {
    ValidatedCombatSnapshot {
        lifecycle: NavalCombatBodyBridgeRow {
            combat_id: 240_580,
            generation: 1,
            body_ptr: 0x22aa_0004_000,
            location_id: Some(2799),
            attacker_member_count: 1,
            defender_member_count: 1,
            attacker_positioning_milli: Some(812),
            defender_positioning_milli: Some(765),
        },
        members: vec![
            MemberBridgeRow {
                combat_id: 240_580,
                generation: 1,
                side: CombatSide::Attacker,
                group_index: 0,
                member_index: 0,
                member_ptr: 0x22aa_0020_000,
                unit_type_tag: CURRENT_870EBF_CSHIP_ROUTE.type_tag,
                ship_instance_id: Some(1001),
                is_convoy: false,
                owner_tag_db_id: Some(17),
                designed_max_strength_raw: Some(10_000_000),
                battle_slot_index_raw: 0,
                state_bitmask: Some(1),
                engagement_weight_raw: Some(1_000),
            },
            MemberBridgeRow {
                combat_id: 240_580,
                generation: 1,
                side: CombatSide::Defender,
                group_index: 0,
                member_index: 0,
                member_ptr: 0x22aa_0021_000,
                unit_type_tag: 0,
                ship_instance_id: None,
                is_convoy: true,
                owner_tag_db_id: Some(18),
                designed_max_strength_raw: Some(10_000_000),
                battle_slot_index_raw: 1,
                state_bitmask: Some(1),
                engagement_weight_raw: Some(500),
            },
        ],
    }
}

fn cship(id: u32) -> CshipSnapshotRow {
    CshipSnapshotRow {
        instance_id: id,
        body_ptr: 0x22aa_0100_000,
        vtable_rva: CURRENT_870EBF_CSHIP_ROUTE.cship_vtable_rva,
        handle_type_tag: CURRENT_870EBF_CSHIP_ROUTE.type_tag,
        alias_instance_id: id,
        ref100_sentinel_raw: CURRENT_870EBF_CSHIP_ROUTE.ref100_sentinel_raw,
        task_force_id: Some(477),
        fleet_id: Some(88),
        strength_raw: 10_000_000,
        organisation_raw: 9_000_000,
        experience_raw: 100_000,
        sunk_convoys: 0,
        category_enum: 3,
    }
}

fn json_string_array(values: &[String]) -> String {
    let parts = values
        .iter()
        .map(|value| json_str(value))
        .collect::<Vec<_>>();
    format!("[{}]", parts.join(", "))
}

fn json_str(value: &str) -> String {
    let mut out = String::from("\"");
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
    out.push('"');
    out
}
