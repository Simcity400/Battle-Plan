use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use compact_contract::{
    parse_compact_record_header, validate_compact_record_semantics, RecordFramingError,
};
use construction_buildings_state_damage_live::collector_module::{
    collect_domain48_compact, CollectorRoots, COLLECTOR_MODULE_NAME,
};
use construction_buildings_state_damage_live::decoder_gate::{
    decode_domain48_compact, DECODER_MODULE_NAME,
};
use construction_buildings_state_damage_live::{
    CountryConstructionRoot, LaneMemory, ProvinceRoot, StateRoot, BUILD_PROFILE,
    CBUILDING_OFF_HEALTHY_LEVEL, CBUILDING_OFF_LEVEL, CBUILDING_OFF_TYPE, CBUILDING_VTABLE_RVA,
    CONSTRUCTION_BEGIN, CONSTRUCTION_CAP, CONSTRUCTION_SIZE, CONSTRUCTION_TARGET_LOCATION_PTR,
    CONSTRUCTION_TEMPLATE_ID, CPROVINCE_OFF_BUILDINGS_VEC, DOMAIN_ID, DOMAIN_NAME,
    LINE_ACTIVE_FACTORIES, LINE_CONSTRUCTION_SUB, LINE_COST, LINE_ID, LINE_PRODUCED, LINE_TYPE,
    LINE_TYPE_CONSTRUCTION, OBJ_PROD_MANAGER, SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE,
    STATE_DEF_OFF_RESOURCES_CAP, STATE_DEF_OFF_RESOURCES_PTR, STATE_DEF_OFF_RESOURCES_SIZE,
    STATE_OFF_BUILDINGS_VEC, STATE_OFF_DEF, STATE_RESOURCE_STRIDE, TARGET_LOCATION_ID,
};
use serde_json::json;
use snapshot::container::{Container, ExpectedIdentity};
use snapshot::sha256::Sha256;
use snapshot::{pack, PackEntry, PackIdentity};

const STREAM_MAGIC: &[u8; 4] = b"H4CR";
const STREAM_VERSION: u16 = 1;
const STREAM_HEADER_LEN: u16 = 32;
const RECORD_TYPE_DOMAIN_PERF: u16 = 1;
const RECORD_TYPE_FRAME_END: u16 = 2;
const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
const DOMAIN_FRAME: u16 = 1000;

