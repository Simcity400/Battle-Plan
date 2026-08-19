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
    assert_eq!(debt, (1, 1));
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
        (0, 0)
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
