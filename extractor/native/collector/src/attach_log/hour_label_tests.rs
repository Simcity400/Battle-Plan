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
        opening: Some(clock_obs(opening, 1)),
        closing: Some(clock_obs(closing, 2)),
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
                opening: Some(clock_obs(LABEL_HOUR, 1)),
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
/// One bracket does not cover one domain. d49's worker builds d50's member
/// payload out of the same walk that produced its own rows, so ONE mismatched
/// bracket there costs TWO domain-hours -- and the reserved probe domain is
/// never refused at all, so a bracket held for it costs none. A bracket count
/// would have understated the first and overstated the second, and both errors
/// are invisible in a capture: they are just a smaller or larger number with
/// nothing to compare it to.
///
/// This used to be written on the country focus+tech child, which walked
/// domains 12 AND 13 under one tag. Lane LE split that child in two, so every
/// country bracket now covers exactly one domain and d49/d50 is the surviving
/// two-domain bracket. The rule under test is unchanged.
///
/// MUTATION RUN: revert the instrument to counting brackets --
///   `brackets.iter().filter(|b| bracket_hour_verdict(b, hour) ==
///    BracketHourVerdict::LabelMismatch).count()`, with `brackets.len()` as the
///   denominator.
/// OBSERVED: `one d49 bracket costs two domain-hours, not one` — left
/// `(1, 2)`, right `(2, 3)`. Both errors show at once: the walk's two
/// domain-hours collapse to one, and the probe bracket is priced as a refusal
/// that will never happen. Every other test in this file stays green, because
/// every other bracket in them covers exactly its own domain.
#[test]
fn the_instrument_prices_a_mismatch_in_domain_hours_not_in_brackets() {
    let brackets = [
        // Covers d49 AND d50: two domain-hours behind one bracket, and the
        // honest price, because that worker really does read both in one walk.
        bracket(
            COMPACT_DOMAIN_NAVAL_COMBAT_LIFECYCLE_LIVE,
            LABEL_HOUR + 1,
            LABEL_HOUR + 1,
        ),
        // Never refused, so it must never be priced.
        bracket(RC_PROBE_DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1),
        bracket(DOMAIN_ID, LABEL_HOUR, LABEL_HOUR),
    ];
    assert_eq!(
        hour_label_mismatched_bracket_count(&brackets, LABEL_HOUR, &BTreeSet::new()),
        (2, 3),
        "one d49 bracket costs two domain-hours, not one; and the probe \
         domain costs none"
    );
    assert_eq!(
        rejected_bracket_domains(&brackets, LABEL_HOUR, HourLabelMismatchPolicy::Refuse).len(),
        2,
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
        (0, 0),
        "a domain that already published a gap costs nothing to refuse, AND is \
         not offered as a domain that could have"
    );
}

