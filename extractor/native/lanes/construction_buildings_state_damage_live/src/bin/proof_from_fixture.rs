use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use construction_buildings_state_damage_live::{
    gate_lane_frame, BuildingRow, ConstructionQueueRow, LaneFrame, StateResourceRow, BUILD_PROFILE,
    CBUILDING_OFF_HEALTHY_LEVEL, CBUILDING_OFF_LEVEL, CBUILDING_OFF_TYPE, CBUILDING_VTABLE_RVA,
    CONSTRUCTION_BEGIN, CONSTRUCTION_CAP, CONSTRUCTION_SIZE, CONSTRUCTION_STRIDE,
    CONSTRUCTION_TEMPLATE_ID, DOMAIN_ID, DOMAIN_NAME, LINE_ACTIVE_FACTORIES, LINE_CONSTRUCTION_SUB,
    LINE_COST, LINE_ID, LINE_PRODUCED, LINE_TYPE, LINE_TYPE_CONSTRUCTION, OBJ_PROD_MANAGER,
    SCOPE_PROVINCE, SCOPE_STATE, SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE,
    SOURCE_FLAG_DLL_NATIVE, SOURCE_FLAG_HEALTHY_LEVEL_CANDIDATE, SOURCE_FLAG_SHARED_COUNTRY_ROOTS,
    SOURCE_FLAG_SHARED_MAP_TOPOLOGY, STATE_DEF_OFF_RESOURCES_CAP, STATE_DEF_OFF_RESOURCES_PTR,
    STATE_DEF_OFF_RESOURCES_SIZE, STATE_OFF_BUILDINGS_VEC, STATE_OFF_DEF,
};
use serde_json::json;