fn main() -> Result<(), String> {
    let args = env::args().collect::<Vec<_>>();
    let out_dir = PathBuf::from(args.get(1).ok_or_else(|| {
        "usage: construction_buildings_state_damage_replay_pack_proof <out-dir> [requested-game-hour-advancements]".to_string()
    })?);
    let requested_game_hour_advancements = parse_requested_game_hour_advancements(&args)?;
    let expected_closed_frames = requested_game_hour_advancements + 1;
    fs::create_dir_all(&out_dir)
        .map_err(|err| format!("failed to create {}: {err}", out_dir.display()))?;

    let module_base = 0x1400_0000usize;
    let fixture = FixtureMemory::production_shaped(module_base);
    let first_game_hour_raw = 123;
    let mut hour_payloads = Vec::new();
    let mut failed_country_roots_total = 0u32;
    let mut failed_state_building_roots_total = 0u32;
    let mut failed_state_resource_roots_total = 0u32;
    let mut failed_province_building_roots_total = 0u32;
    let mut status_flags_or = 0u32;
    let mut source_flags_or = 0u32;

    for hour_index in 0..expected_closed_frames {
        let game_hour_raw = first_game_hour_raw + hour_index as i32;
        let mut roots = fixture.roots.clone();
        roots.game_hour_raw = game_hour_raw;
        let started = Instant::now();
        let compact_output = collect_domain48_compact(&fixture.mem, roots);
        let collect_time_us = duration_micros_u64(started.elapsed());
        let decoded = decode_domain48_compact(&compact_output.compact)?;
        if !decoded.gate.ok {
            return Err(format!(
                "domain-48 decoder gate failed at hour {game_hour_raw}: {:?}",
                decoded.gate.errors
            ));
        }
        if decoded.summary.construction_rows == 0
            || decoded.summary.construction_rows_with_target_state
                != decoded.summary.construction_rows
            || decoded.summary.target_state_unavailable_badged
        {
            return Err(format!(
                "target-state extraction was not preserved at hour {game_hour_raw}"
            ));
        }
        failed_country_roots_total =
            failed_country_roots_total.saturating_add(compact_output.stats.failed_country_roots);
        failed_state_building_roots_total = failed_state_building_roots_total
            .saturating_add(compact_output.stats.failed_state_building_roots);
        failed_state_resource_roots_total = failed_state_resource_roots_total
            .saturating_add(compact_output.stats.failed_state_resource_roots);
        failed_province_building_roots_total = failed_province_building_roots_total
            .saturating_add(compact_output.stats.failed_province_building_roots);
        status_flags_or |= compact_output.gate.status_flags;
        source_flags_or |= compact_output.gate.source_flags;
        hour_payloads.push(HourPayload {
            game_hour_raw,
            compact: compact_output.compact,
            collect_time_us,
            status_flags: compact_output.gate.status_flags,
            source_flags: compact_output.gate.source_flags,
            construction_rows: decoded.summary.construction_rows,
            construction_rows_with_target_state: decoded
                .summary
                .construction_rows_with_target_state,
            target_state_unavailable_badged: decoded.summary.target_state_unavailable_badged,
            building_rows: decoded.summary.building_rows,
            damaged_building_rows: decoded.summary.damaged_building_rows,
            resource_rows: decoded.summary.resource_rows,
        });
    }

    let domain_payload_path =
        out_dir.join("construction_buildings_state_damage_live.domain48.compact.bin");
    fs::write(&domain_payload_path, &hour_payloads[0].compact)
        .map_err(|err| format!("failed to write {}: {err}", domain_payload_path.display()))?;

    let compact_stream = build_compact_stream(&hour_payloads);
    let stream_path =
        out_dir.join("construction_buildings_state_damage_live.domain48.collector_stream.bin");
    fs::write(&stream_path, &compact_stream)
        .map_err(|err| format!("failed to write {}: {err}", stream_path.display()))?;

    let stream_report =
        verify_domain48_stream(&compact_stream, requested_game_hour_advancements as usize)?;
    if !stream_report.ok {
        return Err(format!(
            "domain-48 compact stream proof failed: {:?}",
            stream_report.errors
        ));
    }

    let game_id = "construction_buildings_state_damage_live_domain48_proof";
    let replay_path = out_dir.join("construction_buildings_state_damage_live.domain48.battleplan");
    let manifest_path = out_dir.join("native_session_manifest.json");
    let manifest = fixture_manifest(
        game_id,
        requested_game_hour_advancements,
        expected_closed_frames,
    );
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("failed to write {}: {err}", manifest_path.display()))?;

    let replay_report = write_and_verify_replay_container(
        game_id,
        "collector_stream.bin",
        &compact_stream,
        &manifest_path,
        &replay_path,
    )?;

    let source_unavailable =
        status_flags_or & construction_buildings_state_damage_live::STATUS_SOURCE_UNAVAILABLE != 0;
    let partial_data =
        status_flags_or & construction_buildings_state_damage_live::STATUS_PARTIAL_DATA != 0;
    let worker_panic =
        status_flags_or & construction_buildings_state_damage_live::STATUS_WORKER_PANIC != 0;
    let target_state_unavailable_badged_any = hour_payloads
        .iter()
        .any(|payload| payload.target_state_unavailable_badged);
    let total_construction_rows = hour_payloads
        .iter()
        .map(|payload| payload.construction_rows)
        .sum::<usize>();
    let total_construction_rows_with_target_state = hour_payloads
        .iter()
        .map(|payload| payload.construction_rows_with_target_state)
        .sum::<usize>();
    let total_building_rows = hour_payloads
        .iter()
        .map(|payload| payload.building_rows)
        .sum::<usize>();
    let total_damaged_building_rows = hour_payloads
        .iter()
        .map(|payload| payload.damaged_building_rows)
        .sum::<usize>();
    let total_resource_rows = hour_payloads
        .iter()
        .map(|payload| payload.resource_rows)
        .sum::<usize>();
    let collect_times_us = hour_payloads
        .iter()
        .map(|payload| payload.collect_time_us)
        .collect::<Vec<_>>();
    let hour_summaries = hour_payloads
        .iter()
        .map(|payload| {
            json!({
                "game_hour_raw": payload.game_hour_raw,
                "payload_bytes": payload.compact.len(),
                "collect_time_us": payload.collect_time_us,
                "status_flags": payload.status_flags,
                "source_flags": payload.source_flags,
                "construction_rows": payload.construction_rows,
                "construction_rows_with_target_state": payload.construction_rows_with_target_state,
                "target_state_unavailable_badged": payload.target_state_unavailable_badged,
                "building_rows": payload.building_rows,
                "damaged_building_rows": payload.damaged_building_rows,
                "resource_rows": payload.resource_rows,
            })
        })
        .collect::<Vec<_>>();

    let proof = json!({
        "schema": "construction_buildings_state_damage_replay_pack_proof_v1",
        "artifact_kind": "lane_local_host_shaped_fixture",
        "proof_kind": "lane_owned_host_shaped_hourly_schedule_compact_stream_hoi4replay_roundtrip",
        "production_complete": false,
        "production_certifying": false,
        "source_capture_verified": false,
        "domain": {
            "id": DOMAIN_ID,
            "name": DOMAIN_NAME,
            "build_profile": BUILD_PROFILE,
        },
        "files": {
            "domain_payload": path_slash(&domain_payload_path),
            "collector_compact_stream": path_slash(&stream_path),
            "session_manifest": path_slash(&manifest_path),
            "hoi4replay": path_slash(&replay_path),
        },
        "lane_modules": {
            "collector": COLLECTOR_MODULE_NAME,
            "decoder_gate": DECODER_MODULE_NAME,
        },
        "collector_result": {
            "status_flags_or": status_flags_or,
            "source_flags_or": source_flags_or,
            "stats": {
                "failed_country_roots": failed_country_roots_total,
                "failed_state_building_roots": failed_state_building_roots_total,
                "failed_state_resource_roots": failed_state_resource_roots_total,
                "failed_province_building_roots": failed_province_building_roots_total,
            },
        },
        "decoder_gate": {
            "ok": true,
            "errors": [],
            "summary": {
                "domain_id": DOMAIN_ID,
                "domain_name": DOMAIN_NAME,
                "frames_decoded": hour_payloads.len(),
                "construction_rows": total_construction_rows,
                "construction_rows_with_target_state": total_construction_rows_with_target_state,
                "target_state_unavailable_badged_any": target_state_unavailable_badged_any,
                "building_rows": total_building_rows,
                "damaged_building_rows": total_damaged_building_rows,
                "resource_rows": total_resource_rows,
            },
        },
        "schedule_proof": {
            "proof_type": "host_shaped_scheduled_hour_snapshot",
            "scheduled_snapshot_step_hours": 1,
            "scheduled_domain": DOMAIN_NAME,
            "requested_game_hour_advancements": requested_game_hour_advancements,
            "expected_closed_frames": expected_closed_frames,
            "observed_game_hour_advancements": stream_report.observed_game_hour_advancements,
            "observed_closed_frames": stream_report.unique_hours.len(),
            "first_game_hour_raw": first_game_hour_raw,
            "last_game_hour_raw": first_game_hour_raw + expected_closed_frames as i32 - 1,
            "per_hour_domain48_perf_records": 1,
            "per_hour_domain48_data_records": 1,
            "per_hour_frame_end_records": 1,
            "target_state_all_rows_all_hours": total_construction_rows > 0 && total_construction_rows == total_construction_rows_with_target_state,
            "zero_skipped_hours": stream_report.zero_skipped_hours,
            "collect_time_us": timing_summary_json(&collect_times_us),
            "hour_summaries": hour_summaries,
            "host_shape_limit": "does not edit or invoke shared collector entrypoints; proves the registered lane collector/encoder/gate can satisfy one domain-48 compact data record every scheduled game hour",
        },
        "certification": {
            "claimed": false,
            "minimum_game_hour_advancements": null,
            "requested_game_hour_advancements": requested_game_hour_advancements,
            "expected_closed_frames": expected_closed_frames,
            "observed_game_hour_advancements": stream_report.observed_game_hour_advancements,
            "observed_closed_frames": stream_report.unique_hours.len(),
            "reason": "lane-owned host-shaped fixture proof only; no live production-duration certification is claimed",
        },
        "compact_stream": {
            "ok": stream_report.ok,
            "errors": stream_report.errors,
            "total_bytes": compact_stream.len(),
            "sha256": Sha256::hex(&compact_stream),
            "record_count": stream_report.record_count,
            "domain_perf_records": stream_report.domain_perf_records,
            "domain_data_records": stream_report.domain_data_records,
            "frame_end_records": stream_report.frame_end_records,
            "domains_seen": stream_report.domains_seen,
            "unique_hours": stream_report.unique_hours,
            "skipped_hours": stream_report.skipped_hours,
            "zero_skipped_hours": stream_report.zero_skipped_hours,
            "domain48_payload_bytes_first_hour": hour_payloads[0].compact.len(),
            "domain48_payload_sha256_first_hour": Sha256::hex(&hour_payloads[0].compact),
        },
        "hoi4replay_verification": {
            "ok": replay_report.ok,
            "errors": replay_report.errors,
            "path": path_slash(&replay_path),
            "stream_entry": replay_report.stream_entry,
            "manifest_entry": replay_report.manifest_entry,
            "stream_bytes_equal": replay_report.stream_bytes_equal,
            "manifest_bytes_equal": replay_report.manifest_bytes_equal,
            "non_certifying_metadata_verified": replay_report.non_certifying_metadata_verified,
            "stream_sha256": replay_report.stream_sha256,
            "container_verify_all": replay_report.container_verify_all,
        },
        "target_state_extraction": {
            "ok": total_construction_rows > 0
                && total_construction_rows_with_target_state == total_construction_rows
                && !target_state_unavailable_badged_any,
            "source_flag": "SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE",
            "source_flag_value_when_blocked": format!("0x{:X}", SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE),
            "construction_rows_with_target_state": total_construction_rows_with_target_state,
            "construction_rows": total_construction_rows,
            "target_state_unavailable_badged_any": target_state_unavailable_badged_any,
            "root": "subobject+0x1D8 -> target_location +0x6C",
            "state_location_rows_per_hour": 1,
            "province_location_rows_per_hour": 1,
            "province_mapping": "province construction target state is resolved from exact province location through ProvinceRoot province_id -> state_id",
            "rejected_candidates_not_emitted": ["subobject+0xC0", "subobject+0xC8"],
        },
        "production_health_gate": {
            "source_unavailable": source_unavailable,
            "partial_data": partial_data,
            "worker_panic": worker_panic,
            "bridge_failure": false,
            "lane_gate_errors": [],
        },
        "orchestration_blocker": {
            "shared_integration_slot": "air_variant_archetypes_live owns active integration; this lane did not edit shared collector/recorder entrypoints",
            "shared_recorder_dispatch": "current shared recorder has no domain-48 decode dispatch until the registration-only patch is applied",
            "live_dll_speed_five_proof": "shared production runner still cannot schedule domain 48 until the registration-only patch flips the registry descriptors and adds collector/recorder hooks; this host-shaped proof verifies the lane-owned collector path for consecutive hourly snapshots",
        },
        "validated_layout_used_by_fixture": {
            "construction_queue": {
                "country_to_production_manager": format_hex(OBJ_PROD_MANAGER),
                "manager_vector_begin": format_hex(CONSTRUCTION_BEGIN),
                "manager_vector_cap": format_hex(CONSTRUCTION_CAP),
                "manager_vector_size": format_hex(CONSTRUCTION_SIZE),
                "line_type_offset": format_hex(LINE_TYPE),
                "line_type_value": LINE_TYPE_CONSTRUCTION,
                "line_id_offset": format_hex(LINE_ID),
                "active_factories_offset": format_hex(LINE_ACTIVE_FACTORIES),
                "cost_raw_offset": format_hex(LINE_COST),
                "produced_raw_offset": format_hex(LINE_PRODUCED),
                "building_subobject_offset": format_hex(LINE_CONSTRUCTION_SUB),
                "template_id_offset": format_hex(CONSTRUCTION_TEMPLATE_ID),
                "target_location_ptr_offset": format_hex(CONSTRUCTION_TARGET_LOCATION_PTR),
                "target_location_id_offset": format_hex(TARGET_LOCATION_ID),
            },
            "buildings": {
                "state_building_vector": format_hex(STATE_OFF_BUILDINGS_VEC),
                "province_building_vector": format_hex(CPROVINCE_OFF_BUILDINGS_VEC),
                "cbuilding_vtable_rva": format_hex(CBUILDING_VTABLE_RVA),
                "type_id_offset": format_hex(CBUILDING_OFF_TYPE),
                "level_offset": format_hex(CBUILDING_OFF_LEVEL),
                "healthy_level_offset": format_hex(CBUILDING_OFF_HEALTHY_LEVEL),
            },
            "state_resources": {
                "state_def_offset": format_hex(STATE_OFF_DEF),
                "resources_ptr_offset": format_hex(STATE_DEF_OFF_RESOURCES_PTR),
                "resources_size_offset": format_hex(STATE_DEF_OFF_RESOURCES_SIZE),
                "resources_cap_offset": format_hex(STATE_DEF_OFF_RESOURCES_CAP),
                "resource_stride": STATE_RESOURCE_STRIDE,
            },
        },
    });

    let proof_path = out_dir.join("replay_pack_proof.json");
    fs::write(
        &proof_path,
        serde_json::to_vec_pretty(&proof).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("failed to write {}: {err}", proof_path.display()))?;

    if source_unavailable
        || partial_data
        || worker_panic
        || !stream_report.zero_skipped_hours
        || total_construction_rows == 0
        || total_construction_rows != total_construction_rows_with_target_state
        || target_state_unavailable_badged_any
    {
        return Err(format!(
            "replay pack proof failed health gate; wrote {}",
            proof_path.display()
        ));
    }

    Ok(())
}