/// THE TWO HALVES OF THE RATIO ARE ONE POPULATION (lane GF, 2026-08-10).
///
/// The manifest publishes these two numbers together and calls the denominator
/// "refusable bracketed domain-hours". The numerator has filtered
/// `already_gapped` since the counter was written; the denominator had not, so
/// a domain that CANNOT enter the numerator was still being offered by the
/// denominator, and the published rate was an incremental loss divided by a
/// population that includes something incapable of contributing to it. On t45
/// that was d81 on 341 of 595 frames.
///
/// The test uses TWO domains so the error is visible as a ratio rather than as
/// a count: one mismatched and refusable, one mismatched and already gapped.
/// The right answer is 1 of 1, not 1 of 2.
///
/// MUTATION RUN: drop `&& !already_gapped.contains(domain)` from the
/// `bracketed` filter in `hour_label_mismatched_bracket_count`.
/// OBSERVED: `the denominator must offer only domains the numerator could
/// count` — left `(1, 2)`, right `(1, 1)`.
#[test]
fn the_denominator_offers_only_domains_the_numerator_could_count() {
    const OTHER_DOMAIN_ID: u16 = DOMAIN_ID + 1;
    let brackets = [
        bracket(DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1),
        bracket(OTHER_DOMAIN_ID, LABEL_HOUR + 1, LABEL_HOUR + 1),
    ];

    assert_eq!(
        hour_label_mismatched_bracket_count(&brackets, LABEL_HOUR, &BTreeSet::new()),
        (2, 2),
        "premise: with nothing already published both mismatches are charged \
         against both brackets"
    );
    assert_eq!(
        hour_label_mismatched_bracket_count(
            &brackets,
            LABEL_HOUR,
            &BTreeSet::from([OTHER_DOMAIN_ID])
        ),
        (1, 1),
        "the denominator must offer only domains the numerator could count"
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
/// So the constant is unguarded, and it is not a cosmetic one. In either
/// direction an accidental flip is a silent semantic change: to `CountOnly`,
/// mislabelled data quietly re-admitted to the stream — the exact condition
/// the owner's ruling names a stop condition; to a stricter future policy, a
/// plausible-looking completeness regression with every test green.
///
/// Capture t20 (2026-08-04, `runs/live_unlock/t20`) priced the refusal the
/// old docstring said was unpriceable without the instrument: summed over its
/// 252 lane-wall probes, `lw_hour_label_mismatch_domains` = 574 against
/// `lw_bracketed_domains` = 17,136 — 3.35% of bracketed domain-reads, worst
/// frame 44 of 68. With the price known, keeping mislabelled hours was no
/// longer defensible, so the shipped policy is `Refuse`. This test is what
/// makes any change back a deliberate act rather than a possible one.
///
/// MUTATION: set `HOUR_LABEL_MISMATCH_POLICY` to `CountOnly` and this fails.
#[test]
fn the_shipped_policy_refuses_mislabelled_hours_since_t20_priced_them() {
    assert_eq!(
        HOUR_LABEL_MISMATCH_POLICY,
        HourLabelMismatchPolicy::Refuse,
        "t20 measured the refusal at 3.35% of bracketed domain-reads (574 of \
         17,136) and the owner's hour-misattribution ruling makes keeping \
         mislabelled hours a stop condition; reverting to CountOnly re-admits \
         them and needs an owner decision, not an edit"
    );
}

// ---------------------------------------------------------------------------
// WHERE the boundary fell, not only how much it cost.
//
// t21 established the mechanism: frame build p50 89.7 ms against hours whose
// period p50 is 154 ms but whose p05 is 75 ms, and 102 of the 317 hours with a
// successor ended before their own frame's build did. What the stream could NOT
// say is where INSIDE that build the hour ended, so every proposal to shorten
// or reorder the sweep could only be bounded, never priced.
// `frame_boundary_position` is that missing coordinate, and these are its
// contract.
// ---------------------------------------------------------------------------

/// A bracket with explicit ordinals AND explicit stamps, for the cases where
/// the two must be allowed to disagree. `clock_obs` deliberately ties them
/// together; this is the seam that unties them.
fn stamped(tag: u16, opening: (i32, u64, u64), closing: (i32, u64, u64)) -> DomainClockBracket {
    DomainClockBracket {
        tag,
        opening: Some(ClockObservation {
            hour: opening.0,
            ordinal: opening.1,
            at_us: opening.2,
        }),
        closing: Some(ClockObservation {
            hour: closing.0,
            ordinal: closing.1,
            at_us: closing.2,
        }),
        worker_panicked: false,
    }
}

/// The position is the FIRST post-label observation by ordinal, and the count
/// is the brackets that had closed before it.
///
/// MUTATION RUN: in `frame_boundary_position`, `min_by_key(|o| o.ordinal)`
/// changed to `max_by_key`, suite run in `--release`. OBSERVED:
///   left: Some(FrameBoundaryPosition { at_us: 90000, brackets_closed_before: 3 })
///  right: Some(FrameBoundaryPosition { at_us: 61000, brackets_closed_before: 2 })
/// The LAST witness reported as the boundary, which places every boundary at
/// the end of the build and makes the instrument say the opposite of the truth.
/// Six other tests in this file fail with it; every pre-existing hour-label
/// test stays green. Restored.
#[test]
fn the_boundary_position_is_the_first_post_label_observation() {
    // Four brackets. The first two run wholly inside the label and close at
    // ordinals 2 and 4; the third opens at ordinal 5 already in label+1, and a
    // fourth observes label+1 again later. The answer is the ordinal-5 read.
    let brackets = [
        stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000)),
        stamped(11, (LABEL_HOUR, 3, 21_000), (LABEL_HOUR, 4, 40_000)),
        stamped(12, (LABEL_HOUR + 1, 5, 61_000), (LABEL_HOUR + 1, 6, 80_000)),
        stamped(13, (LABEL_HOUR + 1, 7, 81_000), (LABEL_HOUR + 1, 8, 90_000)),
    ];
    assert_eq!(
        frame_boundary_position(&brackets, LABEL_HOUR),
        Some(FrameBoundaryPosition {
            at_us: 61_000,
            brackets_closed_before: 2,
        }),
        "the boundary is witnessed by the earliest read that saw a later hour, \
         and two brackets had closed by then"
    );
}

