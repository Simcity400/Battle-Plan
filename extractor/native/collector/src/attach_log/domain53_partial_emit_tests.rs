use super::*;
use diplomacy53::compact::{
    CountryDiplomacyRow, OpinionRow, SubjectAutonomyStatusRow, ROW_ID_DELETED,
};
use diplomacy53::extractor::ExtractedDomain53Rows;

const CLEAN: u32 = 0;
const PARTIAL: u32 = COMPACT_STATUS_PARTIAL_DATA;

fn op(src: u32, tgt: u32, value: i32) -> OpinionRow {
    OpinionRow {
        src_tag_db_id: src,
        target_tag_db_id: tgt,
        opinion: value,
    }
}

fn country(tag: u32, casualties: i64) -> CountryDiplomacyRow {
    CountryDiplomacyRow {
        tag_db_id: tag,
        faction_group: 0,
        at_war_count: 0,
        casualties,
    }
}

fn rows(countries: Vec<CountryDiplomacyRow>, opinions: Vec<OpinionRow>) -> ExtractedDomain53Rows {
    ExtractedDomain53Rows {
        countries,
        opinions,
        ..Default::default()
    }
}

fn apply(
    r: ExtractedDomain53Rows,
    status: u32,
    tracker: &mut CompactDiplomacyIntelOperativesLiveTracker,
) -> (ExtractedDomain53Rows, u32) {
    apply_mode(r, status, D53EmitMode::DeltaPartial, tracker)
}

fn apply_mode(
    r: ExtractedDomain53Rows,
    status: u32,
    mode: D53EmitMode,
    tracker: &mut CompactDiplomacyIntelOperativesLiveTracker,
) -> (ExtractedDomain53Rows, u32) {
    let (rows, status, commit) = compact_domain53_apply_emit_mode(r, status, mode, tracker);
    commit.commit(tracker);
    (rows, status)
}

fn is_delta(status: u32) -> bool {
    status & COMPACT_STATUS_DELTA_DATA != 0
}

#[test]
fn default_delta_mode_re_emits_full_on_partial() {
    let mut tracker = CompactDiplomacyIntelOperativesLiveTracker::default();
    apply_mode(
        rows(vec![], vec![op(1, 2, 5)]),
        CLEAN,
        D53EmitMode::Delta,
        &mut tracker,
    );
    let (emitted, status) = apply_mode(
        rows(vec![], vec![op(1, 2, 6), op(1, 3, 9)]),
        PARTIAL,
        D53EmitMode::Delta,
        &mut tracker,
    );
    assert!(
        !is_delta(status),
        "default Delta mode does not delta on partial"
    );
    assert_eq!(status & PARTIAL, PARTIAL);
    assert_eq!(
        emitted.opinions,
        vec![op(1, 2, 6), op(1, 3, 9)],
        "full read re-emitted"
    );
    assert_eq!(
        tracker.last_emitted.opinions.get(&(1, 2)).unwrap().opinion,
        5
    );
    assert!(tracker.last_emitted.opinions.get(&(1, 3)).is_none());
}

#[test]
fn unchanged_partial_emits_zero_base_rows() {
    let mut tracker = CompactDiplomacyIntelOperativesLiveTracker::default();
    let (base, base_status) = apply(
        rows(vec![country(1, 10)], vec![op(1, 2, 5)]),
        CLEAN,
        &mut tracker,
    );
    assert!(!is_delta(base_status), "clean baseline is not a delta");
    assert_eq!(base.opinions.len(), 1, "clean frame emits the full read");

    let (emitted, status) = apply(
        rows(vec![country(1, 10)], vec![op(1, 2, 5)]),
        PARTIAL,
        &mut tracker,
    );
    assert!(is_delta(status), "partial now emits a delta");
    assert_eq!(status & PARTIAL, PARTIAL, "partial bit preserved");
    assert_eq!(
        emitted.opinions.len(),
        0,
        "unchanged rows are not re-emitted"
    );
    assert_eq!(emitted.countries.len(), 0);
    assert_eq!(
        tracker.last_emitted.opinions.get(&(1, 2)).unwrap().opinion,
        5
    );
}

