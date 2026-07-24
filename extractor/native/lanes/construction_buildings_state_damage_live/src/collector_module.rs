use crate::{
    encode_lane_frame, gate_lane_frame, read_country_construction_queue_with_target_lookup,
    read_country_railway_construction_queue, read_province_building_rows,
    read_state_building_rows, read_state_resource_rows,
    CountryConstructionRoot, GateReport, LaneFrame, LaneMemory, ProvinceRoot, StateRoot,
    TargetStateLookup, SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE, SOURCE_FLAG_DLL_NATIVE,
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
    pub skipped_construction_rows: u32,
    pub skipped_construction_invalid_line_ptr_rows: u32,
    pub skipped_construction_non_construction_line_rows: u32,
    pub skipped_construction_invalid_active_factories_rows: u32,
    pub skipped_construction_invalid_progress_rows: u32,
    pub skipped_building_rows: u32,
    pub skipped_resource_rows: u32,
}

impl CollectorStats {
    fn has_partial_data(&self) -> bool {
        self.failed_country_roots > 0
            || self.failed_state_building_roots > 0
            || self.failed_state_resource_roots > 0
            || self.failed_province_building_roots > 0
            || self.skipped_construction_rows > 0
            || self.skipped_building_rows > 0
            || self.skipped_resource_rows > 0
    }
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
    let mut railway_construction_rows = Vec::new();
    let mut building_rows = Vec::new();
    let mut resource_rows = Vec::new();

    let target_lookup = TargetStateLookup::new(&roots.state_roots, &roots.province_roots);
    for root in &roots.country_roots {
        let mut country_failed = false;
        match read_country_construction_queue_with_target_lookup(mem, root.clone(), &target_lookup)
        {
            Some(mut read) => {
                stats.skipped_construction_rows = stats
                    .skipped_construction_rows
                    .saturating_add(read.skipped_rows);
                stats.skipped_construction_invalid_line_ptr_rows = stats
                    .skipped_construction_invalid_line_ptr_rows
                    .saturating_add(read.construction_skip_stats.invalid_line_ptr_rows);
                stats.skipped_construction_non_construction_line_rows = stats
                    .skipped_construction_non_construction_line_rows
                    .saturating_add(read.construction_skip_stats.non_construction_line_rows);
                stats.skipped_construction_invalid_active_factories_rows = stats
                    .skipped_construction_invalid_active_factories_rows
                    .saturating_add(read.construction_skip_stats.invalid_active_factories_rows);
                stats.skipped_construction_invalid_progress_rows = stats
                    .skipped_construction_invalid_progress_rows
                    .saturating_add(read.construction_skip_stats.invalid_progress_rows);
                construction_rows.append(&mut read.rows);
            }
            None => country_failed = true,
        }
        match read_country_railway_construction_queue(mem, roots.module_base, root.clone()) {
            Some(mut read) => {
                // Reuse the existing D48 partial cause: a rejected railway row
                // is an uncertain construction-queue omission and must not be
                // allowed to clear prior fold state.
                stats.skipped_construction_rows = stats
                    .skipped_construction_rows
                    .saturating_add(read.skipped_rows);
                railway_construction_rows.append(&mut read.rows);
            }
            None => country_failed = true,
        }
        if country_failed {
            stats.failed_country_roots = stats.failed_country_roots.saturating_add(1);
        }
    }
    railway_construction_rows.sort_by_key(|row| (row.tag_db_id, row.queue_index, row.line_id));

    for root in &roots.state_roots {
        match read_state_building_rows(mem, roots.module_base, *root) {
            Some(mut read) => {
                stats.skipped_building_rows = stats
                    .skipped_building_rows
                    .saturating_add(read.skipped_rows);
                building_rows.append(&mut read.rows);
            }
            None => {
                stats.failed_state_building_roots =
                    stats.failed_state_building_roots.saturating_add(1);
            }
        }
        match read_state_resource_rows(mem, *root) {
            Some(mut read) => {
                stats.skipped_resource_rows = stats
                    .skipped_resource_rows
                    .saturating_add(read.skipped_rows);
                resource_rows.append(&mut read.rows);
            }
            None => {
                stats.failed_state_resource_roots =
                    stats.failed_state_resource_roots.saturating_add(1);
            }
        }
    }

    for root in &roots.province_roots {
        match read_province_building_rows(mem, roots.module_base, *root) {
            Some(mut read) => {
                stats.skipped_building_rows = stats
                    .skipped_building_rows
                    .saturating_add(read.skipped_rows);
                building_rows.append(&mut read.rows);
            }
            None => {
                stats.failed_province_building_roots =
                    stats.failed_province_building_roots.saturating_add(1);
            }
        }
    }

    let mut status_flags = 0u32;
    if stats.has_partial_data() {
        status_flags |= STATUS_PARTIAL_DATA;
    }

    let all_construction_targets_present = !construction_rows.is_empty()
        && construction_rows
            .iter()
            .all(|row| row.target_state_id.is_some());
    if !all_construction_targets_present {
        badge_construction_target_tuple_unavailable(&mut construction_rows);
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
        railway_construction_rows: Some(railway_construction_rows),
        building_rows,
        resource_rows,
    };

    CollectorOutput { frame, stats }
}

fn badge_construction_target_tuple_unavailable(rows: &mut [crate::ConstructionQueueRow]) {
    for row in rows {
        row.target_state_id = None;
        // Location and scope are one promoted target tuple with the state
        // join. Retaining either half while globally badging the join
        // unavailable creates a self-contradictory wire row.
        row.target_location_id = None;
        row.target_scope = None;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_target_join_clears_the_entire_promoted_tuple() {
        let mut rows = vec![crate::ConstructionQueueRow {
            tag_db_id: 1,
            line_id: 2,
            template_id: Some(12_790),
            active_factories: 1,
            cost_raw: 10,
            produced_raw: 5,
            priority: 0,
            target_state_id: Some(42),
            target_location_id: Some(2_068),
            target_scope: Some(crate::SCOPE_PROVINCE),
            repair_levels_remaining: None,
            created_date_raw: Some(60_000_000),
        }];

        badge_construction_target_tuple_unavailable(&mut rows);

        assert_eq!(rows[0].target_state_id, None);
        assert_eq!(rows[0].target_location_id, None);
        assert_eq!(rows[0].target_scope, None);
        let frame = crate::LaneFrame {
            game_hour_raw: 60_000_000,
            status_flags: 0,
            source_flags: crate::SOURCE_FLAG_DLL_NATIVE
                | crate::SOURCE_FLAG_SHARED_COUNTRY_ROOTS
                | crate::SOURCE_FLAG_SHARED_MAP_TOPOLOGY
                | crate::SOURCE_FLAG_HEALTHY_LEVEL_CANDIDATE
                | crate::SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE,
            observed_country_roots: 1,
            observed_state_roots: 1,
            observed_province_roots: 1,
            construction_rows: rows,
            railway_construction_rows: Some(Vec::new()),
            building_rows: Vec::new(),
            resource_rows: Vec::new(),
        };
        assert!(crate::decode_lane_frame(&crate::encode_lane_frame(&frame)).is_ok());
    }
}
