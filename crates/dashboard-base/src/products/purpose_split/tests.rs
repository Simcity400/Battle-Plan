use super::*;
use serde_json::json;

fn run(first_hour: i32, last_hour: i32, value: i128, break_before: bool) -> Run {
    Run::new(first_hour, last_hour, value, break_before)
}

fn design(without_refits: Vec<Run>, combined: Vec<Run>) -> Design {
    Design {
        unresolved: false,
        without_refits,
        combined,
        separate: [Vec::new(), Vec::new()],
        unproven: Vec::new(),
    }
}

/// New, refit and converted amounts, in that order, with nothing unrecorded.
fn amounts(new: i128, refit: i128, converted: i128) -> Amounts {
    let mut amounts = [0; Purpose::ALL.len()];
    amounts[Purpose::New.index()] = new;
    amounts[Purpose::Refit.index()] = refit;
    amounts[Purpose::Converted.index()] = converted;
    amounts
}

const NONE: Amounts = [0; Purpose::ALL.len()];

#[test]
fn a_month_counts_new_and_refit_input_from_its_baseline_and_refuses_what_cannot_be_told() {
    use ProductionSpendingUnknown::{CountryChanged, SpendingUnavailable};
    // From the observation's start: 10 new, then 4 refit and 6 more new.
    let ship = design(
        vec![run(0, 99, 10, true), run(100, 299, 16, false)],
        vec![
            run(0, 49, 10, true),
            run(50, 99, 14, false),
            run(100, 299, 20, false),
        ],
    );
    // The first month starts at the observation's zero.
    assert_eq!(design_change(&ship, 0, 99), Ok(amounts(10, 4, 0)));
    assert_eq!(design_change(&ship, 100, 299), Ok(amounts(6, 0, 0)));
    assert_eq!(design_change(&ship, 40, 120), Ok(amounts(6, 4, 0)));
    // A refit-only design has no series without refits at all. Its first
    // input, after the observation began, opens its series with a break, as
    // every series' first run does: it counts from zero.
    let refitted = design(vec![], vec![run(150, 299, 7, true)]);
    assert_eq!(design_change(&refitted, 100, 199), Ok(amounts(0, 7, 0)));
    assert_eq!(design_change(&refitted, 200, 299), Ok(NONE));
    // Before a design's first input nothing is spent on it.
    assert_eq!(design_change(&refitted, 0, 99), Ok(NONE));
    // A series that ended is not known after it, nor across a gap.
    let ended = design(vec![], vec![run(0, 99, 5, true), run(150, 299, 9, false)]);
    assert_eq!(
        design_change(&ended, 100, 120),
        Err(SpendingUnavailable)
    );
    assert_eq!(
        design_change(&ended, 50, 200),
        Err(SpendingUnavailable)
    );
    // Its first value after the gap holds input from the gap too.
    assert_eq!(
        design_change(&ended, 150, 200),
        Err(SpendingUnavailable)
    );
    assert_eq!(design_change(&ended, 151, 200), Ok(NONE));
    // A later segment is another holder's.
    let replaced = design(vec![], vec![run(0, 99, 5, true), run(100, 299, 2, true)]);
    assert_eq!(
        design_change(&replaced, 50, 150),
        Err(CountryChanged)
    );
    assert_eq!(design_change(&replaced, 150, 200), Ok(NONE));
    // Signed corrections stay exact.
    let corrected = design(
        vec![run(0, 99, 8, true), run(100, 199, 5, false)],
        vec![run(0, 99, 8, true), run(100, 199, 5, false)],
    );
    assert_eq!(
        design_change(&corrected, 100, 199),
        Ok(amounts(-3, 0, 0))
    );
}

