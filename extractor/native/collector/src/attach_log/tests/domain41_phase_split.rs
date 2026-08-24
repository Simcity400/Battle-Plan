use super::*;

/// Lane DS's six FIRST, then lane RZ's five, then lane TB's six.
///
/// The order is the contract, not an accident, but it buys exactly ONE thing:
/// `allocate_unit_budget` guarantees the first `MAX_FLOOR_UNITS_PER_OFFENDER`
/// (8) units of this list a place on a budget-constrained record, so nothing a
/// later lane appends can push an earlier lane's split out of the floor.
///
/// IT DOES NOT ORDER THE DISCARD. Past the floor the allocator walks the
/// discretionary units and keeps trying later, SMALLER ones after a bigger one
/// does not fit, so a short late name can outlive a long early one — the doc
/// here used to say a truncated record loses the newest sub-split first, and
/// that is false (Codex review 01a033e9-b478-7141-96c8-880dd283e66c,
/// SHOULD-FIX 2). This array pins the builder's emission order, which is what
/// the floor is computed from; it pins nothing about retention beyond it.
const D41_PHASE_UNITS: [&str; 17] = [
    "d41_phase_d18_reread_us",
    "d41_phase_lifecycle_us",
    "d41_phase_cache_rows_us",
    "d41_phase_breakthrough_us",
    "d41_phase_emit_mode_us",
    "d41_phase_total_us",
    "d41_phase_lifecycle_decode_us",
    "d41_phase_lifecycle_sort_us",
    "d41_phase_breakthrough_records_us",
    "d41_phase_breakthrough_classify_us",
    "d41_lifecycle_bulk_fallback_countries",
    "d41_phase_lifecycle_pool_copy_us",
    "d41_specialization_scan_gap_restarts",
    "d41_specialization_scan_completions",
    "d41_specialization_scan_state_sample",
    "d41_specialization_scan_handle_restarts",
    "d41_phase_specialization_sweep_us",
];

fn read_with_phase(status_flags: u32) -> CompactProjectsRead {
    read_with_phase_and_shortfall(status_flags, 0, 0)
}

fn read_with_phase_and_shortfall(
    status_flags: u32,
    facility_row_shortfall: u32,
    scientist_row_shortfall: u32,
) -> CompactProjectsRead {
    CompactProjectsRead {
        facilities: Vec::new(),
        projects: Vec::new(),
        scientists: Vec::new(),
        project_states: Vec::new(),
        breakthrough: Vec::new(),
        phase: Domain41PhaseTimes {
            d18_reread_us: 11,
            lifecycle_us: 22,
            cache_rows_us: 33,
            breakthrough_us: 44,
            emit_mode_us: 55,
            total_us: 66,
            lifecycle_decode_us: 77,
            lifecycle_sort_us: 88,
            breakthrough_records_us: 99,
            breakthrough_classify_us: 110,
            lifecycle_bulk_fallback_countries: 121,
            lifecycle_pool_copy_us: 132,
            specialization_scan_gap_restarts: 143,
            specialization_scan_completions: 154,
            specialization_scan_state_sample: 165,
            specialization_scan_handle_restarts: 176,
            specialization_sweep_us: 187,
        },
        d18_authority: None,
        read_time_us: 0,
        changed_rows: 0,
        status_flags,
        observed_facility_rows: 0,
        observed_project_rows: 0,
        observed_scientist_rows: 0,
        observed_project_state_rows: 0,
        observed_breakthrough_rows: 0,
        scan_ms: 0,
        bridge_rows_supplied: 0,
        source_kind: 0,
        invalid_program_status_countries: 0,
        invalid_facility_countries: 0,
        invalid_project_pool_countries: 0,
        neutral_program_status_countries: 0,
        neutral_facility_countries: 0,
        neutral_project_pool_countries: 0,
        invalid_breakthrough_countries: 0,
        breakthrough_specializations_resolved: true,
        facility_row_shortfall,
        scientist_row_shortfall,
        root_reasons: ProjectRootReasonCounters::default(),
        lifecycle: Vec::new(),
        lifecycle_countries: 0,
        lifecycle_aborted_countries: 0,
        owner_progress: Vec::new(),
        owner_progress_countries: 0,
        owner_progress_aborted_countries: 0,
        reward_history: Vec::new(),
        reward_history_countries: 0,
        reward_history_aborted_countries: 0,
    }
}