#[test]
fn delta_carries_complete_subject_autonomy_status_atomically() {
    fn status(ratio_raw: i32, current_score_raw: i64) -> SubjectAutonomyStatusRow {
        SubjectAutonomyStatusRow {
            overlord_tag_db_id: 1,
            subject_tag_db_id: 22,
            current_state_hash: 0xAA,
            previous_state_hash: Some(0xBB),
            next_state_hash: Some(0xCC),
            ratio_raw,
            current_score_raw,
            lower_score_raw: 0,
            upper_score_raw: 100_000,
            last_modified_date_raw: 60_795_475,
        }
    }

    let mut tracker = CompactDiplomacyIntelOperativesLiveTracker::default();
    let mut baseline = rows(vec![], vec![]);
    baseline.subject_autonomy_statuses = vec![status(40_000, 40_000)];
    baseline.subject_autonomy_status_complete = true;
    apply(baseline, CLEAN, &mut tracker);

    let mut current = rows(vec![], vec![]);
    current.subject_autonomy_statuses = vec![status(45_000, 45_000)];
    current.subject_autonomy_status_complete = true;
    let (emitted, emitted_status) = apply(current, CLEAN, &mut tracker);

    assert!(is_delta(emitted_status));
    assert_eq!(emitted.changed_rows, Some(0));
    assert!(emitted.subject_autonomy_status_complete);
    assert_eq!(
        emitted.subject_autonomy_statuses,
        vec![status(45_000, 45_000)]
    );
}

#[test]
fn unproven_subject_autonomy_is_withheld_and_remains_typed_partial_debt() {
    let status = SubjectAutonomyStatusRow {
        overlord_tag_db_id: 1,
        subject_tag_db_id: 22,
        current_state_hash: 0xAA,
        previous_state_hash: Some(0xBB),
        next_state_hash: Some(0xCC),
        ratio_raw: 40_000,
        current_score_raw: 40_000,
        lower_score_raw: 0,
        upper_score_raw: 100_000,
        last_modified_date_raw: 60_795_475,
    };
    let mut incomplete = diplomacy53::extractor::ExtractedDomain53Rows {
        subjects: vec![diplomacy53::compact::SubjectRow {
            overlord_tag_db_id: 1,
            subject_tag_db_id: 22,
            autonomy_state_hash: 0xAA,
            autonomy_ratio_raw: 40_000,
            start_date_raw: 60_700_000,
        }],
        subject_autonomy_statuses: vec![status.clone()],
        subject_autonomy_status_complete: false,
        ..diplomacy53::extractor::ExtractedDomain53Rows::default()
    };
    let debt = domain53_withhold_unproven_subject_autonomy(&mut incomplete);
    assert_eq!((debt.0, debt.1), (1, 1));
    assert!(incomplete.subject_autonomy_statuses.is_empty());
    assert_eq!(incomplete.subjects.len(), 1, "base subject rows survive");
    assert!(!incomplete.subject_autonomy_status_complete);

    let units = domain53_diag_units(&Domain53Diag {
        subject_autonomy_subject_rows_without_witness: debt.0,
        subject_autonomy_status_rows_withheld: debt.1,
        ..Domain53Diag::default()
    });
    let val = |name: &str| {
        units
            .iter()
            .find(|unit| unit.name == name)
            .map(|unit| unit.value)
    };
    assert_eq!(
        val("domain53_subject_autonomy_subject_rows_without_witness"),
        Some(1)
    );
    assert_eq!(
        val("domain53_subject_autonomy_status_rows_withheld"),
        Some(1)
    );

    let mut complete = diplomacy53::extractor::ExtractedDomain53Rows {
        subject_autonomy_statuses: vec![status],
        subject_autonomy_status_complete: true,
        ..diplomacy53::extractor::ExtractedDomain53Rows::default()
    };
    assert_eq!(
        domain53_withhold_unproven_subject_autonomy(&mut complete),
        (0, 0, Vec::new())
    );
    assert_eq!(complete.subject_autonomy_statuses.len(), 1);
}

