//! The frame's label, checked against every bracket rather than against d47's
//! prewarm alone.
//!
//! A clock bracket proves the hour did not CHANGE across a read. It does not
//! prove the read happened in the hour the frame publishes under, and those are
//! different claims: a sweep that begins after its labelled hour has already
//! elapsed reads H+1 from end to end, so `opening == closing` and every tear
//! check passes while the frame is still labelled H.
//!
//! Lane AB closed that for d47's prewarm. These tests cover the generalisation
//! to every bracketed domain, the typed outcome it mints, and the guard that
//! keeps it away from a label that was never an observation.

use std::collections::{BTreeMap, BTreeSet};

use compact_contract::domain_gap::{DomainGapPart, DomainGapRecordV1, DomainGapReason};

use super::*;

/// A real calendar hour. `hour_index_from_raw` rejects small integers, and the
/// whole point of `frame_label_is_a_real_hour` is that it distinguishes those,
/// so a test hour has to be one a live capture could carry. This one is from
/// t17.
const LABEL_HOUR: i32 = 60_832_037;
const DOMAIN_ID: u16 = COMPACT_DOMAIN_DIVISIONS;

fn bracket(tag: u16, opening: i32, closing: i32) -> DomainClockBracket {
    DomainClockBracket {
        tag,
        opening: Some((opening, 1)),
        closing: Some((closing, 2)),
        worker_panicked: false,
    }
}

/// Every way a bracket can land, against one label.
///
/// The two arms that matter are the last two: they are the SAME bracket shape
/// -- opening equal to closing, nothing torn -- separated only by whether that
/// shared hour is the frame's. Before this rule existed both were
/// `InFrameHour`, because nothing compared them to the label at all.
///
/// MUTATION RUN: delete the `opening.0 != game_hour_raw` clause from
/// `bracket_hour_verdict`, so it returns `InFrameHour` for both.
/// OBSERVED: `a bracket that ran wholly in another hour is a label mismatch`
/// left `InFrameHour`, right `LabelMismatch`. 3 of the 6 tests in this file
/// fail, plus `the_frame_probe_reports_how_many_brackets_missed_the_frames_label`
/// in `state_fixture` (left `Some(0)`, right `Some(3)`).
#[test]
fn a_bracket_is_graded_against_the_frames_label_not_only_against_itself() {
    assert_eq!(
        bracket_hour_verdict(&bracket(DOMAIN_ID, LABEL_HOUR, LABEL_HOUR), LABEL_HOUR),
        BracketHourVerdict::InFrameHour,
        "a bracket that opened and closed in the frame's own hour publishes"
    );
    assert_eq!(
        bracket_hour_verdict(&bracket(DOMAIN_ID, LABEL_HOUR, LABEL_HOUR + 1), LABEL_HOUR),
        BracketHourVerdict::Straddled,
        "the hour changed mid-read, which the tear check already saw"
    );
    assert_eq!(
        bracket_hour_verdict(
            &bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1),
            LABEL_HOUR
        ),
        BracketHourVerdict::LabelMismatch,
        "a bracket that ran wholly in another hour is a label mismatch"
    );
    assert_eq!(
        bracket_hour_verdict(
            &DomainClockBracket {
                tag: DOMAIN_ID,
                opening: Some((LABEL_HOUR, 1)),
                closing: None,
                worker_panicked: false,
            },
            LABEL_HOUR
        ),
        BracketHourVerdict::ClockUnreadable,
        "half a bracket proves nothing; 'could not observe' is not 'observed \
         the right hour'"
    );
    assert_eq!(
        bracket_hour_verdict(
            &DomainClockBracket {
                worker_panicked: true,
                ..bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1)
            },
            LABEL_HOUR
        ),
        BracketHourVerdict::WorkerPanic,
        "the panic outranks the hour: nothing this bracket says is trustworthy"
    );
}

