use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use construction_buildings_state_damage_live::collector_module::{
    collect_domain48_compact, CollectorRoots, COLLECTOR_MODULE_NAME,
};
use construction_buildings_state_damage_live::decoder_gate::{
    decode_domain48_compact, DECODER_MODULE_NAME,
};
use construction_buildings_state_damage_live::{
    CountryConstructionRoot, LaneMemory, ProvinceRoot, StateRoot, BUILD_PROFILE,
    CBUILDING_OFF_HEALTHY_LEVEL, CBUILDING_OFF_LEVEL, CBUILDING_OFF_TYPE, CBUILDING_VTABLE_RVA,
    CONSTRUCTION_BEGIN, CONSTRUCTION_CAP, CONSTRUCTION_SIZE, CONSTRUCTION_TEMPLATE_ID,
    CPROVINCE_OFF_BUILDINGS_VEC, DOMAIN_ID, DOMAIN_NAME, LINE_ACTIVE_FACTORIES,
    LINE_CONSTRUCTION_SUB, LINE_COST, LINE_ID, LINE_PRODUCED, LINE_TYPE, LINE_TYPE_CONSTRUCTION,
    OBJ_PROD_MANAGER, SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE,
    STATE_DEF_OFF_RESOURCES_CAP, STATE_DEF_OFF_RESOURCES_PTR, STATE_DEF_OFF_RESOURCES_SIZE,
    STATE_OFF_BUILDINGS_VEC, STATE_OFF_DEF, STATE_RESOURCE_STRIDE,
};
use serde_json::json;