#[test]
fn changed_opinion_on_partial_emits_only_that_row() {
    let mut tracker = CompactDiplomacyIntelOperativesLiveTracker::default();
    apply(
        rows(vec![], vec![op(1, 2, 5), op(1, 3, 7)]),
        CLEAN,
        &mut tracker,
    );
    let (emitted, status) = apply(
        rows(vec![], vec![op(1, 2, 6), op(1, 3, 7)]),
        PARTIAL,
        &mut tracker,
    );
    assert!(is_delta(status) && status & PARTIAL == PARTIAL);
    assert_eq!(emitted.opinions, vec![op(1, 2, 6)]);
}

#[test]
fn partial_never_advances_baseline_reverted_value_recorrected() {
    let mut tracker = CompactDiplomacyIntelOperativesLiveTracker::default();
    apply(rows(vec![], vec![op(1, 2, 5)]), CLEAN, &mut tracker);

    let (pushed, _) = apply(rows(vec![], vec![op(1, 2, 9)]), PARTIAL, &mut tracker);
    assert_eq!(pushed.opinions, vec![op(1, 2, 9)], "changed row emitted");
    assert_eq!(
        tracker.last_emitted.opinions.get(&(1, 2)).unwrap().opinion,
        5,
        "baseline still pinned at the clean value"
    );

    let (reverted, _) = apply(rows(vec![], vec![op(1, 2, 5)]), PARTIAL, &mut tracker);
    assert_eq!(
        reverted.opinions.len(),
        0,
        "reverted-to-baseline value emits nothing (== pinned baseline)"
    );
}

#[test]
fn absent_baseline_row_is_tombstoned_on_partial() {
    let mut tracker = CompactDiplomacyIntelOperativesLiveTracker::default();
    apply(
        rows(vec![], vec![op(1, 2, 5), op(1, 3, 7)]),
        CLEAN,
        &mut tracker,
    );
    let (emitted, status) = apply(rows(vec![], vec![op(1, 2, 5)]), PARTIAL, &mut tracker);
    assert!(is_delta(status) && status & PARTIAL == PARTIAL);
    assert_eq!(
        emitted.opinions.len(),
        1,
        "one tombstone for the absent row"
    );
    let tomb = &emitted.opinions[0];
    assert_ne!(tomb.src_tag_db_id & ROW_ID_DELETED, 0, "deleted bit set");
    assert_eq!(tomb.src_tag_db_id & !ROW_ID_DELETED, 1);
    assert_eq!(tomb.target_tag_db_id, 3);
    assert!(tracker.last_emitted.opinions.contains_key(&(1, 3)));
}

#[test]
fn partial_first_frame_is_full_and_seeds_baseline() {
    let mut tracker = CompactDiplomacyIntelOperativesLiveTracker::default();
    let (emitted, status) = apply(rows(vec![], vec![op(1, 2, 5)]), PARTIAL, &mut tracker);
    assert!(!is_delta(status), "first frame -> full emission, not delta");
    assert_eq!(status & PARTIAL, PARTIAL);
    assert_eq!(emitted.opinions, vec![op(1, 2, 5)], "full read emitted");
    assert!(
        !tracker.last_emitted.is_empty(),
        "first frame seeds the baseline even when partial"
    );

    let (delta, dstatus) = apply(rows(vec![], vec![op(1, 2, 6)]), PARTIAL, &mut tracker);
    assert!(is_delta(dstatus) && dstatus & PARTIAL == PARTIAL);
    assert_eq!(delta.opinions, vec![op(1, 2, 6)]);
    assert_eq!(
        tracker.last_emitted.opinions.get(&(1, 2)).unwrap().opinion,
        5
    );
}

