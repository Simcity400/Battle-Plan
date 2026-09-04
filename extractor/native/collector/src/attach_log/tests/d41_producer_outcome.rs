//! Domain 41's own account of a failing frame.
//!
//! Measured on `captures/e2e_05` (120 s at speed 5, 1512 game hours): d41
//! degraded 65 consecutive hours (60833776-60833937) and every one reached the
//! report as *"domain 41 published a failing PERF status 0x00000200 (offending
//! 0x00000200 = partial_data) and its producer recorded no outcome of its own"*.
//! The trailer under that sentence survived nine units, all of them the
//! breakthrough tree walk's SUCCESS words. The capture's own per-domain totals
//! name the cause the trailer could not fit: `d41_facility_row_shortfall = 86`
//! summed over those 65 frames, with every other d41 failure counter at zero.

use super::super::*;
use super::domain41_phase_split::read_with_phase_and_shortfall;

fn failing_read() -> CompactProjectsRead {
    read_with_phase_and_shortfall(compact_contract::COMPACT_STATUS_PARTIAL_DATA, 0, 0)
}

fn unit_names(read: &CompactProjectsRead) -> Vec<&'static str> {
    compact_projects_diagnostic_units(read)
        .into_iter()
        .map(|unit| unit.name)
        .collect()
}

/// Law: the eight units that decide WHICH refusal fired lead the failure
/// trailer, so a budget-squeezed record cannot lose the diagnosis.
///
/// `allocate_unit_budget` guarantees an offender the first
/// `MAX_FLOOR_UNITS_PER_OFFENDER` units of its list. On `captures/e2e_05` that
/// floor was spent on the breakthrough walk and the one nonzero failure value
/// was budgeted out.
///
/// What production change makes this fail: any unit inserted ahead of the cause
/// words, or a cause whose magnitude leaves the floor.
#[test]
fn d41_cause_words_lead_the_failure_trailer() {
    let names = unit_names(&failing_read());
    assert_eq!(
        &names[..8],
        &[
            "d41_refusal_first_cause_code",
            "d41_refusal_cause_mask",
            "d41_facility_row_shortfall",
            "d41_scientist_row_shortfall",
            "d41_invalid_breakthrough_countries",
            "d41_lifecycle_aborted_countries",
            "d41_owner_progress_aborted_countries",
            "d41_reward_history_aborted_countries",
        ],
        "the per-offender floor must carry the cause and every magnitude it can \
         be read from"
    );
}

/// Law: each of the nine status sites is its own named cause, and the cause is
/// derived from the field the site raised its bit on.
///
/// What production change makes this fail: a site that stops storing its
/// quantity, or a new site with no code.
#[test]
fn every_d41_status_site_has_its_own_cause() {
    let cases: [(u32, fn(&mut CompactProjectsRead)); 9] = [
        (domain41_cause::SOURCE_ABSENT, |read| {
            read.source_kind =
                compact_contract::payload_source_kinds::COMPACT_SOURCE_KIND_ABSENT;
            read.status_flags |= COMPACT_STATUS_SOURCE_UNAVAILABLE;
        }),
        (domain41_cause::CACHE_OVER_CAPACITY, |read| {
            read.cache_too_many = true;
        }),
        (domain41_cause::COUNTRY_ROOTS_REFUSED, |read| {
            read.invalid_project_pool_countries = 3;
        }),
        (domain41_cause::LIFECYCLE_ABORTED, |read| {
            read.lifecycle_aborted_countries = 1;
        }),
        (domain41_cause::OWNER_PROGRESS_ABORTED, |read| {
            read.owner_progress_aborted_countries = 1;
        }),
        (domain41_cause::REWARD_HISTORY_ABORTED, |read| {
            read.reward_history_aborted_countries = 1;
        }),
        (domain41_cause::BREAKTHROUGH_COUNTRIES_REFUSED, |read| {
            read.invalid_breakthrough_countries = 2;
        }),
        (domain41_cause::FACILITY_ROWS_LOST, |read| {
            read.facility_row_shortfall = 2;
        }),
        (domain41_cause::SCIENTIST_ROWS_LOST, |read| {
            read.scientist_row_shortfall = 5;
        }),
    ];
    assert_eq!(cases.len() as u32, domain41_cause::COUNT);
    for (code, apply) in cases {
        let mut read = failing_read();
        apply(&mut read);
        let diag = domain41_diag(&read);
        assert_eq!(
            diag.first_cause,
            code,
            "cause {code} ({}) must be the diagnosis of the site that raises it",
            domain41_cause::name(code)
        );
        assert_eq!(diag.causes, domain41_cause::bit(code));
        let reason = domain41_failure_reason(&read);
        assert!(
            reason.contains(domain41_cause::name(code)),
            "the sentence must carry the branch name: {reason}"
        );
        assert!(
            !reason.contains("recorded no outcome of its own"),
            "the producer's sentence must not be the completeness walk's \
             stand-in: {reason}"
        );
    }
}