/// ABSENT AND ZERO ARE DIFFERENT FACTS, and the type says so.
///
/// A frame whose sweep ran wholly inside its own hour witnessed no boundary at
/// all; a frame whose very first clock read already saw the next hour witnessed
/// one at offset zero. Collapsing them would turn the healthiest 220 frames of
/// t21 into the sickest.
///
/// MUTATION RUN: the `?` on the `min_by_key` replaced with
/// `.unwrap_or(ClockObservation { hour: game_hour_raw, ordinal: 0, at_us: 0 })`
/// — a measured zero where nothing was observed. OBSERVED:
///   left: Some(FrameBoundaryPosition { at_us: 0, brackets_closed_before: 0 })
///  right: None
/// Every clean frame reports a boundary at the build origin. Restored.
#[test]
fn no_boundary_is_absent_and_a_boundary_at_the_origin_is_zero() {
    let clean = [
        stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000)),
        stamped(11, (LABEL_HOUR, 3, 21_000), (LABEL_HOUR, 4, 40_000)),
    ];
    assert_eq!(
        frame_boundary_position(&clean, LABEL_HOUR),
        None,
        "a sweep that never saw a later hour witnessed no boundary"
    );

    // The same shape, except the very first read of the build already saw the
    // next hour: a MEASURED zero.
    let from_the_origin = [stamped(10, (LABEL_HOUR + 1, 1, 0), (LABEL_HOUR + 1, 2, 20_000))];
    assert_eq!(
        frame_boundary_position(&from_the_origin, LABEL_HOUR),
        Some(FrameBoundaryPosition {
            at_us: 0,
            brackets_closed_before: 0,
        }),
        "a build that began in the next hour witnessed the boundary at its own \
         origin, which is a measurement and not an absence"
    );
}

/// Any LATER hour, not only `label + 1`.
///
/// t21 carries 16 `HourLabelMismatch` witnesses at `(+2, +2)` — two boundaries
/// inside one build. A frame whose first post-label observation is label+2 has
/// still witnessed a boundary; requiring exactly label+1 would report it as
/// having witnessed nothing, which is the opposite of the truth and would hide
/// precisely the worst frames.
///
/// MUTATION RUN: filter changed to `observation.hour == game_hour_raw + 1`.
/// OBSERVED: `left: None`, `right: Some(FrameBoundaryPosition { at_us: 45000,
/// brackets_closed_before: 1 })` — the frame that fell two hours behind reports
/// as clean, and it is the ONLY test in the file that fails, so nothing else
/// would have caught it. Restored.
#[test]
fn a_frame_that_fell_two_hours_behind_still_reports_its_boundary() {
    let brackets = [
        stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000)),
        stamped(11, (LABEL_HOUR + 2, 3, 45_000), (LABEL_HOUR + 2, 4, 60_000)),
    ];
    assert_eq!(
        frame_boundary_position(&brackets, LABEL_HOUR),
        Some(FrameBoundaryPosition {
            at_us: 45_000,
            brackets_closed_before: 1,
        })
    );
}

/// A label the calendar refuses yields no position, for the same reason it
/// suppresses the mismatch rule: comparing an observed hour against a
/// placeholder is arithmetic on something that was never an observation. The
/// `COMPACT_STATUS_NO_CLOCK` sentinel `0` is one such label.
///
/// MUTATION RUN: the `frame_label_is_a_real_hour` guard deleted. OBSERVED:
///   left: Some(FrameBoundaryPosition { at_us: 5000, brackets_closed_before: 0 })
///  right: None
/// Every bracket in a NO_CLOCK frame is "later than" the sentinel, so the
/// position reported is simply the frame's first read and means nothing.
/// Restored.
#[test]
fn a_label_the_calendar_refuses_has_no_boundary_position() {
    let brackets = [stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000))];
    assert!(!frame_label_is_a_real_hour(0), "0 is the NO_CLOCK sentinel");
    assert_eq!(frame_boundary_position(&brackets, 0), None);
    // CONTROL: the identical brackets against a real label they DO outrun, so
    // the assertion above is not passing because this shape never reports
    // anything.
    assert!(frame_boundary_position(&brackets, LABEL_HOUR - 1).is_some());
}