#[test]
fn empty_first_frame_seeds_and_next_frame_deltas() {
    let mut tracker = CompactDiplomacyIntelOperativesLiveTracker::default();
    let (e0, s0) = apply(rows(vec![], vec![]), PARTIAL, &mut tracker);
    assert!(
        !is_delta(s0),
        "empty first frame emits full (baseline), not delta"
    );
    assert!(e0.opinions.is_empty());
    assert!(
        tracker.baseline_seeded,
        "empty first frame seeds the baseline"
    );
    assert!(tracker.last_emitted.is_empty(), "seeded baseline is empty");

    let (e1, s1) = apply(rows(vec![], vec![op(1, 2, 5)]), PARTIAL, &mut tracker);
    assert!(
        is_delta(s1),
        "second frame deltas (no re-seed) — the bug would re-emit full"
    );
    assert_eq!(s1 & PARTIAL, PARTIAL);
    assert_eq!(
        e1.opinions,
        vec![op(1, 2, 5)],
        "row emitted as new vs the empty baseline"
    );
    assert!(tracker.last_emitted.is_empty());
    let (e2, s2) = apply(rows(vec![], vec![op(1, 2, 5)]), PARTIAL, &mut tracker);
    assert!(is_delta(s2));
    assert_eq!(e2.opinions, vec![op(1, 2, 5)]);
}

#[test]
fn clean_frame_after_partials_advances_baseline() {
    let mut tracker = CompactDiplomacyIntelOperativesLiveTracker::default();
    apply(rows(vec![], vec![op(1, 2, 5)]), CLEAN, &mut tracker);
    apply(rows(vec![], vec![op(1, 2, 9)]), PARTIAL, &mut tracker);
    assert_eq!(
        tracker.last_emitted.opinions.get(&(1, 2)).unwrap().opinion,
        5
    );

    let (_clean2, s) = apply(rows(vec![], vec![op(1, 2, 9)]), CLEAN, &mut tracker);
    assert!(is_delta(s), "non-first clean frame emits a delta");
    assert_eq!(s & PARTIAL, 0, "clean frame carries no partial bit");
    assert_eq!(
        tracker.last_emitted.opinions.get(&(1, 2)).unwrap().opinion,
        9,
        "clean frame advanced the baseline"
    );
    let (after, _) = apply(rows(vec![], vec![op(1, 2, 9)]), PARTIAL, &mut tracker);
    assert_eq!(
        after.opinions.len(),
        0,
        "now equal to the advanced baseline"
    );
}

#[test]
fn full_mode_unaffected_on_partial() {
    let mut tracker = CompactDiplomacyIntelOperativesLiveTracker::default();
    apply(rows(vec![], vec![op(1, 2, 5)]), CLEAN, &mut tracker);
    let (emitted, status) = apply_mode(
        rows(vec![], vec![op(1, 2, 6)]),
        PARTIAL,
        D53EmitMode::Full,
        &mut tracker,
    );
    assert!(!is_delta(status));
    assert_eq!(emitted.opinions, vec![op(1, 2, 6)], "full read passthrough");
    assert!(
        tracker.last_emitted.is_empty(),
        "full mode clears the baseline"
    );
    assert!(
        !tracker.baseline_seeded,
        "full mode resets the seeded sentinel"
    );
}

#[test]
fn the_air_disclosure_threshold_rides_delta_frames_too() {
    fn rows_with_threshold(
        opinions: Vec<OpinionRow>,
        threshold_raw: Option<i64>,
    ) -> ExtractedDomain53Rows {
        ExtractedDomain53Rows {
            opinions,
            air_disclosure_threshold_raw: threshold_raw,
            ..Default::default()
        }
    }

    let mut tracker = CompactDiplomacyIntelOperativesLiveTracker::default();
    let (seed, seed_status) = apply(
        rows_with_threshold(vec![op(1, 2, 5)], Some(30_000)),
        CLEAN,
        &mut tracker,
    );
    assert!(!is_delta(seed_status), "the first frame is not a delta");
    assert_eq!(seed.air_disclosure_threshold_raw, Some(30_000));

    let (delta, delta_status) = apply(
        rows_with_threshold(vec![op(1, 2, 6)], Some(30_000)),
        CLEAN,
        &mut tracker,
    );
    assert!(is_delta(delta_status), "the second frame is a delta");
    assert_eq!(
        delta.air_disclosure_threshold_raw,
        Some(30_000),
        "a delta frame must carry the gate's right-hand side, not only its left"
    );

    let (unread, unread_status) = apply(
        rows_with_threshold(vec![op(1, 2, 7)], None),
        CLEAN,
        &mut tracker,
    );
    assert!(is_delta(unread_status));
    assert_eq!(unread.air_disclosure_threshold_raw, None);
}