/// THE REFUSAL, on the wire, through the production replacement function.
///
/// Not a check on a `BTreeMap`: the DOMAIN_DATA record is really deleted from
/// the byte buffer, the DOMAIN_GAP that replaces it is really framed and really
/// decoded back through `DomainGapRecordV1::decode`, which runs the payload
/// grammar. A refusal that could not encode would fail here rather than in a
/// capture.
///
/// MUTATION RUN: in `rejected_bracket_domains`, change the
/// `BracketHourVerdict::LabelMismatch` arm to `None` unconditionally (i.e.
/// delete the refusal, keeping the verdict).
/// OBSERVED: stops one assertion earlier than the DATA check, on the rejected
/// set itself — left `{}`, right `{20}`. Nothing is refused, so the frame goes
/// on to publish the other hour's rows with no gap and no PARTIAL flag, which
/// is the defect exactly. `the_policy_switch_changes_the_refusal_and_not_the_measurement`
/// fails with it (left `None`, right `Some(HourLabelMismatch)`); the other four
/// tests in this file stay green, because the VERDICT is untouched.
#[test]
fn a_wrong_hour_bracket_replaces_its_domains_data_with_a_typed_gap() {
    let mut records = Vec::new();
    append_compact_record(
        &mut records,
        1,
        LABEL_HOUR,
        DOMAIN_ID,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &[7u8; 8],
    );
    let mut ordinals = BTreeMap::new();

    let (_next, rejected) = replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        LABEL_HOUR,
        // Nothing tore. Both reads agree, and both disagree with the label.
        &[bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1)],
        &mut ordinals,
        HourLabelMismatchPolicy::Refuse,
    )
    .expect("a wrong-hour refusal must be encodable");

    assert_eq!(rejected, BTreeSet::from([DOMAIN_ID]));
    let header = compact_contract::parse_compact_record_header(&records).unwrap();
    assert_eq!(
        header.record_type,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
        "the wrong-hour domain's DATA record must not survive"
    );
    let gap = DomainGapRecordV1::decode(
        &records[compact_contract::COMPACT_RECORD_HEADER_LEN..header.record_len()],
    )
    .expect("the replacement must satisfy the H4DG grammar");
    assert_eq!(gap.reason, DomainGapReason::HourLabelMismatch);
    assert_eq!(
        gap.target_hour_raw, LABEL_HOUR,
        "the lost hour is the one the frame was publishing under"
    );
    assert_eq!(
        (gap.tick_before, gap.tick_after),
        (
            u64::try_from(LABEL_HOUR + 1).unwrap(),
            u64::try_from(LABEL_HOUR + 1).unwrap()
        ),
        "the witnesses are the hour the reader ACTUALLY observed, unmodified; \
         nothing here is fabricated"
    );

    // CONTROL: the identical call whose only difference is that the bracket
    // observed the label keeps the DATA record. Without this the assertion
    // above would also pass if every domain were being refused.
    let mut clean = Vec::new();
    append_compact_record(
        &mut clean,
        1,
        LABEL_HOUR,
        DOMAIN_ID,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &[7u8; 8],
    );
    let (_next, rejected) = replace_rejected_domain_records_with_gaps(
        &mut clean,
        1,
        LABEL_HOUR,
        &[bracket(DOMAIN_ID, LABEL_HOUR, LABEL_HOUR)],
        &mut BTreeMap::new(),
        HourLabelMismatchPolicy::Refuse,
    )
    .expect("an in-hour frame replaces nothing");
    assert!(rejected.is_empty());
    assert_eq!(
        compact_contract::parse_compact_record_header(&clean)
            .unwrap()
            .record_type,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA
    );
}