/// Law: the diagnosis is the source of truth for `first_cause`, and a failing
/// word no site explains says exactly that rather than naming an innocent one.
#[test]
fn a_d41_failure_no_site_explains_is_said_to_be_unexplained() {
    let read = failing_read();
    assert_eq!(domain41_diag(&read), Domain41Diag::default());
    let reason = domain41_failure_reason(&read);
    assert!(
        reason.contains("its own guards do not explain"),
        "an unattributed word must say so: {reason}"
    );
}

/// Law: a breakthrough walk that SUCCEEDED contributes no word to a failing
/// frame's trailer.
///
/// The capture's counters are the case: `expected_rows == observed_rows == 4`
/// and header nil marker 1 is the walk's clean exit, `invalid_breakthrough_
/// countries = 0` proves no country was rejected, and the frame's `PARTIAL_DATA`
/// came from the facility shortfall. Publishing the walk's residue anyway is
/// what cost all 65 hours their diagnosis.
///
/// What production change makes this fail: ungating the breakthrough block.
#[test]
fn a_succeeding_breakthrough_walk_is_absent_from_the_failure_trailer() {
    let mut read = failing_read();
    read.facility_row_shortfall = 2;
    read.observed_facility_rows = 27;
    let diag = domain41_diag(&read);
    assert!(!diag.fired(domain41_cause::BREAKTHROUGH_COUNTRIES_REFUSED));
    for name in unit_names(&read) {
        assert!(
            !name.starts_with("d41_breakthrough_"),
            "{name} describes the breakthrough walk, which did not refuse this frame"
        );
    }

    read.invalid_breakthrough_countries = 1;
    let names = unit_names(&read);
    assert!(
        names.contains(&"d41_breakthrough_failure_stage"),
        "a rejected breakthrough country must bring its own locators back"
    );
}

/// Law: a measured nil marker with matching expected and observed rows is a
/// MEASUREMENT, not a partial answer — it sets no status bit and raises no
/// cause.
///
/// `read_project_breakthrough_progress_rows` refuses a header whose nil marker
/// is not 1 and refuses a walk whose row count does not equal the container's
/// size, so "nil 1, expected == observed" is the shape of a complete read. The
/// only breakthrough site that raises `PARTIAL_DATA` is
/// `invalid_breakthrough_countries > 0`, and it is zero on all 65 hours.
///
/// What production change makes this fail: a breakthrough status site that
/// fires on the walk's success values.
#[test]
fn a_measured_nil_marker_with_matching_rows_is_not_a_partial_answer() {
    let mut read = read_with_phase_and_shortfall(0, 0, 0);
    read.observed_breakthrough_rows = 4;
    read.invalid_breakthrough_countries = 0;
    apply_project_shortfall_status(&mut read);
    assert_eq!(
        read.status_flags & COMPACT_STATUS_PARTIAL_DATA,
        0,
        "a complete breakthrough tree is a whole answer"
    );
    assert_eq!(domain41_diag(&read), Domain41Diag::default());
    assert!(compact_projects_diagnostic_units(&read)
        .iter()
        .all(|unit| unit.name.starts_with("d41_phase_")
            || unit.name.starts_with("d41_specialization_")
            || unit.name == "d41_lifecycle_bulk_fallback_countries"));
}

/// Law: the first pin the materialisation walk lost keeps the locator, and the
/// count keeps counting.
///
/// `facility_rows` drops a pin whose guarded body read fails and keeps walking,
/// so the shortfall could be counted but no row could be named.
#[test]
fn the_first_lost_pin_keeps_the_locator() {
    let mut loss = Domain41MaterialiseLoss::default();
    assert_eq!(loss.first_lost_ordinal, 0);
    loss.note_lost(4, 7717, 3312);
    loss.note_lost(9, 9001, 4400);
    assert_eq!(loss.lost_rows, 2);
    assert_eq!(loss.first_lost_ordinal, 5, "the ordinal is 1-based");
    assert_eq!(loss.first_lost_id, 7717);
    assert_eq!(loss.first_lost_aux, 3312);
    assert!(!loss.capped);
}