/// The rc_vanilla7 per-frame word, run through the fold's own arithmetic.
///
/// OLD RULE: d53 had no tolerance entry, so this residual was
/// `COMPACT_STATUS_PARTIAL_DATA` and `world_fold` marked all 227 d53 DATA hours
/// `compact_status_failure` (`runs/tw_ledger_certification/hour_scan_rc_v7.json`,
/// `d53_problem_classes`).
/// NEW RULE: the residual is 0 and the hour seals, while any additional failure
/// bit still refuses.
#[test]
fn the_rc_vanilla7_d53_status_word_now_leaves_no_residual() {
    let observed = 0x0053_0200u32;
    assert_eq!(
        observed & COMPACT_STATUS_PARTIAL_DATA,
        COMPACT_STATUS_PARTIAL_DATA,
        "the measured word's only global failure bit is PARTIAL_DATA"
    );
    assert_eq!(
        compact_contract::compact_status_failure_flags_after_expected_partial_tolerance(
            53, observed,
        ),
        0
    );
    assert_ne!(
        compact_contract::compact_status_failure_flags_after_expected_partial_tolerance(
            53,
            observed | COMPACT_STATUS_SOURCE_UNAVAILABLE,
        ),
        0
    );
}

/// Lane TZ. Every d53 arm that can badge `PARTIAL_DATA` either declares its
/// omission or carries a second failure bit, so no consumer that sees only the
/// status word can tolerate an undeclared partial.
///
/// The two rowless arms are the inherited-upstream-failure return and the
/// post-build gate; both are `SOURCE_UNAVAILABLE | PARTIAL_DATA`. The
/// source-unavailable funnel every other fail-closed arm returns through adds
/// the same pair. `domain53_success_status_flags` no longer has a PARTIAL arm
/// at all.
///
/// kills: dropping `COMPACT_STATUS_SOURCE_UNAVAILABLE` from the inherited arm
/// makes the empty payload tolerable by flags in the stream gate and the
/// compact-replay verification gate, neither of which reads the payload.
#[test]
fn no_d53_arm_can_badge_a_bare_partial_without_declaring() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/attach_log.rs"
    ))
    .expect("attach_log source");
    let observe = source
        .find("fn read_compact_diplomacy_intel_operatives_live_observe")
        .expect("the observe half");
    let finish_end = source
        .find("fn domain53_withhold_unproven_subject_autonomy")
        .expect("the end of the finish half");
    let body = &source[observe..finish_end];
    for (index, line) in body.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || !trimmed.contains("STATUS_PARTIAL_DATA") {
            continue;
        }
        // The only bare PARTIAL in these two halves is the declared arm of the
        // status match, which is guarded by `declarable`.
        let bare = !trimmed.contains("SOURCE_UNAVAILABLE")
            && !trimmed.contains("& COMPACT_STATUS_PARTIAL_DATA")
            && !trimmed.contains("!COMPACT_STATUS_PARTIAL_DATA");
        if bare {
            assert!(
                trimmed.contains("(true, true) =>"),
                "line {index} badges a bare PARTIAL_DATA outside the declared arm: {trimmed}"
            );
        }
    }
    // BOUNDED TO THE FUNCTION: the whole-file needle it replaced matched a
    // PARTIAL arm in an unrelated lane's status function on an LF checkout
    // and nothing on a CRLF one.
    let success = source
        .find("fn domain53_success_status_flags(")
        .expect("the success status function");
    let success_end = source[success + 1..]
        .find("
fn ")
        .map(|offset| success + 1 + offset)
        .expect("the function after it");
    assert!(
        !source[success..success_end].contains("COMPACT_STATUS_PARTIAL_DATA"),
        "domain53_success_status_flags must not have a PARTIAL arm"
    );
}

