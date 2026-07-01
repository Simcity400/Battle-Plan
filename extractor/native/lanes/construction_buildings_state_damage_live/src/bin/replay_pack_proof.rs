use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
        "usage: construction_buildings_state_damage_replay_pack_proof <out-dir> [hours]".to_string()
    })?);
    let scheduled_hours = args
        .get(2)
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|err| format!("invalid hours value {value:?}: {err}"))
        })
        .transpose()?
        .unwrap_or(12)
        .clamp(1, 72);
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

    for hour_index in 0..scheduled_hours {
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

    let stream_report = verify_domain48_stream(&compact_stream, scheduled_hours as usize)?;
    if !stream_report.ok {
        return Err(format!(
            "domain-48 compact stream proof failed: {:?}",
            stream_report.errors
        ));
    }

    let game_id = "construction_buildings_state_damage_live_domain48_proof";
    let replay_path = out_dir.join("construction_buildings_state_damage_live.domain48.battleplan");
    let manifest_path = out_dir.join("native_session_manifest.json");
    let manifest = json!({
        "schema": "native_session_manifest_v1",
        "proof_schema": "construction_buildings_state_damage_replay_pack_proof_v1",
        "replay_game_id": game_id,
        "stream_format": "compact",
        "stream_file": "collector_stream.bin",
        "scheduled_domains": [DOMAIN_NAME],
        "expected_domain_ids": [DOMAIN_ID],
        "scheduled_snapshot_step_hours": 1,
        "scheduled_game_hours": scheduled_hours,
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
    });
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
        "proof_kind": "lane_owned_host_shaped_hourly_schedule_compact_stream_hoi4replay_roundtrip",
        "production_complete": false,
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
            "scheduled_hours": scheduled_hours,
            "first_game_hour_raw": first_game_hour_raw,
            "last_game_hour_raw": first_game_hour_raw + scheduled_hours as i32 - 1,
            "per_hour_domain48_perf_records": 1,
            "per_hour_domain48_data_records": 1,
            "per_hour_frame_end_records": 1,
            "target_state_all_rows_all_hours": total_construction_rows > 0 && total_construction_rows == total_construction_rows_with_target_state,
            "zero_skipped_hours": stream_report.zero_skipped_hours,
            "collect_time_us": timing_summary_json(&collect_times_us),
            "hour_summaries": hour_summaries,
            "host_shape_limit": "does not edit or invoke shared collector entrypoints; proves the registered lane collector/encoder/gate can satisfy one domain-48 compact data record every scheduled game hour",
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
            "stream_bytes_equal": replay_report.stream_bytes_equal,
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
        sequence = sequence.saturating_add(1);
        append_compact_record(
            &mut stream,
            sequence,
            hour.game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &hour.compact,
        );
        sequence = sequence.saturating_add(1);
        append_compact_record(
            &mut stream,
            sequence,
            hour.game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_end_payload(hour.collect_time_us, hour.status_flags),
        );
        sequence = sequence.saturating_add(1);
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
    out.extend_from_slice(&(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
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
}

fn verify_domain48_stream(raw: &[u8], expected_hours: usize) -> Result<StreamReport, String> {
    let mut errors = Vec::new();
    let mut offset = 0usize;
    let mut previous_sequence = None;
    let mut record_count = 0usize;
    let mut domain_perf_records = 0usize;
    let mut domain_data_records = 0usize;
    let mut frame_end_records = 0usize;
    let mut domains_seen = BTreeSet::new();
    let mut unique_hours = BTreeSet::new();
    let mut perf_by_hour: BTreeMap<i32, usize> = BTreeMap::new();
    let mut data_by_hour: BTreeMap<i32, usize> = BTreeMap::new();
    let mut frame_end_by_hour: BTreeMap<i32, usize> = BTreeMap::new();

    while offset < raw.len() {
        if raw.len().saturating_sub(offset) < STREAM_HEADER_LEN as usize {
            errors.push(format!("offset {offset}: truncated compact stream header"));
            break;
        }
        let header = &raw[offset..offset + STREAM_HEADER_LEN as usize];
        if &header[0..4] != STREAM_MAGIC {
            errors.push(format!("offset {offset}: invalid compact stream magic"));
            break;
        }
        let version = read_u16(header, 4).ok_or("missing stream version")?;
        let header_len = read_u16(header, 6).ok_or("missing stream header length")?;
        if version != STREAM_VERSION {
            errors.push(format!(
                "offset {offset}: unsupported stream version {version}"
            ));
            break;
        }
        if header_len != STREAM_HEADER_LEN {
            errors.push(format!(
                "offset {offset}: unsupported header_len {header_len}"
            ));
            break;
        }
        let sequence = read_u64(header, 8).ok_or("missing stream sequence")?;
        let game_hour_raw = read_i32(header, 16).ok_or("missing stream hour")?;
        let domain_id = read_u16(header, 20).ok_or("missing stream domain id")?;
        let record_type = read_u16(header, 22).ok_or("missing stream record type")?;
        let payload_len = read_u32(header, 24).ok_or("missing stream payload len")? as usize;
        let payload_start = offset + STREAM_HEADER_LEN as usize;
        let payload_end = payload_start.saturating_add(payload_len);
        if payload_end > raw.len() {
            errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining stream bytes"
            ));
            break;
        }
        if let Some(previous) = previous_sequence {
            if sequence <= previous {
                errors.push(format!(
                    "offset {offset}: sequence {sequence} did not advance past {previous}"
                ));
            }
        }
        previous_sequence = Some(sequence);
        record_count += 1;
        domains_seen.insert(domain_id);
        unique_hours.insert(game_hour_raw);
        let payload = &raw[payload_start..payload_end];

        match record_type {
            RECORD_TYPE_DOMAIN_PERF => {
                domain_perf_records += 1;
                *perf_by_hour.entry(game_hour_raw).or_default() += 1;
                if domain_id != DOMAIN_ID {
                    errors.push(format!(
                        "offset {offset}: unexpected perf domain {domain_id}"
                    ));
                }
                if payload.len() < 40 {
                    errors.push(format!("offset {offset}: short domain perf payload"));
                }
            }
            RECORD_TYPE_DOMAIN_DATA => {
                domain_data_records += 1;
                *data_by_hour.entry(game_hour_raw).or_default() += 1;
                if domain_id != DOMAIN_ID {
                    errors.push(format!(
                        "offset {offset}: unexpected data domain {domain_id}"
                    ));
                } else {
                    let decoded = decode_domain48_compact(payload)?;
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
            }
            RECORD_TYPE_FRAME_END => {
                frame_end_records += 1;
                *frame_end_by_hour.entry(game_hour_raw).or_default() += 1;
                if domain_id != DOMAIN_FRAME {
                    errors.push(format!("offset {offset}: frame-end domain {domain_id}"));
                }
                if payload.len() < 32 {
                    errors.push(format!("offset {offset}: short frame-end payload"));
                }
            }
            other => errors.push(format!("offset {offset}: unknown record type {other}")),
        }

        offset = payload_end;
    }

    if domain_perf_records != expected_hours {
        errors.push(format!(
            "expected {expected_hours} domain-48 perf records, got {domain_perf_records}"
        ));
    }
    if domain_data_records != expected_hours {
        errors.push(format!(
            "expected {expected_hours} domain-48 data records, got {domain_data_records}"
        ));
    }
    if frame_end_records != expected_hours {
        errors.push(format!(
            "expected {expected_hours} frame-end records, got {frame_end_records}"
        ));
    }

    let unique_hours_vec: Vec<i32> = unique_hours.iter().copied().collect();
    if unique_hours_vec.len() != expected_hours {
        errors.push(format!(
            "expected {expected_hours} unique scheduled hours, got {}",
            unique_hours_vec.len()
        ));
    }
    for hour in &unique_hours_vec {
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
    })
}

#[derive(Debug)]
struct ReplayReport {
    ok: bool,
    errors: Vec<String>,
    stream_entry: String,
    stream_bytes_equal: bool,
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
    }))
    .map_err(|err| format!("serialize replay timeline failed: {err}"))?;
    let manifest_bytes =
        fs::read(manifest_path).map_err(|err| format!("read session manifest failed: {err}"))?;
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
            path: format!("payload/{game_id}/{stream_payload_path}"),
            bytes: stream_bytes.to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/{manifest_payload_path}"),
            bytes: manifest_bytes,
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

    Ok(ReplayReport {
        ok: true,
        errors: Vec::new(),
        stream_entry,
        stream_bytes_equal,
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

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn read_i32(bytes: &[u8], offset: usize) -> Option<i32> {
    Some(i32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(offset..offset.checked_add(8)?)?.try_into().ok()?,
    ))
}

fn format_hex(value: usize) -> String {
    format!("0x{value:X}")
}

fn path_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