/// A PANICKED bracket is in neither population.
///
/// `bracket_hour_verdict` answers `WorkerPanic` for one, meaning nothing it
/// says about the clock is trustworthy. A position derived from a bracket that
/// function refuses would be the only place in the tree that trusts one, and
/// the two numbers returned would then describe different sets of brackets.
///
/// MUTATION RUN: the `!bracket.worker_panicked` filter deleted from the
/// `trustworthy` closure. OBSERVED:
///   left: Some(FrameBoundaryPosition { at_us: 30000, brackets_closed_before: 1 })
///  right: None
/// A boundary witnessed only by an unwinding worker published as the frame's
/// boundary. Restored.
#[test]
fn a_panicked_bracket_witnesses_no_boundary_and_closes_nothing() {
    let panicked = [
        stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000)),
        DomainClockBracket {
            worker_panicked: true,
            ..stamped(11, (LABEL_HOUR + 1, 3, 30_000), (LABEL_HOUR + 1, 4, 40_000))
        },
    ];
    assert_eq!(
        frame_boundary_position(&panicked, LABEL_HOUR),
        None,
        "the only witness unwound, and the panic is already its own refusal"
    );

    // And it is not counted as CLOSED either, so both numbers describe the same
    // population. Here a trustworthy bracket does witness the boundary, and the
    // panicked one that closed before it must not be counted.
    let mixed = [
        DomainClockBracket {
            worker_panicked: true,
            ..stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000))
        },
        stamped(11, (LABEL_HOUR, 3, 21_000), (LABEL_HOUR, 4, 40_000)),
        stamped(12, (LABEL_HOUR + 1, 5, 61_000), (LABEL_HOUR + 1, 6, 80_000)),
    ];
    assert_eq!(
        frame_boundary_position(&mixed, LABEL_HOUR),
        Some(FrameBoundaryPosition {
            at_us: 61_000,
            brackets_closed_before: 1,
        }),
        "one trustworthy bracket had closed; the panicked one is not counted"
    );
}

/// A bracket whose CLOSING read failed authentication has no ordinal, so its
/// position in the order is unknown and it is not counted as closed. Inventing
/// an order for it would be arithmetic on an absence, and the condition is
/// separately visible as a `ClockUnreadable` refusal.
///
/// MUTATION RUN: the count's predicate changed to
/// `bracket.closing.is_none_or(|c| c.ordinal < first.ordinal)`. OBSERVED:
///   left: Some(FrameBoundaryPosition { at_us: 61000, brackets_closed_before: 2 })
///  right: Some(FrameBoundaryPosition { at_us: 61000, brackets_closed_before: 1 })
/// A bracket that never proved it closed counted as part of the completed
/// sweep. Restored.
#[test]
fn an_unreadable_close_is_not_counted_as_closed() {
    let brackets = [
        stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000)),
        DomainClockBracket {
            tag: 11,
            opening: Some(clock_obs(LABEL_HOUR, 3)),
            closing: None,
            worker_panicked: false,
        },
        stamped(12, (LABEL_HOUR + 1, 5, 61_000), (LABEL_HOUR + 1, 6, 80_000)),
    ];
    assert_eq!(
        frame_boundary_position(&brackets, LABEL_HOUR),
        Some(FrameBoundaryPosition {
            at_us: 61_000,
            brackets_closed_before: 1,
        })
    );
}