/// The switch decides refusal and NOTHING else. Same brackets, same label, same
/// buffer: only the policy differs, and only the refusal changes.
///
/// This is what makes `CountOnly` safe to ship. If the switch also moved the
/// verdict, a capture running with it off would be measuring something other
/// than what turning it on would cost.
///
/// MUTATION RUN: make `hour_label_mismatched_bracket_count` consult
/// `HOUR_LABEL_MISMATCH_POLICY` and return `(0, ..)` under `CountOnly`.
/// OBSERVED: `the instrument counts the mismatch under BOTH policies` — left
/// `(0, 2)`, right `(1, 2)`. A capture would then report nothing to decide the
/// flip with.
#[test]
fn the_policy_switch_changes_the_refusal_and_not_the_measurement() {
    let brackets = [
        bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1),
        bracket(COMPACT_DOMAIN_LAND_COMBATS, LABEL_HOUR, LABEL_HOUR),
    ];

    assert_eq!(
        hour_label_mismatched_bracket_count(&brackets, LABEL_HOUR, &BTreeSet::new()),
        (1, 2),
        "the instrument counts the mismatch under BOTH policies, over the \
         DOMAINS each bracket covered"
    );
    assert!(
        rejected_bracket_domains(&brackets, LABEL_HOUR, HourLabelMismatchPolicy::CountOnly)
            .is_empty(),
        "CountOnly must publish the frame unchanged"
    );
    assert_eq!(
        rejected_bracket_domains(&brackets, LABEL_HOUR, HourLabelMismatchPolicy::Refuse)
            .get(&DOMAIN_ID)
            .map(|outcome| outcome.0),
        Some(DomainGapReason::HourLabelMismatch),
        "Refuse must reject exactly the mismatched domain"
    );
}

/// The instrument is priced in DOMAIN-HOURS, which is what the capture loses,
/// and not in brackets, which is not.
///
/// One bracket does not cover one domain. The country-bundle worker reads
/// domains 10..=17 in a single pass under `LW_TAG_COUNTRY_BUNDLE`, so ONE
/// mismatched bracket there costs EIGHT domain-hours -- and the reserved probe
/// domain is never refused at all, so a bracket held for it costs none. A
/// bracket count would have understated the first and overstated the second,
/// and both errors are invisible in a capture: they are just a smaller or
/// larger number with nothing to compare it to.
///
/// MUTATION RUN: revert the instrument to counting brackets --
///   `brackets.iter().filter(|b| bracket_hour_verdict(b, hour) ==
///    BracketHourVerdict::LabelMismatch).count()`, with `brackets.len()` as the
///   denominator.
/// OBSERVED: `one bundle bracket costs eight domain-hours, not one` — left
/// `(2, 3)`, right `(8, 9)`. Both errors show at once: the bundle's eight
/// domain-hours collapse to one, and the probe bracket is priced as a refusal
/// that will never happen. Every other test in this file stays green, because
/// every other bracket in them covers exactly its own domain.
#[test]
fn the_instrument_prices_a_mismatch_in_domain_hours_not_in_brackets() {
    let brackets = [
        // Covers domains 10..=17: eight domain-hours behind one bracket.
        bracket(LW_TAG_COUNTRY_BUNDLE, LABEL_HOUR + 1, LABEL_HOUR + 1),
        // Never refused, so it must never be priced.
        bracket(RC_PROBE_DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1),
        bracket(DOMAIN_ID, LABEL_HOUR, LABEL_HOUR),
    ];
    assert_eq!(
        hour_label_mismatched_bracket_count(&brackets, LABEL_HOUR, &BTreeSet::new()),
        (8, 9),
        "one bundle bracket costs eight domain-hours, not one; and the probe \
         domain costs none"
    );
    assert_eq!(
        rejected_bracket_domains(&brackets, LABEL_HOUR, HourLabelMismatchPolicy::Refuse).len(),
        8,
        "the instrument must equal what Refuse would actually reject"
    );
}