#[test]
fn d41_phase_units_are_emitted_on_a_clean_status() {
    let units = compact_projects_diagnostic_units(&read_with_phase(0));
    let names: Vec<&str> = units.iter().map(|unit| unit.name).collect();
    assert_eq!(
        names,
        D41_PHASE_UNITS.to_vec(),
        "a clean d41 frame must carry exactly the phase split and no \
         failure-context word"
    );
    let values: Vec<u64> = units.iter().map(|unit| unit.value).collect();
    assert_eq!(
        values,
        vec![11, 22, 33, 44, 55, 66, 77, 88, 99, 110, 121, 132, 143, 154, 165, 176, 187],
        "phase unit wiring"
    );
}

#[test]
fn d41_row_shortfall_units_report_the_measured_comparison() {
    let mut read =
        read_with_phase_and_shortfall(compact_contract::COMPACT_STATUS_PARTIAL_DATA, 7, 3);
    read.observed_facility_rows = 9;
    read.observed_scientist_rows = 5;
    assert!(read.facilities.is_empty() && read.scientists.is_empty());

    let units = compact_projects_diagnostic_units(&read);
    let value_of = |name: &str| {
        units
            .iter()
            .find(|unit| unit.name == name)
            .unwrap_or_else(|| panic!("{name} must be emitted on a failing d41 frame"))
            .value
    };
    assert_eq!(value_of("d41_facility_row_shortfall"), 7);
    assert_eq!(value_of("d41_scientist_row_shortfall"), 3);
}

#[test]
fn project_row_shortfall_is_the_status_predicate() {
    let cases: [(u32, usize, u32); 8] = [
        (0, 0, 0),
        (9, 9, 0),
        (9, 0, 9),
        (9, 4, 5),
        (9, 12, 0),
        (u32::MAX, 0, u32::MAX),
        (u32::MAX, usize::MAX, 0),
        (9, 1usize << 32, 0),
    ];
    for (observed, materialised, expected) in cases {
        assert_eq!(
            project_row_shortfall(observed, materialised),
            expected,
            "shortfall for {observed} pinned / {materialised} materialised"
        );
        assert_eq!(
            project_row_shortfall(observed, materialised) > 0,
            materialised < observed as usize,
            "predicate for {observed} pinned / {materialised} materialised"
        );
    }
}

#[test]
fn project_shortfall_status_is_derived_from_the_stored_fields() {
    for (facility, scientist) in [(7u32, 0u32), (0, 3), (7, 3)] {
        let mut read = read_with_phase_and_shortfall(0, facility, scientist);
        apply_project_shortfall_status(&mut read);
        assert!(
            compact_contract::compact_status_is_failure(read.status_flags),
            "shortfall {facility}/{scientist} must raise PARTIAL_DATA"
        );
    }
    let mut clean = read_with_phase_and_shortfall(0, 0, 0);
    apply_project_shortfall_status(&mut clean);
    assert_eq!(clean.status_flags, 0);
    let mut other_cause =
        read_with_phase_and_shortfall(compact_contract::COMPACT_STATUS_PARTIAL_DATA, 0, 0);
    apply_project_shortfall_status(&mut other_cause);
    assert_eq!(
        other_cause.status_flags,
        compact_contract::COMPACT_STATUS_PARTIAL_DATA
    );
}

#[test]
fn d41_row_shortfall_units_do_not_ride_a_clean_frame() {
    let units = compact_projects_diagnostic_units(&read_with_phase_and_shortfall(0, 7, 3));
    let names: Vec<&str> = units.iter().map(|unit| unit.name).collect();
    assert_eq!(names, D41_PHASE_UNITS.to_vec());
}

#[test]
fn d41_failure_context_precedes_the_phase_split() {
    let units = compact_projects_diagnostic_units(&read_with_phase(
        compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
    ));
    let names: Vec<&str> = units.iter().map(|unit| unit.name).collect();
    assert!(
        names.len() > D41_PHASE_UNITS.len(),
        "a failing frame must carry failure context as well as the split"
    );
    assert!(
        !names[0].starts_with("d41_phase_"),
        "failure context must fill the per-offender floor before the phase \
         timers; first unit was {:?}",
        names[0]
    );
    assert_eq!(
        &names[names.len() - D41_PHASE_UNITS.len()..],
        &D41_PHASE_UNITS[..],
        "the phase split is appended last"
    );
}