/// The ORDINAL orders the reads; the stamp only reports one.
///
/// Two brackets on different threads can have their wall stamps land in the
/// opposite order to the reads themselves; only the ordinal is a total order.
/// If the position were selected by `at_us`, a frame would report a boundary
/// earlier than the read that actually witnessed it, and the closed-count —
/// which is keyed on the ordinal — would describe a different moment than the
/// offset published beside it.
///
/// MUTATION RUN: `min_by_key(|o| o.ordinal)` changed to
/// `min_by_key(|o| o.at_us)`. OBSERVED:
///   left: Some(FrameBoundaryPosition { at_us: 50000, brackets_closed_before: 1 })
///  right: Some(FrameBoundaryPosition { at_us: 52000, brackets_closed_before: 1 })
/// The reported offset is bracket 11's closing stamp while the count is still
/// keyed on the ordinal that selected it — one field describing one
/// observation and the other describing another. Restored.
#[test]
fn the_ordinal_orders_the_reads_and_the_stamp_only_reports_them() {
    // Ordinal 5 is the first post-label read, but its stamp landed AFTER
    // ordinal 6's. Bracket 11 closes at ordinal 6, i.e. after the witness.
    let brackets = [
        stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000)),
        stamped(12, (LABEL_HOUR + 1, 5, 52_000), (LABEL_HOUR + 1, 7, 80_000)),
        stamped(11, (LABEL_HOUR, 3, 21_000), (LABEL_HOUR + 1, 6, 50_000)),
    ];
    assert_eq!(
        frame_boundary_position(&brackets, LABEL_HOUR),
        Some(FrameBoundaryPosition {
            at_us: 52_000,
            brackets_closed_before: 1,
        }),
        "ordinal 5 is the first witness even though its stamp is later, and \
         only bracket 10 had closed before it"
    );
}

/// THE WIRE, not only the function: the three units reach the probe record with
/// the values the derivation produced, and the two value units are OMITTED —
/// not zeroed — when there is no boundary.
///
/// MUTATION RUN: both value units emitted unconditionally with
/// `boundary.map_or(0, |boundary| ..)`. OBSERVED: `left: Some(0)`,
/// `right: None` on `an absent boundary must publish NO position; a zero would
/// read as a build that began in the next hour` — the unit present at zero on
/// a frame that observed nothing, which a manifest cannot tell apart from a
/// build that began in the next hour. Restored.
#[test]
fn the_boundary_position_reaches_the_probe_record_and_absence_omits_it() {
    let summary = summarize_lane_spans(&[]);
    let witnessed = [
        stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000)),
        stamped(11, (LABEL_HOUR + 1, 3, 61_000), (LABEL_HOUR + 1, 4, 80_000)),
    ];
    let mut out = Vec::new();
    append_compact_lane_wall_probe_record(
        &mut out,
        7,
        LABEL_HOUR,
        5,
        900,
        125,
        Some(0),
        FrameHourAge::default(),
        &witnessed,
        &BTreeSet::new(),
        MapTopologyShareCounts::default(),
        130,
        700,
        23_641,
        &OutsideBuildSpans::default(),
        &summary,
        FrameCpuSplit::default(),
    );
    let units = compact_perf_diagnostic_units(&out[compact_contract::COMPACT_RECORD_HEADER_LEN..]);
    let value = |name: &str| {
        units
            .iter()
            .find(|unit| unit.name == name)
            .map(|unit| unit.value)
    };
    assert_eq!(value("lw_next_hour_at_us"), Some(61_000));
    assert_eq!(value("lw_next_hour_brackets_closed"), Some(1));
    assert_eq!(value("lw_next_hour_missing"), Some(0));
    assert_eq!(
        value("lw_fanout_origin_us"),
        Some(23_641),
        "the distance between the two origins this record mixes must be on the \
         wire, or a build-relative offset cannot be placed against a \
         fan-out-relative span"
    );

    // A frame whose sweep stayed inside its hour: the two value units are gone
    // and the flag says why.
    let clean = [stamped(10, (LABEL_HOUR, 1, 5_000), (LABEL_HOUR, 2, 20_000))];
    let mut out = Vec::new();
    append_compact_lane_wall_probe_record(
        &mut out,
        7,
        LABEL_HOUR,
        5,
        900,
        125,
        Some(0),
        FrameHourAge::default(),
        &clean,
        &BTreeSet::new(),
        MapTopologyShareCounts::default(),
        130,
        700,
        23_641,
        &OutsideBuildSpans::default(),
        &summary,
        FrameCpuSplit::default(),
    );
    let units = compact_perf_diagnostic_units(&out[compact_contract::COMPACT_RECORD_HEADER_LEN..]);
    let value = |name: &str| {
        units
            .iter()
            .find(|unit| unit.name == name)
            .map(|unit| unit.value)
    };
    assert_eq!(
        value("lw_next_hour_at_us"),
        None,
        "an absent boundary must publish NO position; a zero would read as a \
         build that began in the next hour"
    );
    assert_eq!(value("lw_next_hour_brackets_closed"), None);
    assert_eq!(value("lw_next_hour_missing"), Some(1));
}