#[test]
fn converted_input_is_split_from_new_production_against_the_same_baseline() {
    // 10 new, then the line converts 6 in hours 100..=199 and goes back to
    // new production for 3 more; a -1 correction of conversions follows.
    let mut plane = design(
        vec![
            run(0, 99, 10, true),
            run(100, 199, 16, false),
            run(200, 299, 19, false),
            run(300, 399, 18, false),
        ],
        vec![
            run(0, 99, 10, true),
            run(100, 199, 16, false),
            run(200, 299, 19, false),
            run(300, 399, 18, false),
        ],
    );
    plane.separate[0] = vec![run(100, 299, 6, false), run(300, 399, 5, false)];
    assert_eq!(design_change(&plane, 0, 99), Ok(amounts(10, 0, 0)));
    // A mode change inside one month splits it exactly.
    assert_eq!(design_change(&plane, 50, 250), Ok(amounts(3, 0, 6)));
    assert_eq!(design_change(&plane, 200, 299), Ok(amounts(3, 0, 0)));
    // A signed correction of conversions stays in conversions.
    assert_eq!(design_change(&plane, 300, 399), Ok(amounts(0, 0, -1)));
    assert_eq!(design_change(&plane, 0, 399), Ok(amounts(13, 0, 5)));
    // Converted input before the series began is none.
    assert_eq!(design_change(&plane, 0, 50), Ok(amounts(10, 0, 0)));
}

/// The endpoint holding a month's last hour: the source's latest read and
/// restart then.
fn endpoint(read: Option<i32>, restart: Option<i32>) -> facts::Endpoint {
    facts::Endpoint {
        first_hour: 0,
        last_hour: 0,
        equipment_read: read,
        rocket_read: None,
        equipment_origin: read.map(|_| 100),
        equipment_restart: restart,
        rocket_origin: None,
        rocket_restart: None,
        names_hour: None,
        ranking_offset: 0,
        ranking_len: 0,
        keyframe: true,
        continues: false,
    }
}

#[test]
fn a_month_is_known_only_where_the_equipment_source_was_read_and_untainted() {
    use ProductionSpendingUnknown::{
        CountryChanged, NotRecorded, ObservationRestarted, RecordingGap,
    };
    // `country` is the country's own refusal; None where it refuses nothing
    // or the endpoint does not list it.
    let month = |endpoint: Option<facts::Endpoint>, country, tainted, last| {
        SourceCoverage::at(endpoint.as_ref(), country, tainted).month(last)
    };
    // Before any read nothing is known, not zero: no series is no proof.
    assert_eq!(month(None, None, None, 99), Err(NotRecorded));
    assert_eq!(
        month(Some(endpoint(None, None)), None, None, 99),
        Err(NotRecorded)
    );
    // The month the recording starts in counts from its first read.
    assert_eq!(
        month(Some(endpoint(Some(100), None)), None, None, 100),
        Ok(())
    );
    // A country the endpoint does not list refuses nothing of its own.
    assert_eq!(
        month(Some(endpoint(Some(148), None)), None, None, 10_000),
        Ok(())
    );
    let gap = Some((500, RecordingGap));
    assert_eq!(
        month(Some(endpoint(Some(148), None)), None, gap, 499),
        Ok(())
    );
    assert_eq!(
        month(Some(endpoint(Some(148), None)), None, gap, 500),
        Err(RecordingGap)
    );
    assert_eq!(
        month(Some(endpoint(Some(948), Some(900))), None, None, 950),
        Err(ObservationRestarted)
    );
    // Its holder changed: the country's own refusal, as production spending's.
    assert_eq!(
        month(Some(endpoint(Some(148), None)), Some(CountryChanged), None, 150),
        Err(CountryChanged)
    );
}

#[test]
fn a_month_is_purpose_uncertain_only_where_unproven_input_joined_it() {
    // Unrecorded-purpose input in the first month only; later months add
    // recorded input of the same design.
    let mut ship = design(
        vec![run(0, 99, 10, true), run(100, 299, 16, false)],
        vec![run(0, 99, 10, true), run(100, 299, 16, false)],
    );
    ship.unproven = vec![run(0, 299, 4, true)];
    assert_eq!(unproven_in(&ship, 0, 99), Ok(true));
    assert_eq!(unproven_in(&ship, 100, 299), Ok(false));
    // Evidence that begins later is read from its own baseline.
    let mut refitted = design(vec![], vec![run(0, 99, 5, true), run(100, 299, 9, false)]);
    refitted.unproven = vec![run(150, 299, 3, false)];
    assert_eq!(unproven_in(&refitted, 0, 99), Ok(false));
    assert_eq!(unproven_in(&refitted, 100, 199), Ok(true));
    assert_eq!(unproven_in(&refitted, 200, 299), Ok(false));
    // Summed magnitudes: +1 and -1 of unproven input on two lines grow the
    // evidence by 2, so a month whose net input is zero is still uncertain.
    let mut cancelled = design(
        vec![run(0, 99, 10, true), run(100, 299, 10, false)],
        vec![run(0, 99, 10, true), run(100, 299, 10, false)],
    );
    cancelled.unproven = vec![run(0, 99, 0, true), run(100, 299, 2, false)];
    assert_eq!(design_change(&cancelled, 100, 299), Ok(NONE));
    assert_eq!(unproven_in(&cancelled, 100, 299), Ok(true));
}

