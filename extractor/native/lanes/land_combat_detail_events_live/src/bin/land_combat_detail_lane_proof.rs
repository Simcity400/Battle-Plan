use std::env;
use std::fs;
use std::path::PathBuf;

use land_combat_detail_events_live_lane::{
    build_transition_capture_frame, gate_stream_for_product, result_json, LandCombatSnapshot,
    SideDetail, DOMAIN_LAND_COMBAT_DETAIL_EVENTS, DOMAIN_LAND_COMBAT_DETAIL_EVENTS_NAME,
};
use serde_json::json;
use snapshot::{pack, Container, ExpectedIdentity, PackEntry, PackIdentity};

fn main() {
    let mut out_dir = PathBuf::from(
        "extractor/native/runs/land_combat_detail_events_live_lane_stack_870ebf_20260701_v1",
    );
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

    // Attacker: 2 divisions front (committed 2, reserve 0). Detail carries the
    // validated CCombatSide scalars; defender likewise.
    let start = vec![snapshot(
        240_100,
        1,
        12_740,
        &[8_001, 8_002],
        &[9_001],
        Some(side_detail(2, 0)),
        Some(side_detail(1, 0)),
    )];
    let update = vec![snapshot(
        240_100,
        1,
        12_740,
        &[8_001],
        &[9_001, 9_002],
        Some(side_detail(1, 0)),
        Some(side_detail(2, 0)),
    )];
    let frame1 = match build_transition_capture_frame(&[], &start, 1, 60_787_539, 11) {
        Ok(frame) => frame,
        Err(errors) => fail(errors),
    };
    let frame2 = match build_transition_capture_frame(&start, &update, 4, 60_787_540, 22) {
        Ok(frame) => frame,
        Err(errors) => fail(errors),
    };
    let frame3 = match build_transition_capture_frame(&update, &[], 7, 60_787_541, 33) {
        Ok(frame) => frame,
        Err(errors) => fail(errors),
    };

    let frame1_row_count = frame_row_count(&frame1);
    let frame2_row_count = frame_row_count(&frame2);
    let frame3_row_count = frame_row_count(&frame3);
    let mut compact_stream = frame1.compact_stream;
    append_perf_and_frame_end(&mut compact_stream, 2, 60_787_539, 11, frame1_row_count);
    compact_stream.extend_from_slice(&frame2.compact_stream);
    append_perf_and_frame_end(&mut compact_stream, 5, 60_787_540, 22, frame2_row_count);
    compact_stream.extend_from_slice(&frame3.compact_stream);
    append_perf_and_frame_end(&mut compact_stream, 8, 60_787_541, 33, frame3_row_count);

    let compact_path = out_dir.join("land_combat_detail_events_live_domain55_compact.bin");
    fs::write(&compact_path, &compact_stream).unwrap_or_else(|err| {
        eprintln!("write {} failed: {err}", compact_path.display());
        std::process::exit(1);
    });

    let gate = gate_stream_for_product(&compact_stream, 2);
    let gate_path = out_dir.join("land_combat_detail_events_live_domain55_gate.json");
    fs::write(&gate_path, result_json(&gate)).unwrap_or_else(|err| {
        eprintln!("write {} failed: {err}", gate_path.display());
        std::process::exit(1);
    });

    let replay_path = out_dir.join("native_session.battleplan");
    let replay_verify = match write_and_verify_replay(
        &replay_path,
        &compact_stream,
        "land_combat_detail_events_live_lane_synthetic",
        "lane-local-domain55-v1",
    ) {
        Ok(report) => report,
        Err(err) => {
            eprintln!("replay verification failed: {err}");
            std::process::exit(1);
        }
    };

    let proof = json!({
        "status": if gate.ok { "passed" } else { "failed" },
        "domain_id": DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
        "domain_name": DOMAIN_LAND_COMBAT_DETAIL_EVENTS_NAME,
        "scope": "phase lifecycle plus participant join/leave events, PLUS per-side CCombatSide detail scalars (committed/reserve/units, air_kills, org-loss, i64x1e5 damage accumulators) from validated domain21 CLandCombat roots",
        "not_closed": [
            "losses (all-zero oracle on this save; not disambiguable)",
            "retreat versus overrun classification",
            "tactics (pointer/table-backed; PARTIAL)",
            "width",
            "terrain",
            "modifiers"
        ],
        "side_detail_rows": gate.summary.side_detail_rows,
        "compact_path": compact_path,
        "gate_path": gate_path,
        "replay_path": replay_path,
        "hoi4replay_verified": replay_verify.verified,
        "collector_compact_stream_byte_match": replay_verify.byte_match,
        "collector_compact_stream_bytes": replay_verify.collector_compact_stream_bytes,
        "gate_ok": gate.ok,
        "phase_started_rows": gate.summary.phase_started_rows,
        "phase_updated_rows": gate.summary.phase_updated_rows,
        "phase_ended_rows": gate.summary.phase_ended_rows,
        "participant_joined_rows": gate.summary.participant_joined_rows,
        "participant_left_unclassified_rows": gate.summary.participant_left_unclassified_rows,
        "unclassified_rows": gate.summary.unclassified_rows,
        "errors": gate.errors,
    });
    let proof_path = out_dir.join("land_combat_detail_events_live_lane_stack_proof.json");
    fs::write(&proof_path, serde_json::to_vec_pretty(&proof).unwrap()).unwrap_or_else(|err| {
        eprintln!("write {} failed: {err}", proof_path.display());
        std::process::exit(1);
    });
    println!("{}", serde_json::to_string_pretty(&proof).unwrap());
    std::process::exit(
        if gate.ok
            && gate.summary.phase_started_rows == 1
            && gate.summary.phase_updated_rows == 1
            && gate.summary.phase_ended_rows == 1
            && gate.summary.participant_joined_rows == 1
            && gate.summary.participant_left_unclassified_rows == 1
            && replay_verify.verified
        {
            0
        } else {
            1
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn snapshot(
    combat_id: u32,
    generation: u64,
    location: u32,
    attackers: &[u32],
    defenders: &[u32],
    attacker_detail: Option<SideDetail>,
    defender_detail: Option<SideDetail>,
) -> LandCombatSnapshot {
    LandCombatSnapshot {
        combat_id,
        generation,
        location: Some(location),
        day: Some(4),
        duration: Some(4),
        attacker_divisions: attackers.to_vec(),
        defender_divisions: defenders.to_vec(),
        attacker_detail,
        defender_detail,
        attacker_attribution: Vec::new(),
        defender_attribution: Vec::new(),
    }
}

fn side_detail(committed: u32, reserve: u32) -> SideDetail {
    SideDetail {
        units: committed + reserve,
        committed,
        reserve,
        air_kills: 3,
        org_loss_summary_index: 0,
        num_org_losses: 4,
        anti_air_attack_raw: 434_710,
        air_damage_str_raw: 8_961_046,
        air_damage_org_raw: 18_464_611,
        ground_damage_str_raw: 11_248_500,
        ground_damage_org_raw: 15_443_824,
        prevented_damage_str_raw: 111_654,
        prevented_damage_org_raw: 245_433,
        progress_raw: Some(53_000),
        tactic: Some(land_combat_detail_events_live_lane::TacticIdentity {
            id: 7,
            token: "tactic_basic_attack".into(),
            name: "TACTIC_BASIC_ATTACK".into(),
        }),
        tactic_known: true,
        counts_incomplete: false,
    }
}

fn fail(errors: Vec<String>) -> ! {
    eprintln!("fixture validation failed: {errors:?}");
    std::process::exit(1);
}

fn frame_row_count(frame: &land_combat_detail_events_live_lane::CaptureFrame) -> u32 {
    frame
        .rows
        .len()
        .saturating_add(frame.side_detail_rows.len())
        .saturating_add(frame.attribution_rows.len()) as u32
}

fn append_perf_and_frame_end(
    stream: &mut Vec<u8>,
    perf_sequence: u64,
    game_hour_raw: i32,
    elapsed_wall_ms: u64,
    row_count: u32,
) {
    let mut perf_payload = [0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN];
    perf_payload[0..8].copy_from_slice(&elapsed_wall_ms.to_le_bytes());
    for offset in [16usize, 20, 24, 28] {
        perf_payload[offset..offset + 4].copy_from_slice(&row_count.to_le_bytes());
    }
    perf_payload[32..36]
        .copy_from_slice(&compact_contract::COMPACT_STATUS_DELTA_DATA.to_le_bytes());
    let mut frame_end_payload = [0u8; compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN];
    frame_end_payload[16..20].copy_from_slice(&1u32.to_le_bytes());
    frame_end_payload[20..24].copy_from_slice(&1u32.to_le_bytes());
    append_compact_record(
        stream,
        perf_sequence,
        game_hour_raw,
        DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf_payload,
    );
    append_compact_record(
        stream,
        perf_sequence + 1,
        game_hour_raw,
        compact_contract::COMPACT_DOMAIN_FRAME,
        compact_contract::COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload,
    );
}

fn append_compact_record(
    stream: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    payload: &[u8],
) {
    stream.extend_from_slice(b"H4CR");
    stream.extend_from_slice(&1u16.to_le_bytes());
    stream.extend_from_slice(&32u16.to_le_bytes());
    stream.extend_from_slice(&sequence.to_le_bytes());
    stream.extend_from_slice(&game_hour_raw.to_le_bytes());
    stream.extend_from_slice(&domain_id.to_le_bytes());
    stream.extend_from_slice(&record_type.to_le_bytes());
    stream.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    stream.extend_from_slice(&0u32.to_le_bytes());
    stream.extend_from_slice(payload);
}

struct ReplayVerifyReport {
    verified: bool,
    byte_match: bool,
    collector_compact_stream_bytes: usize,
}

fn write_and_verify_replay(
    replay_path: &PathBuf,
    compact_stream: &[u8],
    game_id: &str,
    schema_version: &str,
) -> Result<ReplayVerifyReport, String> {
    let index = serde_json::to_vec(&json!({
        "game_id": game_id,
        "schema_version": schema_version,
        "snapshots": []
    }))
    .map_err(|err| err.to_string())?;
    let entries = vec![
        PackEntry {
            path: format!("payload/{game_id}/index.json"),
            bytes: index,
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/collector_stream.bin"),
            bytes: compact_stream.to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
    ];
    let identity = PackIdentity {
        schema_version: Some(schema_version.to_string()),
        build_exe_sha256: Some(
            "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad".to_string(),
        ),
        build_pinned: true,
        mod_identity: None,
        created_utc: Some("2026-07-01T00:00:00Z".to_string()),
    };
    let replay = pack(game_id, &entries, &identity, true).map_err(|err| err.to_string())?;
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