// ---------------------------------------------------------------------------
// BUILD-START PHASE. The quantity the whole hour-label loss is a function of.
//
// t32 (`runs/live_unlock/t32`, 583 probed frames, 13,053 of 39,644 bracketed
// domain-hours refused `HourLabelMismatch`) has the build wall at p50 87.97 ms
// against a mean hour period of 90.4 ms, and all 180 of its frames that lost 44
// or more domains had `lw_o1_us == 0`: the build never waited for its hour,
// because that hour had already begun before the previous build finished.
//
// The quantity those two facts meet in -- how late into its own hour a build
// began -- had no wire unit. `lw_next_hour_at_us` describes a boundary only
// when one fell inside a trustworthy bracket, which is 316 of the 583 frames
// and 7 of the 274 clean ones, and turning it into a lateness needs a
// capture-wide MEAN hour period. These tests cover the unit that states it per
// frame and needs neither.
// ---------------------------------------------------------------------------

/// The observer stamps an hour ONLY when the clock it just read names a
/// different hour than the one it last stamped.
///
/// THE DEFECT THIS EXISTS FOR (Codex review
/// 019fd946-7d35-7302-a59f-4adbc749f025, finding 1). The first cut gated the
/// stamp on the observer's `advanced` flag, which compares
/// `ReadOnlyScheduler::generation` -- and generation is `telemetry.observations`,
/// which `observe_stable` increments on EVERY readable poll whether or not the
/// hour moved. So the current hour was re-stamped every ~2 ms, and every frame
/// would have reported a lateness of about one poll period however late it
/// really was. The instrument would have shown a perfectly healthy collector
/// while measuring nothing at all.
///
/// The rule therefore takes the last stamped hour, not a flag derived from a
/// counter that means something else. A production change back to a
/// generation-style "anything happened" gate fails the second assertion.
#[test]
fn only_a_poll_that_saw_a_new_hour_publishes_an_observation() {
    const HOUR: i32 = 60_832_190;
    assert_eq!(
        frontier_observation_to_publish(Some(HOUR - 1), Some((HOUR, 7))),
        Some(HOUR),
        "the poll that first sees an hour is the one that knows when it began"
    );
    assert_eq!(
        frontier_observation_to_publish(None, Some((HOUR, 7))),
        Some(HOUR),
        "the very first readable poll has nothing to compare against and stamps"
    );
    assert_eq!(
        frontier_observation_to_publish(Some(HOUR), Some((HOUR, 8))),
        None,
        "re-reading the hour already stamped observes no boundary; stamping it \
         would reset the age of an hour that began long ago and report every \
         build as having started at the boundary"
    );
    assert_eq!(
        frontier_observation_to_publish(Some(HOUR), None),
        None,
        "a clock-unreadable poll knows no hour to stamp"
    );
    assert_eq!(frontier_observation_to_publish(None, None), None);
    // A reload or re-baseline that moves the engine BACKWARDS is a new hour
    // beginning as far as this measurement goes.
    assert_eq!(
        frontier_observation_to_publish(Some(HOUR), Some((HOUR - 5, 9))),
        Some(HOUR - 5),
        "a backwards clock is a different hour, and a different hour is stamped"
    );
}