/// Input whose purpose was not recorded is its own amount, never part of new
/// production: 5 unrecorded and 2 new IC are 2 new and 5 unrecorded.
#[test]
fn unrecorded_purpose_input_is_kept_out_of_new_production() {
    let mut design = design(
        vec![run(0, 99, 7, true), run(100, 199, 9, false)],
        vec![run(0, 99, 7, true), run(100, 199, 9, false)],
    );
    design.separate[1] = vec![run(0, 199, 5, true)];
    design.unproven = vec![run(0, 199, 5, true)];
    let mut expected = amounts(2, 0, 0);
    expected[Purpose::Unknown.index()] = 5;
    assert_eq!(design_change(&design, 0, 99), Ok(expected));
    // Later, recorded new input only.
    assert_eq!(design_change(&design, 100, 199), Ok(amounts(2, 0, 0)));
}

/// Input of a purpose the domain never has stays in the month's whole as
/// its own remainder: 1 naval new and 2 naval converted IC are 1 new ship
/// and 2 other, not 1 IC in all.
#[test]
fn a_purpose_the_domain_never_has_is_a_remainder_of_the_month() {
    let shown = [Purpose::New, Purpose::Refit];
    let month = PurposeMonth {
        first_hour: 0,
        last_hour: 99,
        amounts: Ok(amounts(1, 0, 2)),
        purpose_unproven: false,
        unresolved: false,
    };
    assert_eq!(month.part(Purpose::New), Ok(1));
    assert_eq!(month.other(&shown), Ok(2));
    // Unrecorded-purpose input is never another purpose's remainder.
    let mut unrecorded = amounts(1, 0, 0);
    unrecorded[Purpose::Unknown.index()] = 4;
    let month = PurposeMonth { amounts: Ok(unrecorded), ..month };
    assert_eq!(month.other(&shown), Ok(0));
    assert_eq!(month.part(Purpose::Unknown), Ok(4));
    let unknown = PurposeMonth { amounts: Err(ProductionSpendingUnknown::RecordingGap), ..month };
    assert_eq!(unknown.other(&shown), Err(ProductionSpendingUnknown::RecordingGap));
}

#[test]
fn domain_leaves_keep_every_purpose_of_their_own_designs() {
    let leaf = |domain: Option<Domain>, type_key: &str, purpose: &str, rocket: bool| {
        json!({ "domain": domain, "typeKey": type_key, "purpose": purpose, "rocket": rocket })
    };
    let mut data = json!({ "leaves": [
        leaf(Some(Domain::Air), "fighter", "new", false),
        leaf(Some(Domain::Air), "fighter", "converted", false),
        leaf(Some(Domain::Air), "fighter", "unknown", false),
        // Air input of a purpose the view does not show stays in the whole
        // and is counted for the warnings.
        leaf(Some(Domain::Air), "fighter", "refit", false),
        leaf(Some(Domain::Land), "tank", "new", false),
        leaf(Some(Domain::Naval), "destroyer", "refit", false),
        // Content cannot type it: it could be air.
        leaf(None, ":plane_token", "converted", false),
        // Content cannot type it, and air counts every purpose of its own
        // designs: kept, though a refit, so the whole is not left exact.
        leaf(None, ":ship_token", "refit", false),
        leaf(None, ":any_token", "unknown", false),
        leaf(Some(Domain::Air), "fighter", "new", true),
    ] });
    let shown = [Purpose::New, Purpose::Converted];
    let other = keep_domain(&mut data, Domain::Air, &shown).unwrap();
    assert_eq!(other, 1);
    let kept: Vec<(&str, &str)> = data["leaves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|leaf| (leaf["typeKey"].as_str().unwrap(), leaf["purpose"].as_str().unwrap()))
        .collect();
    assert_eq!(
        kept,
        [
            ("fighter", "new"),
            ("fighter", "converted"),
            ("fighter", "unknown"),
            ("fighter", "refit"),
            (":plane_token", "converted"),
            (":ship_token", "refit"),
            (":any_token", "unknown"),
        ]
    );
}