/// A tear still outranks a mismatch, and the policy cannot switch a tear off.
///
/// `Straddled` and `LabelMismatch` are both "this domain-hour is not
/// publishable", and it would be easy to collapse them -- but they are
/// different failures, they are refused by different machinery, and only one of
/// them is behind the switch. A capture that could not tell a torn read from a
/// wrong-hour read could not be acted on.
///
/// MUTATION RUN: in `bracket_hour_verdict`, move the label check ABOVE the
/// `opening.0 != closing.0` tear check.
/// OBSERVED: `a straddle is Torn regardless of policy` — left
/// `Some(HourLabelMismatch)`, right `Some(Torn)`. This test is the only one of
/// the six that fails: every torn bracket in a late frame would be relabelled,
/// and switching the policy off would then stop refusing genuine tears.
#[test]
fn a_straddle_is_torn_under_either_policy_even_when_it_missed_the_label_too() {
    // Opens at label+1 and closes at label+2: it both tore AND never saw the
    // label. t17 carries 35 of exactly this shape.
    let late_straddle = [bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 2)];
    for policy in [
        HourLabelMismatchPolicy::Refuse,
        HourLabelMismatchPolicy::CountOnly,
    ] {
        assert_eq!(
            rejected_bracket_domains(&late_straddle, LABEL_HOUR, policy)
                .get(&DOMAIN_ID)
                .map(|outcome| outcome.0),
            Some(DomainGapReason::Torn),
            "a straddle is Torn regardless of policy"
        );
    }
    assert_eq!(
        hour_label_mismatched_bracket_count(&late_straddle, LABEL_HOUR, &BTreeSet::new()),
        (0, 1),
        "and it is not double-counted as a mismatch"
    );
}

/// NO_CLOCK. The frame builder substitutes the sentinel `0` when the caller had
/// no authoritative hour, and comparing a real observed hour against a
/// placeholder is not a judgement.
///
/// The cost of getting this wrong is not a bad statistic, it is a dead frame:
/// `hour_index_from_raw` rejects `0`, so the gap could not encode, and
/// `replace_rejected_domain_records_with_gaps` would return `Err` and take the
/// whole frame's rejection path -- including its genuine tears -- with it. The
/// second half of this test is that failure, made concrete.
///
/// MUTATION RUN: delete the `frame_label_is_a_real_hour(game_hour_raw) &&`
/// guard from `bracket_hour_verdict`.
/// OBSERVED: `a sentinel label must not refuse anything` — left `(1, 1)`,
/// right `(0, 1)`. Removing that first assertion as well, so the second half
/// runs, gives the failure this guard actually prevents:
/// `a NO_CLOCK frame must still publish, not fail to encode a gap:
/// "encode read-only DOMAIN_GAP: H4DG game hour 0 is invalid"`.
#[test]
fn a_no_clock_sentinel_label_is_not_something_a_bracket_can_mismatch() {
    const NO_CLOCK_SENTINEL: i32 = 0;
    let brackets = [bracket(DOMAIN_ID, LABEL_HOUR, LABEL_HOUR)];

    assert_eq!(
        hour_label_mismatched_bracket_count(&brackets, NO_CLOCK_SENTINEL, &BTreeSet::new()),
        (0, 1),
        "a sentinel label must not refuse anything: it was never an observation"
    );

    let mut records = Vec::new();
    append_compact_record(
        &mut records,
        1,
        NO_CLOCK_SENTINEL,
        DOMAIN_ID,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &[7u8; 8],
    );
    replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        NO_CLOCK_SENTINEL,
        &brackets,
        &mut BTreeMap::new(),
        HourLabelMismatchPolicy::Refuse,
    )
    .expect("a NO_CLOCK frame must still publish, not fail to encode a gap");
}