fn main() -> Result<(), String> {
    let out_dir = env::args().nth(1).ok_or_else(|| {
        "usage: construction_buildings_state_damage_lane_proof <out-dir>".to_string()
    })?;
    let out_dir = PathBuf::from(out_dir);
    fs::create_dir_all(&out_dir)
        .map_err(|err| format!("failed to create {}: {err}", out_dir.display()))?;

    let frame = LaneFrame {
        game_hour_raw: 0,
        status_flags: 0,
        source_flags: SOURCE_FLAG_DLL_NATIVE
            | SOURCE_FLAG_SHARED_COUNTRY_ROOTS
            | SOURCE_FLAG_SHARED_MAP_TOPOLOGY
            | SOURCE_FLAG_HEALTHY_LEVEL_CANDIDATE
            | SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE,
        observed_country_roots: 87,
        observed_state_roots: 1081,
        observed_province_roots: 13275,
        construction_rows: vec![
            ConstructionQueueRow {
                tag_db_id: 369,
                line_id: 114,
                template_id: Some(20_013),
                active_factories: 15,
                cost_raw: 10_000_000,
                produced_raw: 5_500_000,
                priority: 0,
                target_state_id: None,
                target_location_id: None,
                target_scope: None,
                repair_levels_remaining: Some(1),
                created_date_raw: Some(60_827_043),
            },
            ConstructionQueueRow {
                tag_db_id: 369,
                line_id: 115,
                template_id: Some(20_002),
                active_factories: 3,
                cost_raw: 7_200_000,
                produced_raw: 100_000,
                priority: 1,
                target_state_id: None,
                target_location_id: None,
                target_scope: None,
                repair_levels_remaining: Some(0),
                created_date_raw: None,
            },
        ],
        railway_construction_rows: None,
        building_rows: vec![
            BuildingRow {
                scope: SCOPE_STATE,
                location_id: 42,
                state_id: 42,
                type_id: 0x2FAB,
                level: 5,
                healthy_level: Some(4),
                partial_health_raw: Some(8_184_501),
            },
            BuildingRow {
                scope: SCOPE_STATE,
                location_id: 42,
                state_id: 42,
                type_id: 0x4E2D,
                level: 8,
                healthy_level: Some(8),
                partial_health_raw: Some(0),
            },
            BuildingRow {
                scope: SCOPE_PROVINCE,
                location_id: 2068,
                state_id: 42,
                type_id: 0x4CC1,
                level: 2,
                healthy_level: Some(2),
                partial_health_raw: Some(0),
            },
            BuildingRow {
                scope: SCOPE_PROVINCE,
                location_id: 12406,
                state_id: 42,
                type_id: 0x31F6,
                level: 10,
                healthy_level: Some(10),
                partial_health_raw: Some(0),
            },
        ],
        resource_rows: vec![
            StateResourceRow {
                state_id: 42,
                resource_id: 0x3E03,
                amount_raw: 2_500_000,
            },
            StateResourceRow {
                state_id: 42,
                resource_id: 0x4E37,
                amount_raw: 600_000,
            },
        ],
    };
    let compact = construction_buildings_state_damage_live::encode_lane_frame(&frame);
    let gate = gate_lane_frame(&compact);
    let compact_path = out_dir.join("construction_buildings_state_damage_live.compact.bin");
    fs::write(&compact_path, &compact)
        .map_err(|err| format!("failed to write {}: {err}", compact_path.display()))?;

    let proof = json!({
        "schema": "construction_buildings_state_damage_live_lane_proof_v1",
        "proof_kind": "lane_local_compact_fixture",
        "production_complete": false,
        "completion_note": "Lane-local extraction/schema/gate proof only. Shared DLL collector/recorder integration and speed-five .battleplan proof are still required.",
        "domain": {
            "id": DOMAIN_ID,
            "name": DOMAIN_NAME,
            "build_profile": BUILD_PROFILE,
            "compact_bin": path_slash(&compact_path),
        },
        "validated_or_production_root_offsets": {
            "construction_queue": {
                "country_to_production_manager": format_hex(OBJ_PROD_MANAGER),
                "manager_vector_begin": format_hex(CONSTRUCTION_BEGIN),
                "manager_vector_cap": format_hex(CONSTRUCTION_CAP),
                "manager_vector_size": format_hex(CONSTRUCTION_SIZE),
                "manager_vector_stride": CONSTRUCTION_STRIDE,
                "line_type_offset": format_hex(LINE_TYPE),
                "line_type_value": LINE_TYPE_CONSTRUCTION,
                "line_id_offset": format_hex(LINE_ID),
                "active_factories_offset": format_hex(LINE_ACTIVE_FACTORIES),
                "cost_raw_offset": format_hex(LINE_COST),
                "produced_raw_offset": format_hex(LINE_PRODUCED),
                "building_subobject_offset": format_hex(LINE_CONSTRUCTION_SUB),
                "template_id_offset": format_hex(CONSTRUCTION_TEMPLATE_ID),
                "target_state_status": "not validated; encoded as absent until a production root is proven",
                "target_state_unavailable_source_flag": format_hex_u32(SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE)
            },
            "building_levels": {
                "state_building_vector": format_hex(STATE_OFF_BUILDINGS_VEC),
                "province_building_vector": "0x1C8",
                "cbuilding_vtable_rva": format_hex(CBUILDING_VTABLE_RVA),
                "type_id_offset": format_hex(CBUILDING_OFF_TYPE),
                "level_offset": format_hex(CBUILDING_OFF_LEVEL),
                "healthy_level_candidate_offset": format_hex(CBUILDING_OFF_HEALTHY_LEVEL),
                "healthy_level_status": "candidate carried behind a presence flag; production integration must retain fail-closed validation"
            },
            "state_resources": {
                "state_def_offset": format_hex(STATE_OFF_DEF),
                "resources_ptr_offset": format_hex(STATE_DEF_OFF_RESOURCES_PTR),
                "resources_size_offset": format_hex(STATE_DEF_OFF_RESOURCES_SIZE),
                "resources_cap_offset": format_hex(STATE_DEF_OFF_RESOURCES_CAP)
            }
        },
        "lane_compact_gate": {
            "ok": gate.ok,
            "status_flags": gate.status_flags,
            "source_flags": gate.source_flags,
            "construction_rows": gate.construction_rows,
            "construction_rows_with_target_state": gate.construction_rows_with_target_state,
            "building_rows": gate.building_rows,
            "resource_rows": gate.resource_rows,
            "damaged_building_rows": gate.damaged_building_rows,
            "errors": gate.errors,
        },
        "target_state_badge": {
            "status": "unavailable_fixture_blocked",
            "source_flag": "SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE",
            "source_flag_value": format_hex_u32(SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE),
            "encoded_target_state_rows": gate.construction_rows_with_target_state,
            "candidate_offsets_not_emitted": ["subobject+0xC0", "subobject+0xC8"],
            "fixture_required": "time-aligned save/UI oracle rows from target_candidate_focus_probe.json"
        },
        "negative_evidence_handled": {
            "older_combined_manifest_invalid_construction_rows": "lane extractor and gate reject active_factories outside 0..=100000 and nonzero reserved fields",
            "target_state_location": "not emitted because no validated target-state root is available in the lane-local stack",
            "map_control_occupation_semantics": "not duplicated; building rows carry state/province join keys only"
        }
    });
    let proof_path = out_dir.join("lane_proof.json");
    fs::write(
        &proof_path,
        serde_json::to_string_pretty(&proof).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("failed to write {}: {err}", proof_path.display()))?;
    if !gate.ok {
        return Err(format!(
            "lane proof gate failed; wrote {}",
            proof_path.display()
        ));
    }
    Ok(())
}

fn format_hex(value: usize) -> String {
    format!("0x{value:X}")
}

fn format_hex_u32(value: u32) -> String {
    format!("0x{value:X}")
}

fn path_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