/// Lane TZ. A cause with no per-row key cannot be declared, so the frame is
/// spelled `SOURCE_UNAVAILABLE | PARTIAL_DATA` and refuses everywhere.
#[test]
fn causes_with_no_per_row_key_are_undeclarable() {
    let with_shadow_mismatch = domain53_undeclarable_partial_causes(
        diplomacy53::extractor::Domain53SkipCounts::default(),
        1,
    );
    assert_eq!(with_shadow_mismatch, 1);

    let declarable_only = domain53_undeclarable_partial_causes(
        diplomacy53::extractor::Domain53SkipCounts {
            relation_opinion: 3,
            relation_subject: 2,
            relation_container: 3,
            operative: 71,
            ..Default::default()
        },
        0,
    );
    assert_eq!(
        declarable_only, 0,
        "the rc_vanilla7 census is entirely declarable"
    );

    for counts in [
        diplomacy53::extractor::Domain53SkipCounts {
            relation_root: 1,
            ..Default::default()
        },
        diplomacy53::extractor::Domain53SkipCounts {
            relation_slot: 1,
            ..Default::default()
        },
        diplomacy53::extractor::Domain53SkipCounts {
            incoming_action: 1,
            ..Default::default()
        },
        diplomacy53::extractor::Domain53SkipCounts {
            agency: 1,
            ..Default::default()
        },
        diplomacy53::extractor::Domain53SkipCounts {
            cryptology: 1,
            ..Default::default()
        },
        diplomacy53::extractor::Domain53SkipCounts {
            war: 1,
            ..Default::default()
        },
    ] {
        assert_eq!(domain53_undeclarable_partial_causes(counts, 0), 1);
    }
}

/// Lane TZ. The withheld AUST rows are named, not counted: the identities are
/// the `(overlord, subject)` keys of every subject relation the frame
/// publishes whose current status it could not witness.
#[test]
fn the_withhold_names_whose_current_autonomy_status_is_unknown() {
    let subject = diplomacy53::compact::SubjectRow {
        overlord_tag_db_id: 1,
        subject_tag_db_id: 2,
        autonomy_state_hash: 0,
        autonomy_ratio_raw: 0,
        start_date_raw: 0,
    };
    let mut rows = diplomacy53::extractor::ExtractedDomain53Rows {
        subjects: vec![subject],
        subject_autonomy_status_complete: false,
        ..diplomacy53::extractor::ExtractedDomain53Rows::default()
    };
    let (subject_rows, _status_rows, pairs) =
        domain53_withhold_unproven_subject_autonomy(&mut rows);
    assert_eq!(subject_rows, 1);
    assert_eq!(pairs, vec![(1, 2)]);
}