/// The witnesses must PROVE the reason, not merely accompany it.
///
/// `Torn` is believed on the producer's word -- the grammar checks only that
/// the ticks differ, which is what the producer already said. This reason is
/// checkable end to end from the payload alone, and that is worth having
/// because it is the one reason whose entire content is a claim about a
/// comparison the consumer can redo.
///
/// MUTATION RUN, one conjunct at a time, in `validate_domain_gap_payload`:
///   * drop `tick_before == tick_after` -> the torn case below encodes;
///   * drop `tick_before != target_hour` -> the in-hour case below encodes,
///     which would let this reason swallow every clean domain in the capture;
///   * drop `tick_before != 0` -> the witnessless case below encodes.
/// OBSERVED: each dropped conjunct turns its OWN `expect_err` into a panic,
/// and no other test in the workspace notices --
///   equality -> `a mismatch whose ticks DIFFER is a straddle wearing another name`
///   nonzero  -> `a mismatch whose ticks are the wire's absent-witness value proves nothing`
///   label    -> `a mismatch whose ticks ARE the label proves the opposite of its reason`
/// Each panic prints the payload the grammar wrongly accepted.
#[test]
fn the_hour_label_mismatch_grammar_refuses_a_witness_that_does_not_prove_it() {
    let record = |tick_before: u64, tick_after: u64| DomainGapRecordV1 {
        part: DomainGapPart::State,
        reason: DomainGapReason::HourLabelMismatch,
        target_hour_raw: LABEL_HOUR,
        range_start_hour_raw: LABEL_HOUR,
        range_end_hour_raw: LABEL_HOUR,
        tick_before,
        tick_after,
        queue_capacity: 0,
        queue_depth: 0,
        queue_dropped: 0,
        outcome_ordinal: 0,
    };
    let observed = u64::try_from(LABEL_HOUR + 1).unwrap();
    let label = u64::try_from(LABEL_HOUR).unwrap();

    record(observed, observed)
        .encode()
        .expect("equal, nonzero, not the label: the reason is proven");
    record(observed, observed + 1)
        .encode()
        .expect_err("a mismatch whose ticks DIFFER is a straddle wearing another name");
    record(label, label)
        .encode()
        .expect_err("a mismatch whose ticks ARE the label proves the opposite of its reason");
    record(0, 0)
        .encode()
        .expect_err("a mismatch whose ticks are the wire's absent-witness value proves nothing");
}

/// TWO BRACKETS, ONE DOMAIN. The switch may change WHETHER a domain is refused
/// and must never change WHY.
///
/// A domain can hold more than one bracket -- d47's prewarm bracket is
/// deliberately recorded before d47's worker bracket -- and the rejection map
/// keeps the FIRST outcome per domain. Minting the mismatch in the same pass as
/// the tear therefore let the mismatch take the slot the tear wanted, but only
/// under `Refuse`: the same frame reported `HourLabelMismatch` with the rule on
/// and `Torn` with it off. The domain is refused either way, so no capture
/// would have lost data over it -- it would simply have been told the wrong
/// cause, and this project's whole position is that a refused hour must name
/// what happened to it.
///
/// The invariant asserted here is the strong one, and it is what makes
/// `CountOnly` a measurement rather than a different mode: `Refuse` yields
/// exactly `CountOnly`'s outcomes plus domains nothing else had rejected.
///
/// MUTATION RUN: move the `LabelMismatch` arm back into the first loop (return
/// `Some((DomainGapReason::HourLabelMismatch, opening, closing))` there under
/// `Refuse`) and delete the second pass.
/// OBSERVED: `a real tear outranks a label mismatch for the same domain` — left
/// `Some(HourLabelMismatch)`, right `Some(Torn)`. The other tests in this file
/// stay green, because every one of them gives each domain a single bracket.
#[test]
fn a_tear_on_one_bracket_outranks_a_label_mismatch_on_another_for_the_same_domain() {
    // Ordered as production orders them: the prewarm's bracket is recorded
    // first, the worker's second.
    let brackets = [
        bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1),
        bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 2),
    ];

    let refused = rejected_bracket_domains(&brackets, LABEL_HOUR, HourLabelMismatchPolicy::Refuse);
    let counted =
        rejected_bracket_domains(&brackets, LABEL_HOUR, HourLabelMismatchPolicy::CountOnly);

    assert_eq!(
        refused.get(&DOMAIN_ID).map(|outcome| outcome.0),
        Some(DomainGapReason::Torn),
        "a real tear outranks a label mismatch for the same domain"
    );
    assert_eq!(
        refused, counted,
        "the policy must not change the reason for a domain that is refused          either way"
    );
    assert_eq!(
        hour_label_mismatched_bracket_count(&brackets, LABEL_HOUR, &BTreeSet::new()),
        (0, 1),
        "and a domain already lost to a tear is not counted a second time as          the mismatch's price"
    );
}