/// The age is a real interval or it is ABSENT. It is never a zero standing in
/// for "not measured", and it is never negative.
#[test]
fn the_build_start_lateness_is_measured_or_absent_and_never_negative() {
    let observed_at_us = 1_000_000u64;
    let seen = Some(observed_at_us);

    // The ordinary frame: the build began 66.5 ms into its own hour, which in
    // t32 is the class that loses 63-68 of its 68 bracketed domains.
    let late = FrameHourAge::measure(
        seen,
        Some(LABEL_HOUR),
        observed_at_us + 66_500,
        LABEL_HOUR,
        true,
    );
    assert_eq!(late.age_us, Some(66_500));
    assert_eq!(
        late.frontier_ahead, 0,
        "the engine had not moved past this frame's label"
    );

    // A build that began as its hour was observed. ZERO, and the zero is a
    // measurement.
    let prompt = FrameHourAge::measure(
        seen,
        Some(LABEL_HOUR),
        observed_at_us,
        LABEL_HOUR,
        true,
    );
    assert_eq!(prompt.age_us, Some(0));

    // The loop is a whole hour behind. The age is still honestly about THIS
    // frame's own hour -- that is what keying the stamp by label buys -- and
    // the backlog is reported separately instead of corrupting it.
    let behind = FrameHourAge::measure(
        seen,
        Some(LABEL_HOUR + 1),
        observed_at_us + 112_000,
        LABEL_HOUR,
        true,
    );
    assert_eq!(behind.age_us, Some(112_000));
    assert_eq!(behind.frontier_ahead, 1);

    // No stamp for this label -- the ring never held it, or an intermediate
    // hour of a multi-hour jump was never observed. Absent, not zero.
    assert_eq!(
        FrameHourAge::measure(
            None,
            Some(LABEL_HOUR),
            observed_at_us + 40_000,
            LABEL_HOUR,
            true
        )
        .age_us,
        None
    );

    // A label the calendar cannot place (the NO_CLOCK sentinel among them) is
    // not something this rule can describe.
    assert_eq!(
        FrameHourAge::measure(seen, Some(LABEL_HOUR), observed_at_us + 40_000, 0, false).age_us,
        None
    );

    // A build origin BEFORE the observation is not a negative age; it is an
    // observation this frame cannot be described by. Saturating it to zero
    // would publish "began exactly as the hour was seen" for a frame that did
    // not.
    assert_eq!(
        FrameHourAge::measure(
            seen,
            Some(LABEL_HOUR),
            observed_at_us - 3_000,
            LABEL_HOUR,
            true
        )
        .age_us,
        None
    );
}

/// The three units reach the wire, and absence omits the values rather than
/// zeroing them -- the same absent-vs-zero rule `lw_next_hour_at_us` follows.
#[test]
fn the_build_start_lateness_reaches_the_probe_record_and_absence_omits_it() {
    let summary = summarize_lane_spans(&[]);
    let probe = |hour_age: FrameHourAge| {
        let mut out = Vec::new();
        append_compact_lane_wall_probe_record(
            &mut out,
            7,
            LABEL_HOUR,
            5,
            900,
            125,
            Some(0),
            hour_age,
            &[],
            &BTreeSet::new(),
            MapTopologyShareCounts::default(),
            130,
            700,
            23_641,
            &OutsideBuildSpans::default(),
            &summary,
            FrameCpuSplit::default(),
        );
        compact_perf_diagnostic_units(&out[compact_contract::COMPACT_RECORD_HEADER_LEN..])
    };
    let value = |units: &[compact_contract::diagnostic_channel::DiagUnit], name: &str| {
        units
            .iter()
            .find(|unit| unit.name == name)
            .map(|unit| unit.value)
    };

    let measured = probe(FrameHourAge {
        age_us: Some(66_500),
        frontier_ahead: 0,
    });
    assert_eq!(value(&measured, "lw_hour_age_us"), Some(66_500));
    assert_eq!(value(&measured, "lw_hour_frontier_ahead"), Some(0));
    assert_eq!(value(&measured, "lw_hour_age_missing"), Some(0));

    let behind = probe(FrameHourAge {
        age_us: Some(12_000),
        frontier_ahead: 1,
    });
    assert_eq!(value(&behind, "lw_hour_frontier_ahead"), Some(1));

    let absent = probe(FrameHourAge::default());
    assert_eq!(
        value(&absent, "lw_hour_age_us"),
        None,
        "an unmeasured phase must publish NO value; a zero would read as a \
         build that began exactly at the boundary, which is the CLEANEST \
         possible frame"
    );
    assert_eq!(value(&absent, "lw_hour_frontier_ahead"), None);
    assert_eq!(value(&absent, "lw_hour_age_missing"), Some(1));
}