/// Lane TZ, Codex r2 finding 1. A refusal a declarable class could not NAME
/// still makes the frame partial, and then makes it undeclarable.
///
/// `read_operative_row`'s two identity guards are the ones that decide the id
/// is unusable, so a candidate they refuse takes an operative skip and gives no
/// `op_id`. Before the reconciliation such a frame reported an empty
/// declaration and no undeclarable cause, i.e. a CLEAN status word with a row
/// missing.
///
/// This drives `domain53_partial_decision`, which IS the production rule the
/// finish half destructures — not its ingredients.
///
/// kills: dropping `domain53_declaration_shortfall` from that function's
/// `undeclarable` sum makes the first case declarable and this test fails.
#[test]
fn a_skip_its_class_could_not_name_makes_the_frame_undeclarable() {
    use diplomacy53::compact::Domain53OmissionClass as C;
    let one_operative_skip = diplomacy53::extractor::Domain53SkipCounts {
        operative: 1,
        ..Default::default()
    };

    let unnamed = diplomacy53::compact::Domain53OmissionDeclaration::default();
    let decision = domain53_partial_decision(one_operative_skip, 0, &unnamed);
    assert!(decision.partial, "the census, not the declaration, decides");
    assert!(
        !decision.declarable,
        "an operative skip with no declared id cannot be declared"
    );
    assert_eq!(decision.undeclarable, 1);

    let mut named = diplomacy53::compact::Domain53OmissionDeclaration::default();
    named.declare(C::Operative, (7, 0));
    let decision = domain53_partial_decision(one_operative_skip, 0, &named);
    assert!(decision.partial && decision.declarable);
    assert_eq!(decision.undeclarable, 0);

    // The other direction is the same contradiction: an identity the census
    // never counted.
    let mut over = diplomacy53::compact::Domain53OmissionDeclaration::default();
    over.declare(C::Operative, (7, 0));
    over.declare(C::Operative, (8, 0));
    let decision = domain53_partial_decision(one_operative_skip, 0, &over);
    assert!(!decision.declarable);
    assert_eq!(decision.undeclarable, 1);

    // A cause with no per-row key at all.
    let shadow = domain53_partial_decision(
        diplomacy53::extractor::Domain53SkipCounts::default(),
        1,
        &diplomacy53::compact::Domain53OmissionDeclaration::default(),
    );
    assert!(shadow.partial && !shadow.declarable);

    // A frame that omits nothing declares nothing and is not partial.
    let clean = domain53_partial_decision(
        diplomacy53::extractor::Domain53SkipCounts::default(),
        0,
        &diplomacy53::compact::Domain53OmissionDeclaration::default(),
    );
    assert!(!clean.partial && !clean.declarable);

    // And the rc_vanilla7 census reconciles exactly.
    let rc_v7 = diplomacy53::extractor::Domain53SkipCounts {
        relation_opinion: 3,
        relation_subject: 2,
        relation_container: 3,
        operative: 71,
        ..Default::default()
    };
    let mut declaration = diplomacy53::compact::Domain53OmissionDeclaration::default();
    for op_id in 1..=71u32 {
        declaration.declare(C::Operative, (op_id, 0));
    }
    for target in [2u32, 3, 4] {
        declaration.declare(C::RelationOpinion, (1, target));
    }
    for target in [5u32, 6] {
        declaration.declare(C::RelationSubject, (1, target));
    }
    for target in [7u32, 8, 9] {
        declaration.declare(C::RelationContainer, (1, target));
    }
    for subject in 100..138u32 {
        declaration.declare(C::SubjectAutonomyStatus, (1, subject));
    }
    let decision = domain53_partial_decision(rc_v7, 0, &declaration);
    assert!(decision.partial && decision.declarable);
    assert_eq!(decision.undeclarable, 0);
    assert_eq!(declaration.total(), 117, "the rc_vanilla7 per-frame census");
}

/// Lane TZ, Codex r3 finding 3. The producer and the fold must read a
/// relation-flag state event's `object_hash` from ONE spelling, or a wording
/// change desyncs the fold's removal from the producer's event.
///
/// kills: reverting either call site to a local `format!` puts the literal back
/// in that file and this scan fails.
#[test]
fn the_relation_flag_object_hash_has_no_second_spelling() {
    for (label, path) in [
        (
            "the d53 extractor",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../lanes/diplomacy_intel_operatives_live/src/extractor.rs"
            ),
        ),
        (
            "the world fold",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../recorder_host/src/world_fold.rs"
            ),
        ),
    ] {
        let source = std::fs::read_to_string(path).expect(label);
        assert!(
            !source.contains("relation_flag_kind_{"),
            "{label} formats the relation-flag object name locally instead of calling \
             compact::relation_flag_kind_object_name"
        );
    }
    // And the one place that DOES own the spelling still spells it that way.
    let owner = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../lanes/diplomacy_intel_operatives_live/domain44_compact.rs"
    ))
    .expect("the compact module");
    assert!(owner.contains("relation_flag_kind_{flag_kind}"));
}