/// Law: the sentence NAMES the lost row and separates a capped walk from a
/// refused one.
#[test]
fn the_facility_loss_clause_names_the_row_and_the_cap() {
    let mut read = failing_read();
    read.facility_row_shortfall = 2;
    read.observed_facility_rows = 27;
    read.facility_loss.note_lost(4, 7717, 3312);
    read.facility_loss.note_lost(9, 9001, 4400);
    let reason = domain41_failure_reason(&read);
    for needle in [
        "2 of 27 pinned facility row(s) did not materialise",
        "2 refused by the body read",
        "first at pin 5 id 7717 province 3312",
    ] {
        assert!(reason.contains(needle), "{needle:?} missing from: {reason}");
    }
    assert!(!reason.contains("stopped on its row cap"));

    read.facility_loss.capped = true;
    assert!(domain41_failure_reason(&read).contains("the walk stopped on its row cap"));

    let mut capped_only = failing_read();
    capped_only.facility_row_shortfall = 3;
    capped_only.observed_facility_rows = 27;
    capped_only.facility_loss.capped = true;
    let reason = domain41_failure_reason(&capped_only);
    assert!(
        reason.contains("none refused by identity") && reason.contains("row cap"),
        "a walk that lost rows only to its cap must not claim a refused pin: {reason}"
    );
}

/// Law: the emit site's helper records a FAILURE carrying the producer's own
/// sentence, so the completeness walk never reaches for its stand-in.
///
/// What production change makes this fail: dropping the
/// `record_reader_status_outcome` call from the d41 emit block.
#[test]
fn d41_records_an_outcome_of_its_own() {
    let mut read = failing_read();
    read.facility_row_shortfall = 2;
    read.observed_facility_rows = 27;
    read.facility_loss.note_lost(4, 7717, 3312);
    read.facility_loss.note_lost(9, 9001, 4400);

    let mut outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
    let units = compact_projects_diagnostic_units(&read);
    let published = record_reader_status_outcome(
        &mut outcomes,
        COMPACT_DOMAIN_PROJECTS,
        0,
        read.status_flags,
        || domain41_failure_reason(&read),
        deciding_units_from_perf(&units),
    );
    assert_eq!(
        published, read.status_flags,
        "the PERF record publishes the word the outcome describes"
    );
    let outcome = compact_contract::domain_outcome::failure_diagnosis(
        &outcomes,
        COMPACT_DOMAIN_PROJECTS,
    )
    .expect("d41 must now record an outcome of its own");
    assert_eq!(
        outcome.predicate.code.get(),
        compact_contract::domain_outcome::READ_PREDICATE_READER_RETURNED_ROWS
    );
    assert!(outcome
        .reason
        .contains(domain41_cause::name(domain41_cause::FACILITY_ROWS_LOST)));
    assert!(outcome.reason.contains("first at pin 5 id 7717"));
    assert!(!outcome.reason.contains("recorded no outcome of its own"));
}

/// Law: the d41 emit block substantiates its word and publishes the word the
/// outcome describes.
///
/// A source assertion because the emit block is inside
/// `build_compact_extraction_gate_records`, which no offline fixture can drive.
#[test]
fn the_d41_emit_block_substantiates_its_status_word() {
    const SOURCE: &str = include_str!("../../attach_log.rs");
    let block = SOURCE
        .split("let diagnostics = compact_projects_diagnostic_units(&read);")
        .nth(1)
        .expect("the d41 emit block builds its diagnostic units")
        .split("sequence = sequence.saturating_add(1);")
        .next()
        .expect("the d41 emit block ends at its first sequence bump");
    for needle in [
        "let d41_published_status = record_reader_status_outcome(",
        "|| domain41_failure_reason(&read),",
        "status_flags: d41_published_status,",
    ] {
        assert!(
            block.contains(needle),
            "the d41 emit block no longer contains {needle:?}; a failing d41 word is \
             back to the completeness walk's stand-in. Block was:\n{block}"
        );
    }
}