fn parse_requested_game_hour_advancements(args: &[String]) -> Result<u32, String> {
    if args.len() > 3 {
        return Err(
            "usage: construction_buildings_state_damage_replay_pack_proof <out-dir> [requested-game-hour-advancements]"
                .to_string(),
        );
    }
    let requested = args
        .get(2)
        .map(|value| {
            value.parse::<u32>().map_err(|err| {
                format!("invalid requested-game-hour-advancements value {value:?}: {err}")
            })
        })
        .transpose()?
        .unwrap_or(12);
    if !(1..=71).contains(&requested) {
        return Err(format!(
            "requested-game-hour-advancements {requested} is outside the supported exact range 1..=71"
        ));
    }
    Ok(requested)
}

fn fixture_manifest(
    game_id: &str,
    requested_game_hour_advancements: u32,
    expected_closed_frames: u32,
) -> serde_json::Value {
    json!({
        "schema": "native_session_manifest_v1",
        "proof_schema": "construction_buildings_state_damage_replay_pack_proof_v1",
        "artifact_kind": "lane_local_host_shaped_fixture",
        "production_certifying": false,
        "source_capture_verified": false,
        "replay_game_id": game_id,
        "stream_format": "compact",
        "stream_file": "collector_stream.bin",
        "scheduled_domains": [DOMAIN_NAME],
        "expected_domain_ids": [DOMAIN_ID],
        "scheduled_snapshot_step_hours": 1,
        "requested_game_hour_advancements": requested_game_hour_advancements,
        "expected_closed_frames": expected_closed_frames,
        "lane_modules": {
            "collector": COLLECTOR_MODULE_NAME,
            "decoder_gate": DECODER_MODULE_NAME,
        },
        "target_state_semantics": {
            "status": "all_rows_emitted_from_validated_target_location_root",
            "target_location_root": "subobject+0x1D8 -> target_location +0x6C",
            "source_flag": "SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE",
            "source_flag_value_when_blocked": SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE,
            "rejected_candidates": ["subobject+0xC0", "subobject+0xC8"],
        },
    })
}

struct HourPayload {
    game_hour_raw: i32,
    compact: Vec<u8>,
    collect_time_us: u64,
    status_flags: u32,
    source_flags: u32,
    construction_rows: usize,
    construction_rows_with_target_state: usize,
    target_state_unavailable_badged: bool,
    building_rows: usize,
    damaged_building_rows: usize,
    resource_rows: usize,
}