/// Lane TZ, Codex r6 finding 1. A delta spells two answers -- "unchanged" by
/// omitting a row, "gone" by tombstoning one -- and a declared identity is the
/// third: unreadable this frame. `DeltaPartial` predates D53O, so a declaring
/// frame must not take its path.
///
/// kills: dropping `|| rows.omissions.is_some()` from the guard in
/// `compact_domain53_apply_emit_mode` makes the second frame a delta whose
/// baseline row 1/3 is silently retained as unchanged, and whose declaration
/// `..Default::default()` would drop.
#[test]
fn a_declaring_partial_frame_is_never_delta_emitted() {
    use diplomacy53::compact::{Domain53OmissionClass, Domain53OmissionDeclaration};

    let mut tracker = CompactDiplomacyIntelOperativesLiveTracker::default();
    apply(
        rows(vec![], vec![op(1, 2, 5), op(1, 3, 7)]),
        CLEAN,
        &mut tracker,
    );

    let mut declaring = rows(vec![], vec![op(1, 2, 6)]);
    let mut declaration = Domain53OmissionDeclaration::default();
    declaration.declare(Domain53OmissionClass::RelationOpinion, (1, 3));
    declaring.omissions = Some(declaration.clone());

    let (emitted, status) = apply(declaring, PARTIAL, &mut tracker);
    assert!(!is_delta(status), "status 0x{status:08X}");
    assert_eq!(
        emitted.opinions,
        vec![op(1, 2, 6)],
        "the frame's own read, whole -- no tombstone for the declared pair"
    );
    assert_eq!(emitted.omissions, Some(declaration));
    assert_eq!(
        tracker.last_emitted.opinions.get(&(1, 2)).unwrap().opinion,
        5,
        "and the baseline stays pinned at the clean value"
    );
}

/// THE INTEL TABLE AND ITS ROSTER ARE ONE FAMILY ON THE DELTA PATH TOO.
///
/// `compact_domain53_apply_emit_mode` reconstructs the emitted rows field by
/// field. A delta frame that carried `intel_map` without `intel_map_roster`
/// would state a table against no roster, which both gates refuse -- so the
/// default delta mode could not carry the family past its seed frame.
///
/// kills: dropping `intel_map_roster` from the reconstruction.
#[test]
fn a_delta_frame_carries_the_intel_roster_with_its_table() {
    let square = |countries: &[u32], army: i32| {
        let mut rows = Vec::new();
        for owner in countries {
            for target in countries {
                if owner == target {
                    continue;
                }
                rows.push(diplomacy53::compact::IntelMapRow {
                    owner_tag_db_id: *owner,
                    target_tag_db_id: *target,
                    civilian_raw: 10_000,
                    army_raw: army,
                    navy_raw: 30_000,
                    air_raw: 40_000,
                    resisted: false,
                });
            }
        }
        rows
    };
    let frame = |army: i32| {
        let mut r = diplomacy53::extractor::proof_fixture_rows();
        r.operation_anchors.clear();
        r.intel_map = square(&[1, 3], army);
        r.intel_map_roster = vec![1, 3];
        r
    };
    let proofs = diplomacy53::extractor::CURRENT_VALIDATED_SOURCE_PROOFS
        | diplomacy53::extractor::SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF;

    let mut tracker = CompactDiplomacyIntelOperativesLiveTracker::default();
    apply_mode(frame(20_000), CLEAN, D53EmitMode::Delta, &mut tracker);
    let (emitted, status) = apply_mode(frame(25_000), CLEAN, D53EmitMode::Delta, &mut tracker);

    assert!(is_delta(status), "status 0x{status:08X}");
    assert_eq!(emitted.intel_map_roster, vec![1, 3]);
    let built = diplomacy53::extractor::build_domain53_payload_with_status(emitted, proofs, status);
    assert!(built.ok(), "{:?}", built.errors);
    assert!(
        diplomacy53::compact::decode_payload(0, &built.payload)
            .parse_errors
            .is_empty(),
        "{:?}",
        diplomacy53::compact::decode_payload(0, &built.payload).parse_errors
    );
}
