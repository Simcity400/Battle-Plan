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
    read_with_phase_and_shortfall(status_flags, 0, 0)
}

/// The same read with the two cache-shortfall measurements set.
///
/// They are a parameter rather than a constant because the property
/// `d41_row_shortfall_units_report_the_measured_comparison` needs a read whose
/// shortfall fields DISAGREE with anything derivable from its vectors -- which
/// is the post-emit-mode shape the builder actually sees.
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

/// The two cache-shortfall units report the comparison the STATUS BIT was set
/// on, and not a re-derivation from the read's vectors (lane IJ).
///
/// WHY THIS IS THE PROPERTY. `read_compact_projects` sets
/// `COMPACT_STATUS_PARTIAL_DATA` when the materialised facility/scientist rows
/// are short of the pins `special_projects` holds. That was the one
/// PARTIAL_DATA site in the whole function with no published witness, and on
/// t51_pricing it was the only one that could have fired: d41 degraded 51 of
/// 600 hours during which not one of its failure counters was nonzero, so the
/// capture could not name its own cause. The units close that.
///
/// The trap they have to survive is `compact_projects_apply_emit_mode`, which
/// replaces `facilities` and `scientists` with a DELTA PATCH before
/// `compact_projects_diagnostic_units` runs. This fixture is that shape
/// exactly: empty vectors, nonzero `observed_*_rows`, and shortfalls that are
/// neither.
///
/// MUTATION: build the units at the emission site as
/// `observed_facility_rows - facilities.len()` (the obvious spelling, and the
/// one this lane wrote first) and the assertion fails reporting 9 and 5 -- the
/// whole pin count -- instead of 7 and 3. On the ordinary delta hour, where the
/// patch is empty and nothing was lost, that spelling would report every pinned
/// row as missing on every frame.
#[test]
fn d41_row_shortfall_units_report_the_measured_comparison() {
    let mut read = read_with_phase_and_shortfall(
        compact_contract::COMPACT_STATUS_PARTIAL_DATA,
        7,
        3,
    );
    // The post-emit-mode shape: the vectors are a patch (here, empty) while the
    // observed counts still describe the pins. A re-derivation would read 9/5.
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

/// THE PRODUCTION COMPUTATION ITSELF: `project_row_shortfall` is d41's
/// facility/scientist PARTIAL_DATA condition, and `> 0` over its result is the
/// predicate `read_compact_projects` sets the bit on.
///
/// WHY THIS TEST EXISTS AND THE TWO ABOVE ARE NOT ENOUGH (Codex review
/// 019ffd27-0ddd-7c83-84f0-891b9b8f86a7, finding 1). Those two hand-inject the
/// shortfall into the read struct, so they pin the EMISSION site and nothing
/// else: a production site that computed two zeros while keeping the old
/// `facilities.len() < observed_facility_rows` predicate would pass both, and
/// pass the whole collector battery, while publishing a unit that says nothing
/// was lost on the very hour the bit is set. There is no offline fixture that
/// can drive `read_compact_projects` -- the special-projects source is not
/// stageable, per this file's module note -- so the computation is extracted
/// and tested where it can be reached.
///
/// TWO PROPERTIES, and the second is the one that made the extraction worth
/// doing:
///   * the value is the SHORT-DIRECTION difference, saturating, so a side that
///     materialised more than it pinned reports 0 rather than underflowing;
///   * `shortfall > 0` is exactly `materialised < observed`, which is the
///     predicate the old inline spelling used -- INCLUDING above `u32::MAX`,
///     where the old `usize` comparison and the new clamped `u32` one are the
///     only place they could have diverged.
///
/// MUTATION: return 0 unconditionally (the "replace the calculation" mutation)
/// and the first block fails at (9, 0). Drop the `saturating_` and the
/// more-than-pinned row panics on underflow in debug. Drop the `.min` clamp and
/// the `(9, 1 << 32)` case fails: the truncating cast turns 2^32 into 0 and
/// manufactures a 9-row shortfall out of a side that lost nothing.
///
/// THAT LAST CASE IS 2^32 AND NOT `usize::MAX` ON PURPOSE (Codex review
/// 019ffd3b, finding 1). `usize::MAX as u32` is `u32::MAX`, so the `usize::MAX`
/// row below is satisfied by the clamped and the truncating spelling alike and
/// does NOT discriminate. It is kept because it pins the saturating floor at
/// the top of the range; `1 << 32` is the row that kills the mutation.
///
/// WHAT THIS TEST COVERS AND WHAT COVERS THE REST. This one is the COMPUTATION
/// and the predicate over it. The HANDOFF - "could a production edit store
/// zeros in the two fields while the bit stays set?" - is not tested here and
/// does not need to be: `apply_project_shortfall_status` derives the bit FROM
/// those stored fields, so zeroing them clears it, and
/// `project_shortfall_status_is_derived_from_the_stored_fields` pins that.
/// Between the two, every link from the comparison to the published unit is
/// covered except the plain field initialiser, which feeds both and therefore
/// cannot make them disagree.
#[test]
fn project_row_shortfall_is_the_status_predicate() {
    // (observed pins, materialised rows, expected shortfall)
    let cases: [(u32, usize, u32); 8] = [
        (0, 0, 0),
        (9, 9, 0),
        (9, 0, 9),
        (9, 4, 5),
        (9, 12, 0),
        (u32::MAX, 0, u32::MAX),
        (u32::MAX, usize::MAX, 0),
        // The clamp's discriminating case: 2^32 truncates to 0.
        (9, 1usize << 32, 0),
    ];
    for (observed, materialised, expected) in cases {
        assert_eq!(
            project_row_shortfall(observed, materialised),
            expected,
            "shortfall for {observed} pinned / {materialised} materialised"
        );
        // The predicate equivalence, on every case: the bit is set exactly when
        // the cache came up short.
        assert_eq!(
            project_row_shortfall(observed, materialised) > 0,
            materialised < observed as usize,
            "predicate for {observed} pinned / {materialised} materialised"
        );
    }
}

/// The status bit for THIS cause is derived from the STORED fields, so the
/// published numbers and the status word cannot disagree.
///
/// WHY THIS CLOSES THE HANDOFF (Codex review 019ffd4f, finding 1). An earlier
/// spelling computed the shortfalls into locals, set the bit from the LOCALS,
/// and separately copied the locals into the struct - so an edit that stored
/// zeros in the two fields while leaving the status calculation alone would have
/// published "nothing was lost" on an hour whose bit was set, untestably,
/// because no fixture can drive `read_compact_projects`. Reading the bit off the
/// struct closes it: zeroing the fields now clears the bit with them.
///
/// ONE DIRECTION. `shortfall == 0` with `PARTIAL_DATA` already set is LEGAL and
/// asserted below - five other sites in `read_compact_projects` raise the same
/// bit for their own reasons, and this derivation must not claim otherwise.
///
/// MUTATION: change the derivation to set the bit unconditionally and the
/// "clean stays clean" case fails; change it to read anything other than the two
/// stored fields (a local, a constant) and the first case fails.
#[test]
fn project_shortfall_status_is_derived_from_the_stored_fields() {
    // Short on either side raises the bit.
    for (facility, scientist) in [(7u32, 0u32), (0, 3), (7, 3)] {
        let mut read = read_with_phase_and_shortfall(0, facility, scientist);
        apply_project_shortfall_status(&mut read);
        assert!(
            compact_contract::compact_status_is_failure(read.status_flags),
            "shortfall {facility}/{scientist} must raise PARTIAL_DATA"
        );
    }
    // No shortfall leaves a clean word clean: this derivation is not a
    // second, blanket source of the bit.
    let mut clean = read_with_phase_and_shortfall(0, 0, 0);
    apply_project_shortfall_status(&mut clean);
    assert_eq!(clean.status_flags, 0);
    // ... and does not clear a bit some OTHER site already raised.
    let mut other_cause =
        read_with_phase_and_shortfall(compact_contract::COMPACT_STATUS_PARTIAL_DATA, 0, 0);
    apply_project_shortfall_status(&mut other_cause);
    assert_eq!(other_cause.status_flags, compact_contract::COMPACT_STATUS_PARTIAL_DATA);
}

/// A clean frame carries no shortfall word, for the same reason it carries no
/// other failure context: there is nothing to attribute.
///
/// MUTATION: append either unit outside the
/// `if compact_status_is_failure(..)` block -- where the phase timers correctly
/// live -- and this fails, because the clean list must be exactly the six phase
/// units.
#[test]
fn d41_row_shortfall_units_do_not_ride_a_clean_frame() {
    let units = compact_projects_diagnostic_units(&read_with_phase_and_shortfall(0, 7, 3));
    let names: Vec<&str> = units.iter().map(|unit| unit.name).collect();
    assert_eq!(names, D41_PHASE_UNITS.to_vec());
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