/// A LABEL THE CALENDAR REFUSES is not only the NO_CLOCK sentinel.
///
/// `0` is the only value with NO_CLOCK provenance, but the authenticated clock
/// read and the scheduler stamp both accept any stable `i32` without consulting
/// the calendar, so a corrupt or out-of-window label reaches the rule too. The
/// refusal must stay suppressed for those -- the gap could not encode against
/// such a label, and trying would fail the whole frame's rejection path -- but
/// the frame must SAY the rule did not apply, or a zero mismatch count is
/// indistinguishable from a clean frame.
///
/// MUTATION RUN: delete the `lw_hour_label_unusable` unit from
/// `append_compact_lane_wall_probe_record`.
/// OBSERVED: the probe test in `state_fixture` fails on
/// `a frame whose label the calendar admits must say the rule applied` — left
/// `None`, right `Some(0)`. Nothing in this file notices, which is why the
/// assertion lives there and not here.
#[test]
fn an_out_of_calendar_label_suppresses_the_rule_the_same_way_the_sentinel_does() {
    // Nonzero, stable, and not a hour any calendar admits.
    const CORRUPT_LABEL: i32 = 1;
    assert!(
        !frame_label_is_a_real_hour(CORRUPT_LABEL),
        "premise: the calendar refuses this label"
    );
    let brackets = [bracket(DOMAIN_ID, LABEL_HOUR, LABEL_HOUR)];

    assert!(
        rejected_bracket_domains(&brackets, CORRUPT_LABEL, HourLabelMismatchPolicy::Refuse)
            .is_empty(),
        "a label the calendar refuses cannot be mismatched against"
    );
    // And the reason it cannot: the record does not exist.
    assert!(
        DomainGapRecordV1 {
            part: DomainGapPart::State,
            reason: DomainGapReason::HourLabelMismatch,
            target_hour_raw: CORRUPT_LABEL,
            range_start_hour_raw: CORRUPT_LABEL,
            range_end_hour_raw: CORRUPT_LABEL,
            tick_before: u64::try_from(LABEL_HOUR).unwrap(),
            tick_after: u64::try_from(LABEL_HOUR).unwrap(),
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        }
        .encode()
        .is_err(),
        "refusing against such a label could not be expressed on the wire,          which is why the rule declines rather than tries"
    );
}

