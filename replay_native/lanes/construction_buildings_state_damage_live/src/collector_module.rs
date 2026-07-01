use crate::{
    encode_lane_frame, gate_lane_frame, read_country_construction_queue_with_target_states,
    read_province_building_rows, read_state_building_rows, read_state_resource_rows,
    CountryConstructionRoot, GateReport, LaneFrame, LaneMemory, ProvinceRoot, StateRoot,
    SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE, SOURCE_FLAG_DLL_NATIVE,
    SOURCE_FLAG_HEALTHY_LEVEL_CANDIDATE, SOURCE_FLAG_SHARED_COUNTRY_ROOTS,
    SOURCE_FLAG_SHARED_MAP_TOPOLOGY, STATUS_PARTIAL_DATA,
};

pub const COLLECTOR_MODULE_NAME: &str = "construction_buildings_state_damage_live.collector";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectorRoots {
    pub module_base: usize,
    pub game_hour_raw: i32,
    pub country_roots: Vec<CountryConstructionRoot>,
    pub state_roots: Vec<StateRoot>,
    pub province_roots: Vec<ProvinceRoot>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CollectorStats {
    pub failed_country_roots: u32,
    pub failed_state_building_roots: u32,
    pub failed_state_resource_roots: u32,
    pub failed_province_building_roots: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectorOutput {
    pub frame: LaneFrame,
    pub stats: CollectorStats,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompactCollectorOutput {
    pub compact: Vec<u8>,
    pub gate: GateReport,
    pub stats: CollectorStats,
}

pub fn collect_domain48_frame<M: LaneMemory>(mem: &M, roots: CollectorRoots) -> CollectorOutput {
    let mut stats = CollectorStats::default();
    let mut construction_rows = Vec::new();
    let mut building_rows = Vec::new();
    let mut resource_rows = Vec::new();

    for root in &roots.country_roots {
        match read_country_construction_queue_with_target_states(
            mem,
            root.clone(),
            &roots.state_roots,
            &roots.province_roots,
        ) {
            Some(mut rows) => {
                construction_rows.append(&mut rows);
            }
            None => stats.failed_country_roots = stats.failed_country_roots.saturating_add(1),
        }
    }

    for root in &roots.state_roots {
        match read_state_building_rows(mem, roots.module_base, *root) {
            Some(mut rows) => building_rows.append(&mut rows),
            None => {
                stats.failed_state_building_roots =
                    stats.failed_state_building_roots.saturating_add(1);
            }
        }
        match read_state_resource_rows(mem, *root) {
            Some(mut rows) => resource_rows.append(&mut rows),
            None => {
                stats.failed_state_resource_roots =
                    stats.failed_state_resource_roots.saturating_add(1);
            }
        }
    }

    for root in &roots.province_roots {
        match read_province_building_rows(mem, roots.module_base, *root) {
            Some(mut rows) => building_rows.append(&mut rows),
            None => {
                stats.failed_province_building_roots =
                    stats.failed_province_building_roots.saturating_add(1);
            }
        }
    }

    let mut status_flags = 0u32;
    if stats.failed_country_roots
        + stats.failed_state_building_roots
        + stats.failed_state_resource_roots
        + stats.failed_province_building_roots
        > 0
    {
        status_flags |= STATUS_PARTIAL_DATA;
    }

    let all_construction_targets_present = !construction_rows.is_empty()
        && construction_rows
            .iter()
            .all(|row| row.target_state_id.is_some());
    if !all_construction_targets_present {
        for row in &mut construction_rows {
            row.target_state_id = None;
        }
    }

    let target_state_source_flag = if all_construction_targets_present {
        0
    } else {
        SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE
    };

    let frame = LaneFrame {
        game_hour_raw: roots.game_hour_raw,
        status_flags,
        source_flags: SOURCE_FLAG_DLL_NATIVE
            | SOURCE_FLAG_SHARED_COUNTRY_ROOTS
            | SOURCE_FLAG_SHARED_MAP_TOPOLOGY
            | SOURCE_FLAG_HEALTHY_LEVEL_CANDIDATE
            | target_state_source_flag,
        observed_country_roots: roots.country_roots.len().min(u32::MAX as usize) as u32,
        observed_state_roots: roots.state_roots.len().min(u32::MAX as usize) as u32,
        observed_province_roots: roots.province_roots.len().min(u32::MAX as usize) as u32,
        construction_rows,
        building_rows,
        resource_rows,
    };

    CollectorOutput { frame, stats }
}

pub fn collect_domain48_compact<M: LaneMemory>(
    mem: &M,
    roots: CollectorRoots,
) -> CompactCollectorOutput {
    let output = collect_domain48_frame(mem, roots);
    let compact = encode_lane_frame(&output.frame);
    let gate = gate_lane_frame(&compact);
    CompactCollectorOutput {
        compact,
        gate,
        stats: output.stats,
    }
}
