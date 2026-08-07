//! `domain41_phase_split`: D41's per-phase wall split must ride a CLEAN record
//! (lane DS, 2026-08-07).
//!
//! WHY THE SPLIT EXISTS. t32's `read_time_us` for d41 has a median of 23.9 ms,
//! the third-widest non-critical read in the capture, and its PERF trailer
//! carried nothing but failure-context words: a stage code, a refusing tag, a
//! count of aborted countries. No aggregate in the tree could say which part of
//! that 23.9 ms was engine OBSERVATION (which the snapshot law bounds but
//! cannot delete) and which was COMPUTE over bytes the collector already owned
//! (which may be moved out of the bracket or deleted). `Domain41PhaseTimes`
//! splits the read into its five calls and publishes them as `d41_phase_*_us`.
//!
//! WHY THE GATE MATTERS. Every OTHER `d41_*` unit is failure-context and is
//! correctly built inside `if compact_status_is_failure(..)` — those words mean
//! nothing without a failure to attribute. A per-phase wall is the opposite kind
//! of unit: the frames a perf lane needs the breakdown for are the ones that
//! WORKED, so a split that only appears on failing hours does not exist for its
//! purpose. Nothing in the source stops the next `d41_*` unit being appended
//! inside that block, which is where these six would have gone by default.
//!
//! WHY THIS TESTS THE BUILDER AND NOT A DRIVEN FRAME. There is no offline
//! fixture that drives d41 to a clean frame — the special-projects source is not
//! stageable, so the read always takes its `source_kind == 0` early return and
//! reports `SOURCE_UNAVAILABLE`. On a failing frame the conditional block runs
//! too, so ANY assertion over the emitted list is satisfied by a conditional
//! build as well as an unconditional one. An earlier version of this file
//! asserted the phase units were the list's PREFIX and called that a proxy for
//! unconditionality; Codex review 019fd9aa-4180-7ac1-888e-2d3ed3d6cac7 (finding
//! 3) showed the proxy is unsound in both directions — a build that prepends
//! them INSIDE the gate passes it, and a correct build that appends them outside
//! the gate fails it. Calling `compact_projects_diagnostic_units` with a clean
//! `status_flags` tests the property itself, with no proxy in between.
//!
//! ORDER IS ALSO ASSERTED, and for a separate reason: the channel's budget
//! allocator fills each domain's guaranteed floor (`MAX_FLOOR_UNITS_PER_OFFENDER`
//! = 8) in emission order for role-less units, so phase timers ahead of failure
//! context would evict the reason a frame failed from a truncated record. The
//! failure words must come first.

use super::*;

/// The six units, in the order the builder appends them.
const D41_PHASE_UNITS: [&str; 6] = [
    "d41_phase_d18_reread_us",
    "d41_phase_lifecycle_us",
    "d41_phase_cache_rows_us",
    "d41_phase_breakthrough_us",
    "d41_phase_emit_mode_us",
    "d41_phase_total_us",
];

/// A read carrying distinguishable phase values, so a unit wired to the wrong
/// field is caught by value and not only by name.
fn read_with_phase(status_flags: u32) -> CompactProjectsRead {
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

/// THE property: on a CLEAN status the six phase units are still emitted, and
/// they are the whole list — no failure-context word rides a frame that did not
/// fail.
///
/// MUTATION: move the `units.extend([..phase..])` call in
/// `compact_projects_diagnostic_units` inside the
/// `if compact_status_is_failure(..)` block and this fails on the first
/// assertion, with an empty list. That is the exact regression the earlier
/// prefix assertion could not see.
///
/// MUTATION RUN: recorded in the lane report.
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
    // By VALUE too, so a unit wired to the wrong `Domain41PhaseTimes` field is
    // caught. `read_with_phase` gives every field a distinct number.
    let values: Vec<u64> = units.iter().map(|unit| unit.value).collect();
    assert_eq!(values, vec![11, 22, 33, 44, 55, 66], "phase unit wiring");
}

/// The failure-context words must come FIRST, so the eleven-then-six ordering
/// cannot evict them from the per-offender floor on a truncated record.
///
/// MUTATION: swap the two `extend` calls in
/// `compact_projects_diagnostic_units` so the phase units are appended before
/// the failure block, and this fails naming a `d41_phase_*_us` unit as first.
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