fn main() -> Result<(), String> {
    let out_dir = PathBuf::from(env::args().nth(1).ok_or_else(|| {
        "usage: construction_buildings_state_damage_mini_stack_proof <out-dir>".to_string()
    })?);
    fs::create_dir_all(&out_dir)
        .map_err(|err| format!("failed to create {}: {err}", out_dir.display()))?;

    let module_base = 0x1400_0000usize;
    let fixture = FixtureMemory::production_shaped(module_base);
    let compact_output = collect_domain48_compact(&fixture.mem, fixture.roots);
    let decoded = decode_domain48_compact(&compact_output.compact)?;

    let compact_path =
        out_dir.join("construction_buildings_state_damage_live.domain48.compact.bin");
    fs::write(&compact_path, &compact_output.compact)
        .map_err(|err| format!("failed to write {}: {err}", compact_path.display()))?;

    let source_unavailable = compact_output.gate.status_flags
        & construction_buildings_state_damage_live::STATUS_SOURCE_UNAVAILABLE
        != 0;
    let partial_data = compact_output.gate.status_flags
        & construction_buildings_state_damage_live::STATUS_PARTIAL_DATA
        != 0;
    let worker_panic = compact_output.gate.status_flags
        & construction_buildings_state_damage_live::STATUS_WORKER_PANIC
        != 0;
    let target_state_absent_and_badged = decoded.summary.construction_rows_with_target_state == 0
        && decoded.summary.target_state_unavailable_badged;

    let proof = json!({
        "schema": "construction_buildings_state_damage_live_mini_stack_proof_v1",
        "proof_kind": "lane_owned_collector_encoder_decoder_gate",
        "production_complete": false,
        "domain": {
            "id": DOMAIN_ID,
            "name": DOMAIN_NAME,
            "build_profile": BUILD_PROFILE,
            "compact_bin": path_slash(&compact_path),
        },
        "lane_modules": {
            "collector": COLLECTOR_MODULE_NAME,
            "decoder_gate": DECODER_MODULE_NAME,
            "collector_contract": "shared DLL collector registration passes module_base, game_hour_raw, shared country construction roots, shared state roots, and shared province roots; module owns extraction and compact encoding",
            "decoder_contract": "shared recorder registration passes domain-48 compact payload bytes to lane decoder/gate; module owns row validation and summary counters",
        },
        "collector_result": {
            "status_flags": compact_output.gate.status_flags,
            "source_flags": compact_output.gate.source_flags,
            "stats": {
                "failed_country_roots": compact_output.stats.failed_country_roots,
                "failed_state_building_roots": compact_output.stats.failed_state_building_roots,
                "failed_state_resource_roots": compact_output.stats.failed_state_resource_roots,
                "failed_province_building_roots": compact_output.stats.failed_province_building_roots,
            },
            "observed_country_roots": decoded.frame.observed_country_roots,
            "observed_state_roots": decoded.frame.observed_state_roots,
            "observed_province_roots": decoded.frame.observed_province_roots,
        },
        "decoder_gate": {
            "ok": decoded.gate.ok,
            "errors": decoded.gate.errors,
            "summary": {
                "domain_id": decoded.summary.domain_id,
                "domain_name": decoded.summary.domain_name,
                "construction_rows": decoded.summary.construction_rows,
                "construction_rows_with_target_state": decoded.summary.construction_rows_with_target_state,
                "target_state_unavailable_badged": decoded.summary.target_state_unavailable_badged,
                "building_rows": decoded.summary.building_rows,
                "damaged_building_rows": decoded.summary.damaged_building_rows,
                "resource_rows": decoded.summary.resource_rows,
            },
        },
        "target_state_fail_closed": {
            "ok": target_state_absent_and_badged,
            "source_flag": "SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE",
            "source_flag_value": format!("0x{:X}", SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE),
            "candidate_offsets_not_emitted": ["subobject+0xC0", "subobject+0xC8"],
            "reason": "target-state semantics remain fixture-blocked; construction rows are still emitted without guessed target states",
        },
        "production_health_gate": {
            "source_unavailable": source_unavailable,
            "partial_data": partial_data,
            "worker_panic": worker_panic,
            "bridge_failure": false,
            "lane_gate_errors": decoded.gate.errors,
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
        "live_proof_plan": {
            "blocked_by_shared_slot": "air owns active shared collector/recorder integration slot; do not edit shared entrypoints",
            "registration_only_patch_plan": [
                "Add lane crate path dependency to collector and recorder_host or move lane module under the orchestrator-provided module interface.",
                "Register domain_registry id 48 and scheduled token construction_buildings_state_damage_live.",
                "Collector hook: pass existing shared country construction roots, state roots, province roots, module_base, and game_hour_raw into collector_module::collect_domain48_compact.",
                "Recorder hook: dispatch domain 48 bytes to decoder_gate::decode_domain48_compact and add summary counters for construction_rows, building_rows, damaged_building_rows, resource_rows, and target_state_unavailable_badged.",
                "Run compact no-bridge capture at speed five/current equivalent with --scheduled-domain construction_buildings_state_damage_live and verify .battleplan.",
            ],
            "acceptance": {
                "dll_only": true,
                "compact_stream": true,
                "rust_decoded": true,
                "hoi4replay_verified": true,
                "hourly_or_lower": true,
                "zero_skipped_hours": true,
                "no_partial_source_worker_bridge_failures": true,
                "target_state": "must remain absent with unavailable badge until oracle fixture validates a root",
            },
        },
    });

    let proof_path = out_dir.join("mini_stack_proof.json");
    fs::write(
        &proof_path,
        serde_json::to_string_pretty(&proof).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("failed to write {}: {err}", proof_path.display()))?;

    if !decoded.gate.ok
        || !target_state_absent_and_badged
        || source_unavailable
        || partial_data
        || worker_panic
    {
        return Err(format!(
            "mini stack proof failed; wrote {}",
            proof_path.display()
        ));
    }

    Ok(())
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
        let line = 0x1000_0000_3000usize;
        let sub = 0x1000_0000_4000usize;

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
        mem.write_u32(manager + CONSTRUCTION_CAP, 1);
        mem.write_u32(manager + CONSTRUCTION_SIZE, 1);
        mem.write_u64(construction_vector, line as u64);
        mem.write_u32(line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
        mem.write_u32(line + LINE_ID, 114);
        mem.write_i32(line + LINE_ACTIVE_FACTORIES, 15);
        mem.write_i32(line + LINE_COST, 10_000_000);
        mem.write_i32(line + LINE_PRODUCED, 5_500_000);
        mem.write_u64(line + LINE_CONSTRUCTION_SUB, sub as u64);
        mem.write_u32(sub + CONSTRUCTION_TEMPLATE_ID, 20_013);

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

fn format_hex(value: usize) -> String {
    format!("0x{value:X}")
}

fn path_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