/// A MISMATCH NEVER OVERWRITES AN OUTCOME THAT IS ALREADY ON THE WIRE.
///
/// The two-pass ordering settles precedence among BRACKETS. It says nothing
/// about a domain whose outcome never came from a bracket at all: d81 stages
/// its own `SourceUnavailable` H4DG before bracket reconciliation runs, and
/// reconciliation deletes and re-mints the record of any domain it rejects. So
/// a mismatched d81 bracket would have replaced a true statement about the READ
/// ("the source was not there") with one about its ATTRIBUTION -- and only
/// under `Refuse`, which is the switch changing something other than whether a
/// domain is refused.
///
/// No DATA moves either way. The domain-hour was already lost, and the reason
/// it was lost is the more specific fact, so the existing gap stands.
///
/// A tear is deliberately NOT treated this way and the second half asserts it:
/// `Torn` says the read itself was invalid, which outranks whatever the reader
/// believed it was reporting.
///
/// MUTATION RUN: delete the `rejected.remove(&header.domain_id)` guard from
/// `replace_rejected_domain_records_with_gaps`.
/// OBSERVED: `an existing gap keeps the reason it was minted with` — left
/// `HourLabelMismatch`, right `SourceUnavailable`. Nothing else in the
/// workspace notices: `a_tear_on_one_bracket_outranks...` grades the map, and
/// this is what the map does to records afterwards.
#[test]
fn a_label_mismatch_leaves_an_already_published_gap_alone() {
    let staged = |reason| {
        DomainGapRecordV1 {
            part: domain_gap_part(DOMAIN_ID),
            reason,
            target_hour_raw: LABEL_HOUR,
            range_start_hour_raw: LABEL_HOUR,
            range_end_hour_raw: LABEL_HOUR,
            tick_before: u64::try_from(LABEL_HOUR).unwrap(),
            tick_after: u64::try_from(LABEL_HOUR).unwrap(),
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        }
        .encode()
        .expect("staged gap")
    };
    let seeded = |payload: &[u8]| {
        let mut records = Vec::new();
        append_compact_record(
            &mut records,
            1,
            LABEL_HOUR,
            DOMAIN_ID,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
            0,
            payload,
        );
        records
    };
    let decode = |records: &[u8]| {
        let header = compact_contract::parse_compact_record_header(records).unwrap();
        DomainGapRecordV1::decode(
            &records[compact_contract::COMPACT_RECORD_HEADER_LEN..header.record_len()],
        )
        .expect("reconciled record is a valid H4DG")
        .reason
    };

    // The domain already said why it has no rows. A wrong-hour bracket must not
    // overwrite that.
    let mut records = seeded(&staged(DomainGapReason::SourceUnavailable));
    replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        LABEL_HOUR,
        &[bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1)],
        &mut BTreeMap::from([((DOMAIN_ID, domain_gap_part(DOMAIN_ID).code()), 1)]),
        HourLabelMismatchPolicy::Refuse,
    )
    .expect("reconciliation succeeds");
    assert_eq!(
        decode(&records),
        DomainGapReason::SourceUnavailable,
        "an existing gap keeps the reason it was minted with"
    );

    // CONTROL: a TEAR does replace it, so the assertion above is not passing
    // because reconciliation stopped replacing anything.
    let mut torn = seeded(&staged(DomainGapReason::SourceUnavailable));
    replace_rejected_domain_records_with_gaps(
        &mut torn,
        1,
        LABEL_HOUR,
        &[bracket(DOMAIN_ID, LABEL_HOUR, LABEL_HOUR + 1)],
        &mut BTreeMap::from([((DOMAIN_ID, domain_gap_part(DOMAIN_ID).code()), 1)]),
        HourLabelMismatchPolicy::Refuse,
    )
    .expect("reconciliation succeeds");
    assert_eq!(
        decode(&torn),
        DomainGapReason::Torn,
        "a torn read invalidates the reader's own account of itself"
    );
}

/// THE COUNTER AND THE REFUSAL MUST AGREE, including about the domain the
/// refusal declines to touch.
///
/// `a_label_mismatch_leaves_an_already_published_gap_alone` establishes that a
/// domain with an existing gap keeps it. That decision has a price consequence:
/// such a domain costs NOTHING to refuse -- its DATA never existed and its
/// outcome does not change -- so charging for it would over-state what flipping
/// the switch costs. The number the flip is decided on has to be the number the
/// flip would cost, and this is where the two could silently diverge, because
/// the counter runs on brackets at probe time and the preservation happens
/// later, during reconciliation.
///
/// `compact_domains_already_carrying_a_gap` is what keeps them in step, read
/// off the same record headers reconciliation reads.
///
/// MUTATION RUN: drop the `.filter(|domain| !already_gapped.contains(domain))`
/// from `hour_label_mismatched_bracket_count`.
/// OBSERVED: `a domain that already published a gap costs nothing to refuse` —
/// left `(1, 1)`, right `(0, 1)`. No other test notices: every other one passes
/// an empty set.
#[test]
fn the_instrument_does_not_charge_for_a_domain_whose_gap_is_already_published() {
    let brackets = [bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1)];

    assert_eq!(
        hour_label_mismatched_bracket_count(&brackets, LABEL_HOUR, &BTreeSet::new()),
        (1, 1),
        "premise: with nothing already published, this bracket does cost a \
         domain-hour"
    );
    assert_eq!(
        hour_label_mismatched_bracket_count(
            &brackets,
            LABEL_HOUR,
            &BTreeSet::from([DOMAIN_ID])
        ),
        (0, 1),
        "a domain that already published a gap costs nothing to refuse"
    );
}