fn build_compact_stream(hours: &[HourPayload]) -> Vec<u8> {
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    for hour in hours {
        append_compact_record(
            &mut stream,
            sequence,
            hour.game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &domain_perf_payload(hour),
        );
        sequence = sequence
            .checked_add(1)
            .expect("bounded fixture sequence cannot overflow");
        append_compact_record(
            &mut stream,
            sequence,
            hour.game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &hour.compact,
        );
        sequence = sequence
            .checked_add(1)
            .expect("bounded fixture sequence cannot overflow");
        append_compact_record(
            &mut stream,
            sequence,
            hour.game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_end_payload(hour.collect_time_us, hour.status_flags),
        );
        sequence = sequence
            .checked_add(1)
            .expect("bounded fixture sequence cannot overflow");
    }
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
    out.extend_from_slice(STREAM_MAGIC);
    out.extend_from_slice(&STREAM_VERSION.to_le_bytes());
    out.extend_from_slice(&STREAM_HEADER_LEN.to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    let payload_len = u32::try_from(payload.len()).expect("fixture payload length exceeds u32");
    out.extend_from_slice(&payload_len.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
}

fn domain_perf_payload(hour: &HourPayload) -> Vec<u8> {
    let mut payload = Vec::with_capacity(40);
    let row_count = domain48_row_count(&hour.compact).unwrap_or(0);
    let unit_count = row_count;
    payload.extend_from_slice(&1u64.to_le_bytes());
    payload.extend_from_slice(&hour.collect_time_us.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&unit_count.to_le_bytes());
    payload.extend_from_slice(&unit_count.to_le_bytes());
    payload.extend_from_slice(&hour.status_flags.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload
}

fn frame_end_payload(frame_time_us: u64, status_flags: u32) -> Vec<u8> {
    let mut payload = Vec::with_capacity(32);
    payload.extend_from_slice(&1u64.to_le_bytes());
    payload.extend_from_slice(&frame_time_us.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload
}

fn domain48_row_count(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < 32 {
        return None;
    }
    let construction = read_u32(bytes, 20)?;
    let building = read_u32(bytes, 24)?;
    let resource = read_u32(bytes, 28)?;
    Some(
        construction
            .saturating_add(building)
            .saturating_add(resource),
    )
}

#[derive(Debug)]
struct StreamReport {
    ok: bool,
    errors: Vec<String>,
    record_count: usize,
    domain_perf_records: usize,
    domain_data_records: usize,
    frame_end_records: usize,
    domains_seen: Vec<u16>,
    unique_hours: Vec<i32>,
    skipped_hours: Vec<i32>,
    zero_skipped_hours: bool,
    observed_game_hour_advancements: Option<i64>,
}

fn verify_domain48_stream(
    raw: &[u8],
    requested_game_hour_advancements: usize,
) -> Result<StreamReport, String> {
    let expected_closed_frames = requested_game_hour_advancements
        .checked_add(1)
        .ok_or_else(|| "requested game-hour advancement count overflows usize".to_string())?;
    let mut errors = Vec::new();
    if let Err(error) = compact_contract::stream_validator::validate_freeze_census_stream_placement(raw) {
        errors.push(format!("stateful stream placement: {error}"));
    }
    let mut offset = 0usize;
    let mut previous_sequence: Option<u64> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut pending_perf_status: Option<u32> = None;
    let mut open_frame_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut final_record_is_clean_frame_end = false;
    let mut release_epilogue_seen = false;
    let mut record_count = 0usize;
    let mut domain_perf_records = 0usize;
    let mut domain_data_records = 0usize;
    let mut frame_end_records = 0usize;
    let mut domains_seen = BTreeSet::new();
    let mut observed_record_hours = BTreeSet::new();
    let mut closed_hours = BTreeSet::new();
    let mut perf_by_hour: BTreeMap<i32, usize> = BTreeMap::new();
    let mut data_by_hour: BTreeMap<i32, usize> = BTreeMap::new();
    let mut frame_end_by_hour: BTreeMap<i32, usize> = BTreeMap::new();
    let mut owned_perf_roster = BTreeSet::new();
    let mut owned_perf_status_or = 0u32;

    while offset < raw.len() {
        // Physical EOF authority: even a tail too short to parse as a header
        // invalidates the previous FRAME_END as the final physical record.
        let previous_record_closed_world = final_record_is_clean_frame_end;
        final_record_is_clean_frame_end = false;
        let header = match parse_compact_record_header(&raw[offset..]) {
            Ok(header) => header,
            Err(RecordFramingError::TruncatedHeader(_)) => {
                errors.push(format!("offset {offset}: truncated compact stream header"));
                break;
            }
            Err(RecordFramingError::BadMagic) => {
                errors.push(format!("offset {offset}: invalid compact stream magic"));
                break;
            }
            Err(RecordFramingError::UnsupportedVersion(version)) => {
                errors.push(format!(
                    "offset {offset}: unsupported stream version {version}"
                ));
                break;
            }
            Err(RecordFramingError::UnsupportedHeaderLen(header_len)) => {
                errors.push(format!(
                    "offset {offset}: unsupported header_len {header_len}"
                ));
                break;
            }
            Err(err @ RecordFramingError::PayloadTooLarge(_)) => {
                errors.push(format!("offset {offset}: {err}"));
                break;
            }
        };
        let sequence = header.sequence;
        let game_hour_raw = header.game_hour_raw;
        let domain_id = header.domain_id;
        let record_type = header.record_type;
        let payload_len = header.payload_len;
        let payload_start = offset + STREAM_HEADER_LEN as usize;
        let payload_end = payload_start.saturating_add(payload_len);
        if payload_end > raw.len() {
            errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining stream bytes"
            ));
            break;
        }
        match previous_sequence {
            None if sequence != 1 => errors.push(format!(
                "offset {offset}: sequence {sequence} does not start at 1 (stream prefix is missing)"
            )),
            Some(previous) => match previous.checked_add(1) {
                Some(expected) if sequence != expected => errors.push(format!(
                    "offset {offset}: sequence discontinuity {previous} -> {sequence} (expected {expected})"
                )),
                None => errors.push(format!(
                    "offset {offset}: sequence space exhausted after {previous}; got {sequence} (wrapping is not allowed)"
                )),
                _ => {}
            },
            _ => {}
        }
        previous_sequence = Some(sequence);
        record_count += 1;
        let payload = &raw[payload_start..payload_end];
        let record_semantics_ok = match validate_compact_record_semantics(
            domain_id,
            record_type,
            header.record_flags,
            payload,
        ) {
            Ok(()) => true,
            Err(error) => {
                errors.push(format!("offset {offset}: {error}"));
                false
            }
        };
        let known_record_type = match compact_contract::KnownRecordType::try_from(record_type) {
            Ok(record_type) => record_type,
            Err(unknown) => {
                errors.push(format!(
                    "offset {offset}: unknown record type {}",
                    unknown.0
                ));
                offset = payload_end;
                continue;
            }
        };
        if !matches!(
            known_record_type,
            compact_contract::KnownRecordType::FreezeCensus
                | compact_contract::KnownRecordType::StreamHeader
        ) {
            domains_seen.insert(domain_id);
            observed_record_hours.insert(game_hour_raw);
        }

        match known_record_type {
            compact_contract::KnownRecordType::DomainPerf => {
                domain_perf_records += 1;
                *perf_by_hour.entry(game_hour_raw).or_default() += 1;
                if domain_id != DOMAIN_ID {
                    errors.push(format!(
                        "offset {offset}: unexpected perf domain {domain_id}"
                    ));
                } else {
                    owned_perf_roster.insert(domain_id);
                }
                if record_semantics_ok {
                    let perf_status = read_u32(payload, 32).expect("canonical PERF status word");
                    validate_status_word(&mut errors, "DOMAIN_PERF", perf_status, offset);
                    if domain_id == DOMAIN_ID {
                        owned_perf_status_or |=
                            compact_contract::compact_status_for_global_interpretation(
                                domain_id,
                                perf_status,
                            );
                        match pending_perf_status {
                            Some(pending) if pending != perf_status => errors.push(format!(
                                "offset {offset}: duplicate DOMAIN_PERF status {perf_status:#010x} does not match pending status {pending:#010x}"
                            )),
                            Some(_) => {}
                            None => pending_perf_status = Some(perf_status),
                        }
                    }
                }
                match open_frame_hour {
                    Some(open) if open != game_hour_raw => errors.push(format!(
                        "offset {offset}: DOMAIN_PERF hour {game_hour_raw} does not match open DATA frame hour {open}"
                    )),
                    Some(_) => {}
                    None => {
                        validate_next_domain48_hour(
                            &mut errors,
                            last_closed_hour,
                            game_hour_raw,
                            "DOMAIN_PERF",
                            offset,
                        );
                        match pending_perf_hour {
                            Some(pending) if pending != game_hour_raw => errors.push(format!(
                                "offset {offset}: DOMAIN_PERF hour {game_hour_raw} conflicts with pending frame hour {pending}"
                            )),
                            Some(_) => {}
                            None => pending_perf_hour = Some(game_hour_raw),
                        }
                    }
                }
            }
            compact_contract::KnownRecordType::DomainData => {
                domain_data_records += 1;
                *data_by_hour.entry(game_hour_raw).or_default() += 1;
                let mut data_status = None;
                if domain_id != DOMAIN_ID {
                    errors.push(format!(
                        "offset {offset}: unexpected data domain {domain_id}"
                    ));
                } else if record_semantics_ok {
                    match decode_domain48_compact(payload) {
                        Ok(decoded) => {
                            data_status = Some(decoded.frame.status_flags);
                            validate_status_word(
                                &mut errors,
                                "DOMAIN_DATA",
                                decoded.frame.status_flags,
                                offset,
                            );
                            if !decoded.gate.ok {
                                errors.push(format!(
                                    "offset {offset}: domain-48 payload gate failed {:?}",
                                    decoded.gate.errors
                                ));
                            }
                            if decoded.summary.construction_rows == 0
                                || decoded.summary.construction_rows_with_target_state
                                    != decoded.summary.construction_rows
                                || decoded.summary.target_state_unavailable_badged
                            {
                                errors.push(format!(
                                    "offset {offset}: target-state extraction not preserved"
                                ));
                            }
                        }
                        Err(error) => errors.push(format!(
                            "offset {offset}: domain-48 compact payload rejected: {error}"
                        )),
                    }
                }
                match (pending_perf_hour, pending_perf_status, data_status) {
                    (None, _, _) => errors.push(format!(
                        "offset {offset}: DOMAIN_DATA hour {game_hour_raw} appears before its owned DOMAIN_PERF authority"
                    )),
                    (Some(perf_hour), _, _) if perf_hour != game_hour_raw => errors.push(format!(
                        "offset {offset}: DOMAIN_DATA hour {game_hour_raw} conflicts with pending PERF hour {perf_hour}"
                    )),
                    (Some(_), Some(perf_status), Some(data_status))
                        if data_status != perf_status =>
                    {
                        errors.push(format!(
                            "offset {offset}: DOMAIN_DATA status {data_status:#010x} does not match DOMAIN_PERF status {perf_status:#010x}"
                        ));
                    }
                    _ => {}
                }
                match open_frame_hour {
                    Some(open) if open != game_hour_raw => errors.push(format!(
                        "offset {offset}: DOMAIN_DATA hour {game_hour_raw} encountered before FRAME_END closed open frame hour {open}"
                    )),
                    Some(_) => {}
                    None => {
                        validate_next_domain48_hour(
                            &mut errors,
                            last_closed_hour,
                            game_hour_raw,
                            "DOMAIN_DATA",
                            offset,
                        );
                        open_frame_hour = Some(game_hour_raw);
                    }
                }
            }
            compact_contract::KnownRecordType::FrameEnd => {
                frame_end_records += 1;
                *frame_end_by_hour.entry(game_hour_raw).or_default() += 1;
                let mut clean = record_semantics_ok;
                if record_semantics_ok {
                    if let Err(reason) = validate_frame_end_authority(
                        payload,
                        &owned_perf_roster,
                        owned_perf_status_or,
                    ) {
                        clean = false;
                        errors.push(format!("offset {offset}: {reason}"));
                    }
                }
                if perf_by_hour.get(&game_hour_raw).copied().unwrap_or(0) != 1
                    || data_by_hour.get(&game_hour_raw).copied().unwrap_or(0) != 1
                {
                    clean = false;
                }
                match open_frame_hour {
                    Some(open) if open == game_hour_raw => {}
                    Some(open) => {
                        clean = false;
                        errors.push(format!(
                            "offset {offset}: FRAME_END hour {game_hour_raw} does not close open DATA frame hour {open}"
                        ));
                    }
                    None => {
                        clean = false;
                        errors.push(format!(
                            "offset {offset}: FRAME_END hour {game_hour_raw} has no open DATA frame"
                        ));
                    }
                }
                if let Some(closed) = last_closed_hour {
                    match closed.checked_add(1) {
                        Some(expected) if game_hour_raw == expected => {}
                        Some(expected) => {
                            clean = false;
                            errors.push(format!(
                                "offset {offset}: FRAME_END hour {game_hour_raw} follows closed frame hour {closed}; expected {expected}"
                            ));
                        }
                        None => {
                            clean = false;
                            errors.push(format!(
                                "offset {offset}: closed frame hour {closed} exhausted; another frame is forbidden"
                            ));
                        }
                    }
                }
                if clean {
                    open_frame_hour = None;
                    pending_perf_hour = None;
                    pending_perf_status = None;
                    last_closed_hour = Some(game_hour_raw);
                    closed_hours.insert(game_hour_raw);
                    final_record_is_clean_frame_end = true;
                }
                owned_perf_roster.clear();
                owned_perf_status_or = 0;
            }
            compact_contract::KnownRecordType::FreezeCensus => {
                if let Ok(census) = compact_contract::freeze_census_record::FreezeCensusRecordV1::decode(payload) {
                    final_record_is_clean_frame_end = compact_contract::freeze_census_record::release_epilogue_closes_stream_policy(
                        previous_record_closed_world,
                        census.phase(),
                        release_epilogue_seen,
                    );
                    release_epilogue_seen |= census.phase() == compact_contract::freeze_census_record::FreezeCensusPhase::Release;
                }
            }
            compact_contract::KnownRecordType::StreamHeader => {}
            compact_contract::KnownRecordType::DomainGap
            | compact_contract::KnownRecordType::FrameRefused
            | compact_contract::KnownRecordType::FrameDegraded
            | compact_contract::KnownRecordType::CaptureTerminated
            | compact_contract::KnownRecordType::ClockUnreadableInterval
            | compact_contract::KnownRecordType::UntrustedClockRecovery => errors.push(format!(
                "offset {offset}: record type {} is outside the construction proof grammar",
                known_record_type.canonical_name()
            )),
        }

        offset = payload_end;
    }

    if let Some(open) = open_frame_hour {
        errors.push(format!("stream ended with open DATA frame hour {open}"));
    }
    if !final_record_is_clean_frame_end {
        errors.push("stream is not closed by a final canonical FRAME_END record".to_string());
    }

    if domain_perf_records != expected_closed_frames {
        errors.push(format!(
            "expected {expected_closed_frames} domain-48 perf records for {requested_game_hour_advancements} requested advancements, got {domain_perf_records}"
        ));
    }
    if domain_data_records != expected_closed_frames {
        errors.push(format!(
            "expected {expected_closed_frames} domain-48 data records for {requested_game_hour_advancements} requested advancements, got {domain_data_records}"
        ));
    }
    if frame_end_records != expected_closed_frames {
        errors.push(format!(
            "expected {expected_closed_frames} frame-end records for {requested_game_hour_advancements} requested advancements, got {frame_end_records}"
        ));
    }

    let unique_hours_vec: Vec<i32> = closed_hours.iter().copied().collect();
    if unique_hours_vec.len() != expected_closed_frames {
        errors.push(format!(
            "expected {expected_closed_frames} canonically closed frames for {requested_game_hour_advancements} requested advancements, got {}",
            unique_hours_vec.len()
        ));
    }
    for hour in &observed_record_hours {
        let perf_count = perf_by_hour.get(hour).copied().unwrap_or(0);
        let data_count = data_by_hour.get(hour).copied().unwrap_or(0);
        let frame_end_count = frame_end_by_hour.get(hour).copied().unwrap_or(0);
        if perf_count != 1 {
            errors.push(format!(
                "game_hour_raw {hour}: domain-48 perf count {perf_count} != 1"
            ));
        }
        if data_count != 1 {
            errors.push(format!(
                "game_hour_raw {hour}: domain-48 data count {data_count} != 1"
            ));
        }
        if frame_end_count != 1 {
            errors.push(format!(
                "game_hour_raw {hour}: frame-end count {frame_end_count} != 1"
            ));
        }
    }
    let skipped_hours = skipped_hours(&unique_hours_vec);
    let zero_skipped_hours = skipped_hours.is_empty();
    let observed_game_hour_advancements = unique_hours_vec
        .first()
        .zip(unique_hours_vec.last())
        .map(|(first, last)| i64::from(*last) - i64::from(*first));
    if observed_game_hour_advancements != Some(requested_game_hour_advancements as i64) {
        errors.push(format!(
            "observed game-hour advancements {observed_game_hour_advancements:?} do not exactly match requested {requested_game_hour_advancements}"
        ));
    }
    Ok(StreamReport {
        ok: errors.is_empty(),
        errors,
        record_count,
        domain_perf_records,
        domain_data_records,
        frame_end_records,
        domains_seen: domains_seen.into_iter().collect(),
        unique_hours: unique_hours_vec,
        skipped_hours,
        zero_skipped_hours,
        observed_game_hour_advancements,
    })
}

fn validate_frame_end_authority(
    payload: &[u8],
    owned_perf_roster: &BTreeSet<u16>,
    owned_perf_status_or: u32,
) -> Result<(), String> {
    if payload.len() != compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN {
        return Err(format!(
            "FRAME_END payload length {} is not {}",
            payload.len(),
            compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN
        ));
    }
    let emitted_domains = read_u32(payload, 16).expect("fixed FRAME_END emitted count");
    let scheduled_domains = read_u32(payload, 20).expect("fixed FRAME_END scheduled count");
    let status_flags = read_u32(payload, 24).expect("fixed FRAME_END status word");
    let reserved = read_u32(payload, 28).expect("fixed FRAME_END reserved word");
    let observed_owned_perf_domains = u32::try_from(owned_perf_roster.len()).unwrap_or(u32::MAX);

    if emitted_domains != observed_owned_perf_domains
        || scheduled_domains != observed_owned_perf_domains
    {
        return Err(format!(
            "FRAME_END emitted/scheduled domain counts {emitted_domains}/{scheduled_domains} do not match observed owned PERF roster count {observed_owned_perf_domains} ({owned_perf_roster:?})"
        ));
    }
    if reserved != 0 {
        return Err(format!(
            "FRAME_END reserved word is nonzero ({reserved:#010x})"
        ));
    }
    let unknown = compact_contract::compact_status_unknown_flags_v1(status_flags);
    if unknown != 0 {
        return Err(format!(
            "FRAME_END status flags {status_flags:#010x} contain unknown bits {unknown:#010x}"
        ));
    }
    let fatal = status_flags & compact_contract::COMPACT_STATUS_FAILURE_MASK;
    if fatal != 0 {
        return Err(format!(
            "FRAME_END status flags {status_flags:#010x} contain fatal bits {fatal:#010x}"
        ));
    }
    if status_flags != owned_perf_status_or {
        return Err(format!(
            "FRAME_END status flags {status_flags:#010x} do not equal observed owned PERF status OR {owned_perf_status_or:#010x}"
        ));
    }
    Ok(())
}

fn validate_status_word(errors: &mut Vec<String>, origin: &str, status_flags: u32, offset: usize) {
    let unknown = compact_contract::compact_status_unknown_flags_v1(status_flags);
    if unknown != 0 {
        errors.push(format!(
            "offset {offset}: {origin} status flags {status_flags:#010x} contain unknown bits {unknown:#010x}"
        ));
    }
    let fatal = status_flags & compact_contract::COMPACT_STATUS_FAILURE_MASK;
    if fatal != 0 {
        errors.push(format!(
            "offset {offset}: {origin} status flags {status_flags:#010x} contain fatal bits {fatal:#010x}"
        ));
    }
}

fn validate_next_domain48_hour(
    errors: &mut Vec<String>,
    last_closed_hour: Option<i32>,
    game_hour_raw: i32,
    record_label: &str,
    offset: usize,
) {
    let Some(closed) = last_closed_hour else {
        return;
    };
    match closed.checked_add(1) {
        Some(expected) if game_hour_raw == expected => {}
        Some(expected) => errors.push(format!(
            "offset {offset}: {record_label} hour {game_hour_raw} follows closed frame hour {closed}; expected {expected}"
        )),
        None => errors.push(format!(
            "offset {offset}: closed frame hour {closed} exhausted; another record is forbidden"
        )),
    }
}

#[derive(Debug)]
struct ReplayReport {
    ok: bool,
    errors: Vec<String>,
    stream_entry: String,
    manifest_entry: String,
    stream_bytes_equal: bool,
    manifest_bytes_equal: bool,
    non_certifying_metadata_verified: bool,
    stream_sha256: String,
    container_verify_all: bool,
}

fn write_and_verify_replay_container(
    game_id: &str,
    stream_name: &str,
    stream_bytes: &[u8],
    manifest_path: &Path,
    replay_path: &Path,
) -> Result<ReplayReport, String> {
    let stream_payload_path = format!("collector_raw/{stream_name}");
    let manifest_name = manifest_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("native_session_manifest.json");
    let manifest_payload_path = format!("recorder/{manifest_name}");
    let index_bytes = serde_json::to_vec_pretty(&json!({
        "game_id": game_id,
        "schema_version": "native_collector_session_v1",
        "artifact_kind": "lane_local_host_shaped_fixture",
        "production_certifying": false,
        "source_capture_verified": false,
        "snapshots": [],
    }))
    .map_err(|err| format!("serialize replay index failed: {err}"))?;
    let timeline_bytes = serde_json::to_vec_pretty(&json!({
        "format": "hoi4_compact_stream_timeline_v1",
        "stream_format": "compact",
        "stream": stream_payload_path,
        "session_manifest": manifest_payload_path,
        "raw_stream_role": "collector_compact_stream",
        "session_manifest_role": "recorder_manifest",
        "artifact_kind": "lane_local_host_shaped_fixture",
        "production_certifying": false,
        "source_capture_verified": false,
    }))
    .map_err(|err| format!("serialize replay timeline failed: {err}"))?;
    let manifest_bytes =
        fs::read(manifest_path).map_err(|err| format!("read session manifest failed: {err}"))?;
    let entries = vec![
        PackEntry {
            path: format!("payload/{game_id}/index.json"),
            bytes: index_bytes.clone(),
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/timeline.json"),
            bytes: timeline_bytes.clone(),
            role: "timeline",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/{stream_payload_path}"),
            bytes: stream_bytes.to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/{manifest_payload_path}"),
            bytes: manifest_bytes.clone(),
            role: "recorder_manifest",
            hour_index: None,
        },
    ];
    let identity = PackIdentity {
        schema_version: Some("native_collector_session_v1".to_string()),
        created_utc: Some(created_utc_label()),
        ..Default::default()
    };
    let packed = pack(game_id, &entries, &identity, true)
        .map_err(|err| format!("pack replay container failed: {err}"))?;
    fs::write(replay_path, packed).map_err(|err| format!("write replay failed: {err}"))?;

    let container = Container::open(replay_path, &ExpectedIdentity::default())
        .map_err(|err| format!("verify replay open failed: {err}"))?;
    container
        .verify_all()
        .map_err(|err| format!("verify replay entries failed: {err}"))?;
    let stream_entry = format!("payload/{game_id}/{stream_payload_path}");
    let verified_stream = container
        .read_entry_bytes(&stream_entry)
        .map_err(|err| format!("verify replay stream entry failed: {err}"))?;
    let stream_bytes_equal = verified_stream == stream_bytes;
    if !stream_bytes_equal {
        return Err("verify replay stream entry failed: compact bytes changed".to_string());
    }
    let manifest_entry = format!("payload/{game_id}/{manifest_payload_path}");
    let verified_manifest = container
        .read_entry_bytes(&manifest_entry)
        .map_err(|err| format!("verify replay manifest entry failed: {err}"))?;
    let manifest_bytes_equal = verified_manifest == manifest_bytes;
    if !manifest_bytes_equal {
        return Err("verify replay manifest entry failed: bytes changed".to_string());
    }
    let index_entry = format!("payload/{game_id}/index.json");
    let timeline_entry = format!("payload/{game_id}/timeline.json");
    let verified_index = container
        .read_entry_bytes(&index_entry)
        .map_err(|err| format!("verify replay index entry failed: {err}"))?;
    let verified_timeline = container
        .read_entry_bytes(&timeline_entry)
        .map_err(|err| format!("verify replay timeline entry failed: {err}"))?;
    if verified_index != index_bytes || verified_timeline != timeline_bytes {
        return Err("verify replay metadata entry failed: bytes changed".to_string());
    }
    for (label, bytes) in [
        ("index", verified_index.as_slice()),
        ("timeline", verified_timeline.as_slice()),
        ("manifest", verified_manifest.as_slice()),
    ] {
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|err| format!("verify replay {label} metadata JSON failed: {err}"))?;
        if value.get("artifact_kind").and_then(|field| field.as_str())
            != Some("lane_local_host_shaped_fixture")
            || value
                .get("production_certifying")
                .and_then(|field| field.as_bool())
                != Some(false)
            || value
                .get("source_capture_verified")
                .and_then(|field| field.as_bool())
                != Some(false)
        {
            return Err(format!(
                "verify replay {label} metadata lacks exact non-certifying fixture authority"
            ));
        }
    }

    Ok(ReplayReport {
        ok: true,
        errors: Vec::new(),
        stream_entry,
        manifest_entry,
        stream_bytes_equal,
        manifest_bytes_equal,
        non_certifying_metadata_verified: true,
        stream_sha256: Sha256::hex(stream_bytes),
        container_verify_all: true,
    })
}

#[derive(Default)]
struct FakeMemory {
    bytes: BTreeMap<usize, u8>,
}

impl FakeMemory {
    fn write_u16(&mut self, addr: usize, value: u16) {
        self.write_bytes(addr, &value.to_le_bytes());
    }

    fn write_u32(&mut self, addr: usize, value: u32) {
        self.write_bytes(addr, &value.to_le_bytes());
    }

    fn write_i32(&mut self, addr: usize, value: i32) {
        self.write_bytes(addr, &value.to_le_bytes());
    }

    fn write_u64(&mut self, addr: usize, value: u64) {
        self.write_bytes(addr, &value.to_le_bytes());
    }

    fn write_bytes(&mut self, addr: usize, bytes: &[u8]) {
        for (index, byte) in bytes.iter().copied().enumerate() {
            self.bytes.insert(addr + index, byte);
        }
    }
}

impl LaneMemory for FakeMemory {
    fn read_u8(&self, addr: usize) -> Option<u8> {
        self.bytes.get(&addr).copied()
    }
}

struct FixtureMemory {
    mem: FakeMemory,
    roots: CollectorRoots,
}

impl FixtureMemory {
    fn production_shaped(module_base: usize) -> Self {
        let country = 0x1000_0000_0000usize;
        let manager = 0x1000_0000_1000usize;
        let construction_vector = 0x1000_0000_2000usize;
        let state_line = 0x1000_0000_3000usize;
        let province_line = 0x1000_0000_3080usize;
        let state_sub = 0x1000_0000_4000usize;
        let province_sub = 0x1000_0000_4800usize;
        let state_target_location = 0x1000_0000_5000usize;
        let province_target_location = 0x1000_0000_5800usize;

        let state = 0x1000_0001_0000usize;
        let state_building_vector = 0x1000_0001_1000usize;
        let damaged_state_building = 0x1000_0001_2000usize;
        let healthy_state_building = 0x1000_0001_2100usize;
        let state_def = 0x1000_0001_3000usize;
        let state_resources = 0x1000_0001_4000usize;

        let province = 0x1000_0002_0000usize;
        let province_building_vector = 0x1000_0002_1000usize;
        let province_building = 0x1000_0002_2000usize;

        let mut mem = FakeMemory::default();

        mem.write_u64(country + OBJ_PROD_MANAGER, manager as u64);
        mem.write_u64(manager + CONSTRUCTION_BEGIN, construction_vector as u64);
        mem.write_u32(manager + CONSTRUCTION_CAP, 2);
        mem.write_u32(manager + CONSTRUCTION_SIZE, 2);
        mem.write_u64(construction_vector, state_line as u64);
        mem.write_u64(construction_vector + 8, province_line as u64);

        mem.write_u32(state_line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
        mem.write_u32(state_line + LINE_ID, 114);
        mem.write_i32(state_line + LINE_ACTIVE_FACTORIES, 15);
        mem.write_i32(state_line + LINE_COST, 1_080_000_000);
        mem.write_i32(state_line + LINE_PRODUCED, 5_500_000);
        mem.write_u64(state_line + LINE_CONSTRUCTION_SUB, state_sub as u64);
        mem.write_u32(state_sub + CONSTRUCTION_TEMPLATE_ID, 20_013);
        mem.write_u64(
            state_sub + CONSTRUCTION_TARGET_LOCATION_PTR,
            state_target_location as u64,
        );
        mem.write_u32(state_target_location + TARGET_LOCATION_ID, 42);

        mem.write_u32(province_line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
        mem.write_u32(province_line + LINE_ID, 115);
        mem.write_i32(province_line + LINE_ACTIVE_FACTORIES, 10);
        mem.write_i32(province_line + LINE_COST, 700_000_000);
        mem.write_i32(province_line + LINE_PRODUCED, 250_000_000);
        mem.write_u64(province_line + LINE_CONSTRUCTION_SUB, province_sub as u64);
        mem.write_u32(province_sub + CONSTRUCTION_TEMPLATE_ID, 12_790);
        mem.write_u64(
            province_sub + CONSTRUCTION_TARGET_LOCATION_PTR,
            province_target_location as u64,
        );
        mem.write_u32(province_target_location + TARGET_LOCATION_ID, 2068);

        mem.write_u64(
            state + STATE_OFF_BUILDINGS_VEC,
            state_building_vector as u64,
        );
        mem.write_u64(state + STATE_OFF_BUILDINGS_VEC + 8, 2u64 << 32);
        mem.write_u64(state_building_vector, damaged_state_building as u64);
        mem.write_u64(state_building_vector + 8, healthy_state_building as u64);
        write_building(&mut mem, module_base, damaged_state_building, 0x2FAB, 5, 4);
        write_building(&mut mem, module_base, healthy_state_building, 0x4E2D, 8, 8);

        mem.write_u64(state + STATE_OFF_DEF, state_def as u64);
        mem.write_u64(
            state_def + STATE_DEF_OFF_RESOURCES_PTR,
            state_resources as u64,
        );
        mem.write_u32(state_def + STATE_DEF_OFF_RESOURCES_SIZE, 1);
        mem.write_u32(state_def + STATE_DEF_OFF_RESOURCES_CAP, 1);
        mem.write_bytes(state_resources, &[0; STATE_RESOURCE_STRIDE]);
        mem.write_i32(state_resources, 2_500_000);
        mem.write_u32(state_resources + 8, 0x3E03);

        mem.write_u64(
            province + CPROVINCE_OFF_BUILDINGS_VEC,
            province_building_vector as u64,
        );
        mem.write_u64(province + CPROVINCE_OFF_BUILDINGS_VEC + 8, 1u64 << 32);
        mem.write_u64(province_building_vector, province_building as u64);
        write_building(&mut mem, module_base, province_building, 0x4CC1, 2, 2);

        let roots = CollectorRoots {
            module_base,
            game_hour_raw: 123,
            country_roots: vec![CountryConstructionRoot {
                tag_db_id: 369,
                country_ptr: country,
            }],
            state_roots: vec![StateRoot {
                state_id: 42,
                state_ptr: state,
            }],
            province_roots: vec![ProvinceRoot {
                province_id: 2068,
                province_ptr: province,
                state_id: 42,
            }],
        };

        Self { mem, roots }
    }
}

fn write_building(
    mem: &mut FakeMemory,
    module_base: usize,
    building: usize,
    type_id: u32,
    level: u16,
    healthy: u16,
) {
    mem.write_u64(building, (module_base + CBUILDING_VTABLE_RVA) as u64);
    mem.write_u32(building + CBUILDING_OFF_TYPE, type_id);
    mem.write_u16(building + CBUILDING_OFF_LEVEL, level);
    mem.write_u16(building + CBUILDING_OFF_HEALTHY_LEVEL, healthy);
}

fn skipped_hours(hours: &[i32]) -> Vec<i32> {
    let mut skipped = Vec::new();
    for pair in hours.windows(2) {
        let mut next = pair[0] + 1;
        while next < pair[1] {
            skipped.push(next);
            next += 1;
        }
    }
    skipped
}

fn created_utc_label() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs();
    format!("unix_seconds:{secs}")
}

fn duration_micros_u64(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

fn timing_summary_json(values: &[u64]) -> serde_json::Value {
    if values.is_empty() {
        return json!({
            "count": 0,
            "min": 0,
            "max": 0,
            "avg": 0,
        });
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let sum: u128 = sorted.iter().map(|value| u128::from(*value)).sum();
    let avg = (sum / sorted.len() as u128).min(u128::from(u64::MAX)) as u64;
    let p95_index = ((sorted.len() * 95).div_ceil(100)).saturating_sub(1);
    json!({
        "count": sorted.len(),
        "min": sorted[0],
        "max": *sorted.last().unwrap_or(&0),
        "avg": avg,
        "p95": sorted[p95_index],
    })
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn format_hex(value: usize) -> String {
    format!("0x{value:X}")
}

fn path_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_argument_is_exact_and_rejects_out_of_range_or_extra_values() {
        let args = |value: Option<&str>| {
            let mut args = vec!["proof".to_string(), "out".to_string()];
            if let Some(value) = value {
                args.push(value.to_string());
            }
            args
        };
        assert_eq!(parse_requested_game_hour_advancements(&args(None)), Ok(12));
        assert_eq!(
            parse_requested_game_hour_advancements(&args(Some("1"))),
            Ok(1)
        );
        assert_eq!(
            parse_requested_game_hour_advancements(&args(Some("71"))),
            Ok(71)
        );
        for rejected in ["0", "72", "not-a-number"] {
            assert!(
                parse_requested_game_hour_advancements(&args(Some(rejected))).is_err(),
                "duration {rejected:?} was silently accepted"
            );
        }
        let mut extra = args(Some("1"));
        extra.push("unexpected".to_string());
        assert!(parse_requested_game_hour_advancements(&extra).is_err());
    }

    #[test]
    fn embedded_manifest_is_explicitly_fixture_only_and_non_certifying() {
        let manifest = fixture_manifest("fixture-game", 5, 6);
        assert_eq!(manifest["artifact_kind"], "lane_local_host_shaped_fixture");
        assert_eq!(manifest["production_certifying"], false);
        assert_eq!(manifest["source_capture_verified"], false);
        assert_eq!(manifest["requested_game_hour_advancements"], 5);
        assert_eq!(manifest["expected_closed_frames"], 6);
    }

    #[test]
    fn replay_container_readback_preserves_non_certifying_metadata() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "d48_noncert_metadata_{}_{}",
            std::process::id(),
            unique
        ));
        fs::create_dir_all(&dir).expect("create test directory");
        let manifest_path = dir.join("native_session_manifest.json");
        let replay_path = dir.join("fixture.battleplan");
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&fixture_manifest("fixture-game", 1, 2)).unwrap(),
        )
        .expect("write fixture manifest");

        let report = write_and_verify_replay_container(
            "fixture-game",
            "collector_stream.bin",
            b"fixture compact bytes",
            &manifest_path,
            &replay_path,
        )
        .expect("container roundtrip");
        assert!(report.ok);
        assert!(report.manifest_bytes_equal);
        assert!(report.non_certifying_metadata_verified);

        let _ = fs::remove_file(replay_path);
        let _ = fs::remove_file(manifest_path);
        let _ = fs::remove_dir(dir);
    }

    #[test]
    fn stream_verifier_enforces_canonical_record_flag_placement() {
        let mut legal = build_compact_stream(&valid_hour_payloads());
        replace_record_payload(&mut legal, 0, &perf_payload_with_empty_diag());
        legal[28..32].copy_from_slice(
            &compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER.to_le_bytes(),
        );
        assert!(verify_domain48_stream(&legal, 0).unwrap().ok);

        let mut illegal = build_compact_stream(&valid_hour_payloads());
        let data_offset = record_offsets(&illegal)[1];
        illegal[data_offset + 28..data_offset + 32].copy_from_slice(
            &compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER.to_le_bytes(),
        );
        let report = verify_domain48_stream(&illegal, 0).unwrap();
        assert!(!report.ok);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("contain rejected bits")));

        let mut illegal_frame = build_compact_stream(&valid_hour_payloads());
        let frame_offset = record_offsets(&illegal_frame)[2];
        illegal_frame[frame_offset + 28..frame_offset + 32].copy_from_slice(
            &compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER.to_le_bytes(),
        );
        let report = verify_domain48_stream(&illegal_frame, 0).unwrap();
        assert!(!report.ok);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("contain rejected bits") && error.contains("type 2")));
    }

    #[test]
    fn stream_verifier_rejects_missing_gapped_and_wrapping_sequences() {
        let mut missing = build_compact_stream(&valid_hour_payloads());
        missing[8..16].copy_from_slice(&2u64.to_le_bytes());
        assert!(verify_domain48_stream(&missing, 0)
            .unwrap()
            .errors
            .iter()
            .any(|error| error.contains("does not start at 1")));

        let mut gap = build_compact_stream(&valid_hour_payloads());
        let data_offset = record_offsets(&gap)[1];
        gap[data_offset + 8..data_offset + 16].copy_from_slice(&3u64.to_le_bytes());
        assert!(verify_domain48_stream(&gap, 0)
            .unwrap()
            .errors
            .iter()
            .any(|error| error.contains("sequence discontinuity 1 -> 3")));

        let mut wrapping = build_compact_stream(&valid_hour_payloads());
        let data_offset = record_offsets(&wrapping)[1];
        wrapping[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        wrapping[data_offset + 8..data_offset + 16].copy_from_slice(&0u64.to_le_bytes());
        assert!(verify_domain48_stream(&wrapping, 0)
            .unwrap()
            .errors
            .iter()
            .any(|error| error.contains("wrapping is not allowed")));
    }

    #[test]
    fn stream_verifier_rejects_missing_and_unannounced_perf_trailers_and_bad_frame_size() {
        let mut long_perf = build_compact_stream(&valid_hour_payloads());
        replace_record_payload(
            &mut long_perf,
            0,
            &[0; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN + 1],
        );
        assert!(verify_domain48_stream(&long_perf, 0)
            .unwrap()
            .errors
            .iter()
            .any(|error| error.contains("unannounced trailer byte")));

        let mut announced_without_trailer = build_compact_stream(&valid_hour_payloads());
        announced_without_trailer[28..32].copy_from_slice(
            &compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER.to_le_bytes(),
        );
        assert!(verify_domain48_stream(&announced_without_trailer, 0)
            .unwrap()
            .errors
            .iter()
            .any(|error| error.contains("diagnostic trailer header is truncated")));

        let mut short_frame = build_compact_stream(&valid_hour_payloads());
        let frame_offset = record_offsets(&short_frame)[2];
        replace_record_payload(
            &mut short_frame,
            frame_offset,
            &[0; compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN - 1],
        );
        assert!(verify_domain48_stream(&short_frame, 0)
            .unwrap()
            .errors
            .iter()
            .any(|error| error.contains("FRAME_END payload length 31 is not 32")));
    }

    #[test]
    fn stream_verifier_rejects_cross_hour_data_and_trailing_closed_hour_records() {
        let mut mismatched = build_compact_stream(&valid_hour_payloads());
        let data_offset = record_offsets(&mismatched)[1];
        mismatched[data_offset + 16..data_offset + 20].copy_from_slice(&124i32.to_le_bytes());
        assert!(verify_domain48_stream(&mismatched, 0)
            .unwrap()
            .errors
            .iter()
            .any(|error| error.contains("conflicts with pending PERF hour 123")));

        let mut trailing = build_compact_stream(&valid_hour_payloads());
        append_compact_record(
            &mut trailing,
            4,
            123,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &[0; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN],
        );
        let report = verify_domain48_stream(&trailing, 0).unwrap();
        assert!(report.errors.iter().any(|error| {
            error.contains("follows closed frame hour 123")
                || error.contains("final canonical FRAME_END")
        }));
    }

    #[test]
    fn stream_verifier_requires_frame_end_at_physical_eof() {
        let mut stream = build_compact_stream(&valid_hour_payloads());
        stream.extend_from_slice(b"tail");
        let report = verify_domain48_stream(&stream, 0).unwrap();
        assert!(!report.ok);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("truncated compact stream header")));
        assert!(report
            .errors
            .iter()
            .any(|error| error == "stream is not closed by a final canonical FRAME_END record"));
    }

    #[test]
    fn stream_verifier_requires_consecutive_closed_hours_and_reports_duration() {
        let mut stream = build_compact_stream(&valid_hour_payloads_for_hours(&[123, 124]));
        let report = verify_domain48_stream(&stream, 1).unwrap();
        assert!(report.ok, "{:?}", report.errors);
        assert_eq!(report.observed_game_hour_advancements, Some(1));

        let too_short = build_compact_stream(&valid_hour_payloads());
        let report = verify_domain48_stream(&too_short, 1).unwrap();
        assert!(!report.ok);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("expected 2 canonically closed frames")));
        assert!(report.errors.iter().any(|error| {
            error.contains(
                "observed game-hour advancements Some(0) do not exactly match requested 1",
            )
        }));

        for record_offset in record_offsets(&stream).into_iter().skip(3) {
            stream[record_offset + 16..record_offset + 20].copy_from_slice(&125i32.to_le_bytes());
        }
        let report = verify_domain48_stream(&stream, 1).unwrap();
        assert!(!report.ok);
        assert!(report.errors.iter().any(|error| {
            error.contains("hour 125 follows closed frame hour 123; expected 124")
        }));
    }

    #[test]
    fn stream_verifier_rejects_data_before_perf() {
        let stream = build_compact_stream(&valid_hour_payloads());
        let offsets = record_offsets(&stream);
        let mut reordered = Vec::new();
        reordered.extend_from_slice(&stream[offsets[1]..offsets[2]]);
        reordered[8..16].copy_from_slice(&1u64.to_le_bytes());
        let perf_offset = reordered.len();
        reordered.extend_from_slice(&stream[offsets[0]..offsets[1]]);
        reordered[perf_offset + 8..perf_offset + 16].copy_from_slice(&2u64.to_le_bytes());
        reordered.extend_from_slice(&stream[offsets[2]..]);

        let report = verify_domain48_stream(&reordered, 0).unwrap();
        assert!(!report.ok);
        assert!(report.errors.iter().any(|error| {
            error.contains("DOMAIN_DATA hour 123 appears before its owned DOMAIN_PERF authority")
        }));
    }

    #[test]
    fn stream_verifier_binds_data_status_to_perf_and_frame_end_to_perf_or() {
        let informational = compact_contract::COMPACT_STATUS_EXTRACTION_PERF_GATE;

        let mut data_mismatch = build_compact_stream(&valid_hour_payloads());
        let data_offset = record_offsets(&data_mismatch)[1];
        let data_status_offset = data_offset + STREAM_HEADER_LEN as usize + 12;
        data_mismatch[data_status_offset..data_status_offset + 4]
            .copy_from_slice(&informational.to_le_bytes());
        let report = verify_domain48_stream(&data_mismatch, 0).unwrap();
        assert!(!report.ok);
        assert!(report.errors.iter().any(|error| {
            error.contains("DOMAIN_DATA status") && error.contains("does not match DOMAIN_PERF")
        }));

        let mut frame_mismatch = build_compact_stream(&valid_hour_payloads());
        let frame_offset = record_offsets(&frame_mismatch)[2];
        let frame_status_offset = frame_offset + STREAM_HEADER_LEN as usize + 24;
        frame_mismatch[frame_status_offset..frame_status_offset + 4]
            .copy_from_slice(&informational.to_le_bytes());
        let report = verify_domain48_stream(&frame_mismatch, 0).unwrap();
        assert!(!report.ok);
        assert!(report.errors.iter().any(|error| {
            error.contains("FRAME_END status flags")
                && error.contains("do not equal observed owned PERF status OR")
        }));

        let mut reverse_frame_mismatch = build_compact_stream(&valid_hour_payloads());
        let offsets = record_offsets(&reverse_frame_mismatch);
        let perf_status_offset = offsets[0] + STREAM_HEADER_LEN as usize + 32;
        reverse_frame_mismatch[perf_status_offset..perf_status_offset + 4]
            .copy_from_slice(&informational.to_le_bytes());
        let data_status_offset = offsets[1] + STREAM_HEADER_LEN as usize + 12;
        reverse_frame_mismatch[data_status_offset..data_status_offset + 4]
            .copy_from_slice(&informational.to_le_bytes());
        let report = verify_domain48_stream(&reverse_frame_mismatch, 0).unwrap();
        assert!(!report.ok);
        assert!(report.errors.iter().any(|error| {
            error.contains("FRAME_END status flags")
                && error.contains("do not equal observed owned PERF status OR")
        }));
    }

    #[test]
    fn frame_end_authority_binds_counts_to_owned_perf_roster() {
        for (label, emitted, scheduled) in
            [("zero", 0u32, 0u32), ("inflated", 2, 2), ("mismatch", 1, 0)]
        {
            let mut stream = build_compact_stream(&valid_hour_payloads());
            let frame_offset = record_offsets(&stream)[2];
            let frame_payload = frame_offset + STREAM_HEADER_LEN as usize;
            stream[frame_payload + 16..frame_payload + 20].copy_from_slice(&emitted.to_le_bytes());
            stream[frame_payload + 20..frame_payload + 24]
                .copy_from_slice(&scheduled.to_le_bytes());
            let report = verify_domain48_stream(&stream, 0).unwrap();
            assert!(!report.ok, "{label} counts were accepted");
            assert!(
                report.errors.iter().any(|error| {
                    error.contains("do not match observed owned PERF roster count 1")
                }),
                "{label}: {:?}",
                report.errors
            );
        }

        let mut wrong_roster = build_compact_stream(&valid_hour_payloads());
        wrong_roster[20..22].copy_from_slice(&49u16.to_le_bytes());
        let report = verify_domain48_stream(&wrong_roster, 0).unwrap();
        assert!(!report.ok);
        assert!(report
            .errors
            .iter()
            .any(|error| { error.contains("do not match observed owned PERF roster count 0") }));
    }

    #[test]
    fn frame_end_authority_rejects_reserved_unknown_and_fatal_status() {
        for (label, payload_word_offset, value, expected) in [
            ("reserved", 28usize, 1u32, "reserved word is nonzero"),
            ("unknown", 24, 1u32 << 31, "contain unknown bits"),
            (
                "fatal",
                24,
                compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
                "contain fatal bits",
            ),
        ] {
            let mut stream = build_compact_stream(&valid_hour_payloads());
            let frame_offset = record_offsets(&stream)[2];
            let field_offset = frame_offset + STREAM_HEADER_LEN as usize + payload_word_offset;
            stream[field_offset..field_offset + 4].copy_from_slice(&value.to_le_bytes());
            let report = verify_domain48_stream(&stream, 0).unwrap();
            assert!(!report.ok, "{label} field was accepted");
            assert!(
                report.errors.iter().any(|error| error.contains(expected)),
                "{label}: {:?}",
                report.errors
            );
        }
    }

    #[test]
    fn stream_verifier_rejects_unknown_and_fatal_owned_record_status() {
        for (origin, record_index, payload_status_offset, status, expected) in [
            (
                "DOMAIN_PERF",
                0usize,
                32usize,
                1u32 << 31,
                "contain unknown bits",
            ),
            (
                "DOMAIN_DATA",
                1,
                12,
                compact_contract::COMPACT_STATUS_NO_CLOCK,
                "contain fatal bits",
            ),
        ] {
            let mut stream = build_compact_stream(&valid_hour_payloads());
            let record_offset = record_offsets(&stream)[record_index];
            let field_offset = record_offset + STREAM_HEADER_LEN as usize + payload_status_offset;
            stream[field_offset..field_offset + 4].copy_from_slice(&status.to_le_bytes());
            let report = verify_domain48_stream(&stream, 0).unwrap();
            assert!(!report.ok, "{origin} status was accepted");
            assert!(
                report
                    .errors
                    .iter()
                    .any(|error| error.contains(origin) && error.contains(expected)),
                "{origin}: {:?}",
                report.errors
            );
        }
    }

    #[test]
    fn stream_verifier_fails_closed_on_malformed_domain_data() {
        let mut malformed = build_compact_stream(&valid_hour_payloads());
        let data_offset = record_offsets(&malformed)[1];
        replace_record_payload(&mut malformed, data_offset, &[]);
        let report = verify_domain48_stream(&malformed, 0).unwrap();
        assert!(!report.ok);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("domain-48 compact payload rejected")));
    }

    fn valid_hour_payloads() -> Vec<HourPayload> {
        valid_hour_payloads_for_hours(&[123])
    }

    fn valid_hour_payloads_for_hours(hours: &[i32]) -> Vec<HourPayload> {
        let fixture = FixtureMemory::production_shaped(0x1400_0000);
        hours
            .iter()
            .map(|game_hour_raw| {
                let mut roots = fixture.roots.clone();
                roots.game_hour_raw = *game_hour_raw;
                let compact_output = collect_domain48_compact(&fixture.mem, roots);
                let decoded =
                    decode_domain48_compact(&compact_output.compact).expect("valid fixture");
                HourPayload {
                    game_hour_raw: *game_hour_raw,
                    compact: compact_output.compact,
                    collect_time_us: 1,
                    status_flags: compact_output.gate.status_flags,
                    source_flags: compact_output.gate.source_flags,
                    construction_rows: decoded.summary.construction_rows,
                    construction_rows_with_target_state: decoded
                        .summary
                        .construction_rows_with_target_state,
                    target_state_unavailable_badged: decoded
                        .summary
                        .target_state_unavailable_badged,
                    building_rows: decoded.summary.building_rows,
                    damaged_building_rows: decoded.summary.damaged_building_rows,
                    resource_rows: decoded.summary.resource_rows,
                }
            })
            .collect()
    }

    fn perf_payload_with_empty_diag() -> Vec<u8> {
        let mut payload = vec![0; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN];
        payload.extend_from_slice(compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC);
        payload.extend_from_slice(
            &compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes(),
        );
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload
    }

    fn record_offsets(raw: &[u8]) -> Vec<usize> {
        let mut offsets = Vec::new();
        let mut offset = 0usize;
        while offset < raw.len() {
            offsets.push(offset);
            let header = parse_compact_record_header(&raw[offset..]).expect("test stream header");
            offset += header.record_len();
        }
        offsets
    }

    fn replace_record_payload(raw: &mut Vec<u8>, record_offset: usize, payload: &[u8]) {
        let header =
            parse_compact_record_header(&raw[record_offset..]).expect("test stream header");
        let payload_start = record_offset + STREAM_HEADER_LEN as usize;
        let payload_end = payload_start + header.payload_len;
        raw.splice(payload_start..payload_end, payload.iter().copied());
        raw[record_offset + 24..record_offset + 28]
            .copy_from_slice(&(payload.len() as u32).to_le_bytes());
    }
}