/// The stamp a build gets back is the one for ITS OWN hour.
///
/// THE DEFECT THIS EXISTS FOR (Codex review
/// 019fd92e-d28d-7ae0-9d77-4c6ea8bd3864, finding 6). The first cut of this
/// instrument kept a single "latest observation" pair. An observer that
/// advances between `take_current_request` and the build origin then leaves the
/// build reading H+1's stamp under label H, and the frame reports a lateness a
/// whole hour period too SMALL -- on a frame that was running late, which is
/// the population the unit exists to find. The newest-stamp reading is
/// systematically flattering exactly where it must not be.
///
/// A production change back to "newest stamp wins" fails the H-after-H+1
/// assertion below.
#[test]
fn a_build_reads_the_stamp_for_its_own_hour_not_the_newest_one() {
    hour_frontier_observation::reset_for_test();
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR).0,
        None,
        "before the observer has seen a single advance there is nothing to report"
    );
    assert_eq!(hour_frontier_observation::observation_for(LABEL_HOUR).1, None);

    let at = Instant::now();
    hour_frontier_observation::record(LABEL_HOUR, at);
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR).0,
        Some(hour_frontier_observation::micros_since_origin(at))
    );
    assert_eq!(hour_frontier_observation::observation_for(LABEL_HOUR).1, Some(LABEL_HOUR));

    // The engine moves on while the loop is still building the older hour.
    let later = Instant::now();
    hour_frontier_observation::record(LABEL_HOUR + 1, later);
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR).1,
        Some(LABEL_HOUR + 1)
    );
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR).0,
        Some(hour_frontier_observation::micros_since_origin(at)),
        "the older hour's stamp must survive the newer one, or a frame that is \
         a whole hour behind reports an age about an hour it is not labelled with"
    );
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR + 1).0,
        Some(hour_frontier_observation::micros_since_origin(later))
    );

    // An hour never observed -- an intermediate hour of a multi-hour jump --
    // borrows nobody else's stamp.
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR + 7).0,
        None
    );
    hour_frontier_observation::reset_for_test();

    // Same global ring, so this runs HERE and not as its own concurrent test.
    the_frontier_ring_keeps_recent_hours_and_evicts_the_oldest();
}

/// The ring holds a bounded history and evicts the OLDEST, so a long capture
/// keeps the recent hours a build can actually be labelled with.
///
/// Folded into the test above rather than standing alone: both mutate the same
/// PROCESS-GLOBAL ring, and `cargo test` runs tests concurrently, so as
/// separate `#[test]` functions each could erase the other's entries and fail
/// nondeterministically (Codex review 019fd946-7d35-7302-a59f-4adbc749f025,
/// finding 6).
fn the_frontier_ring_keeps_recent_hours_and_evicts_the_oldest() {
    // Driven off the real `DEPTH`, so raising or lowering it cannot silently
    // stop this from testing eviction at all.
    let depth = i32::try_from(hour_frontier_observation::DEPTH).expect("a sane ring depth");
    hour_frontier_observation::reset_for_test();
    // One advance more than the ring can hold.
    let mut stamps = Vec::new();
    for step in 0..=depth {
        let at = Instant::now();
        hour_frontier_observation::record(LABEL_HOUR + step, at);
        stamps.push(at);
    }
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR).0,
        None,
        "the oldest hour is the one evicted"
    );
    for step in 1..=depth {
        assert_eq!(
            hour_frontier_observation::observation_for(LABEL_HOUR + step).0,
            Some(hour_frontier_observation::micros_since_origin(
                stamps[step as usize]
            )),
            "hour +{step} must still be readable"
        );
    }

    // A re-observation of an hour already held REPLACES it rather than
    // consuming a second slot, which is what keeps a reload or clock
    // re-baseline from describing a fresh frame with a stale stamp.
    let held = depth / 2;
    let again = Instant::now();
    hour_frontier_observation::record(LABEL_HOUR + held, again);
    assert_eq!(
        hour_frontier_observation::observation_for(LABEL_HOUR + held).0,
        Some(hour_frontier_observation::micros_since_origin(again))
    );
    for step in 1..=depth {
        if step == held {
            continue;
        }
        assert!(
            hour_frontier_observation::observation_for(LABEL_HOUR + step)
                .0
                .is_some(),
            "re-observing hour +{held} must not have evicted hour +{step}"
        );
    }
    hour_frontier_observation::reset_for_test();
}