/// The already-gapped set is read off the wire, so it must agree with what
/// reconciliation sees rather than with a tracker that could drift.
///
/// MUTATION RUN: make `compact_domains_already_carrying_a_gap` match on
/// `COMPACT_RECORD_TYPE_DOMAIN_DATA` instead of `..._DOMAIN_GAP`.
/// OBSERVED: `only a published GAP suppresses the charge` — left `{20}`,
/// right `{}` on the DATA buffer, and the GAP buffer then yields `{}` where
/// `{20}` is required.
#[test]
fn the_already_gapped_set_is_read_from_the_frames_own_records() {
    let mut with_data = Vec::new();
    append_compact_record(
        &mut with_data,
        1,
        LABEL_HOUR,
        DOMAIN_ID,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &[7u8; 8],
    );
    assert_eq!(
        compact_domains_already_carrying_a_gap(&with_data),
        BTreeSet::new(),
        "only a published GAP suppresses the charge; a DATA record is exactly \
         the thing the refusal would take away"
    );

    let mut with_gap = Vec::new();
    append_compact_record(
        &mut with_gap,
        1,
        LABEL_HOUR,
        DOMAIN_ID,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
        0,
        &DomainGapRecordV1 {
            part: domain_gap_part(DOMAIN_ID),
            reason: DomainGapReason::SourceUnavailable,
            target_hour_raw: LABEL_HOUR,
            range_start_hour_raw: LABEL_HOUR,
            range_end_hour_raw: LABEL_HOUR,
            tick_before: u64::try_from(LABEL_HOUR).unwrap(),
            tick_after: u64::try_from(LABEL_HOUR).unwrap(),
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        }
        .encode()
        .expect("staged gap"),
    );
    assert_eq!(
        compact_domains_already_carrying_a_gap(&with_gap),
        BTreeSet::from([DOMAIN_ID])
    );

    // A truncated buffer yields what was parsed, and does not panic: this is
    // instrumentation, and reconciliation is about to report the real problem.
    let truncated = &with_gap[..with_gap.len() - 4];
    assert_eq!(
        compact_domains_already_carrying_a_gap(truncated),
        BTreeSet::new()
    );
}

/// The SHIPPED policy is pinned, because nothing else pins it.
///
/// `the_policy_switch_changes_the_refusal_and_not_the_measurement` proves both
/// arms behave correctly, and it does so by passing the policy in explicitly —
/// which is the right design for that test and is exactly why it stays green
/// whichever way the production constant points. I verified that: flipping
/// `HOUR_LABEL_MISMATCH_POLICY` to `Refuse` leaves all twelve tests in this
/// module passing.
///
/// So the constant is unguarded, and it is not a cosmetic one. Turning it on
/// refuses every domain whose bracket read an hour other than its frame's
/// label, and the cost of that is bounded only between 4.6% and 35.5% of
/// published domain-hours — on top of a capture that already loses 32.6% to
/// backpressure. An accidental flip would be a large, silent, and very
/// plausible-looking regression: fewer rows, no new failures, every test green.
///
/// Flipping it is a deliberate act that needs
/// `lw_hour_label_mismatch_brackets / lw_bracket_count` from a live capture
/// first. This test is what makes the flip deliberate rather than possible.
///
/// MUTATION: set `HOUR_LABEL_MISMATCH_POLICY` to `Refuse` and this fails.
#[test]
fn the_shipped_policy_is_count_only_until_a_capture_prices_the_refusal() {
    assert_eq!(
        HOUR_LABEL_MISMATCH_POLICY,
        HourLabelMismatchPolicy::CountOnly,
        "the refusal costs between 4.6% and 35.5% of published domain-hours and \
         that band has not been narrowed by a capture yet; flip this only with \
         the measured lw_hour_label_mismatch_brackets / lw_bracket_count ratio \
         in hand"
    );
}
