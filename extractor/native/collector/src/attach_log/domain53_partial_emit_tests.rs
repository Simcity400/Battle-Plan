//! `domain53_partial_emit_tests`, extracted verbatim from `attach_log.rs`.

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

// The partial-frame delta behavior lives in the opt-in `DeltaPartial` mode;
// these tests exercise it. (`Delta` — the default — keeps today's full
// re-emit on partial frames; see `default_delta_mode_re_emits_full_on_partial`.)
// Mirrors the collector caller's SUCCESS path: run apply_emit_mode, then
// commit the returned baseline mutation (the gate always passes in these
// row-level tests). A failure-path test would simply drop the commit.
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

// The DEFAULT `Delta` mode is byte-identical to today on partial frames: it
// re-emits the full read (no delta), and does NOT seed or advance the delta
// baseline from a partial frame. This is the guard that keeps the measured
// +46-68 ms/frame partial-diff CPU cost OFF the default capture hot path.
#[test]
fn default_delta_mode_re_emits_full_on_partial() {
    let mut tracker = CompactDiplomacyIntelOperativesLiveTracker::default();
    // Establish a clean baseline in Delta mode.
    apply_mode(
        rows(vec![], vec![op(1, 2, 5)]),
        CLEAN,
        D53EmitMode::Delta,
        &mut tracker,
    );
    // A partial frame under Delta re-emits its full read unchanged.
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
    // Baseline untouched by the partial frame.
    assert_eq!(
        tracker.last_emitted.opinions.get(&(1, 2)).unwrap().opinion,
        5
    );
    assert!(tracker.last_emitted.opinions.get(&(1, 3)).is_none());
}

// A clean baseline followed by an UNCHANGED tolerated-partial frame: the
// legacy path re-emitted the full read; now it emits a DELTA|PARTIAL with
// ZERO base rows, and the baseline is untouched.
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
    // Baseline pinned across the partial.
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

// A genuine change on a partial frame emits ONLY the changed row.
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

// The baseline is NEVER advanced on a partial frame, so a value pushed on
// one partial and reverted on the next is corrected WITHOUT a projection
// accumulator (the fold re-seeds from the pinned baseline every hour).
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

// A baseline row absent from a partial read is tombstoned. The collector
// cannot distinguish a torn read from a genuine deletion (the extractor
// reports only a skipped-row COUNT, not keys), so it tombstones EVERY
// absent baseline row (tombstone-all). This reproduces today's full-emit
// snapshot byte-identically — today's seed-from-empty full-emit likewise
// drops both torn and deleted rows for the hour. See the plan doc for why
// tombstone-all was chosen over positive-absence-only tombstones.
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
    // Baseline still holds the tombstoned key (a future clean frame decides
    // whether it is truly gone); pinned across the partial.
    assert!(tracker.last_emitted.opinions.contains_key(&(1, 3)));
}

// The FIRST frame of a session is emitted in full (no delta) and seeds the
// baseline EVEN IF it is partial — deep-war captures are all-partial from
// frame 1, so gating the baseline on a clean frame would leave delta
// permanently off there. Subsequent partial frames then emit DELTA|PARTIAL.
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

    // The very next partial frame now emits a DELTA|PARTIAL vs that baseline.
    let (delta, dstatus) = apply(rows(vec![], vec![op(1, 2, 6)]), PARTIAL, &mut tracker);
    assert!(is_delta(dstatus) && dstatus & PARTIAL == PARTIAL);
    assert_eq!(delta.opinions, vec![op(1, 2, 6)]);
    // Baseline still pinned at the frame-1 value (partial never advances it).
    assert_eq!(
        tracker.last_emitted.opinions.get(&(1, 2)).unwrap().opinion,
        5
    );
}

// REGRESSION (Codex refutation finding 3/4): an EMPTY first frame (a
// success frame carrying zero rows) must still seed the baseline exactly
// once. With the old `last_emitted.is_empty()` sentinel it did NOT — the
// next non-empty frame RE-SEEDED, desyncing from the fold (which keys off
// `delta_baselines_seen`, set on the first frame) and silently dropping
// rows. The `baseline_seeded` bit fixes it: the first non-empty frame emits
// a DELTA (diffed against the empty baseline), not a full re-seed.
#[test]
fn empty_first_frame_seeds_and_next_frame_deltas() {
    let mut tracker = CompactDiplomacyIntelOperativesLiveTracker::default();
    // H0: empty success frame, partial.
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

    // H1: rows appear. The FIX makes this a DELTA (vs the empty baseline),
    // NOT a full re-seed — so the fold, seeding from its empty baseline,
    // reconstructs the rows exactly.
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
    // Baseline still empty (partial never advances) -> H2 re-emits it,
    // which is lossless (fold rebuilds it each hour from the empty base).
    assert!(tracker.last_emitted.is_empty());
    let (e2, s2) = apply(rows(vec![], vec![op(1, 2, 5)]), PARTIAL, &mut tracker);
    assert!(is_delta(s2));
    assert_eq!(e2.opinions, vec![op(1, 2, 5)]);
}

// REGRESSION (Codex refutation finding B): a frame that WOULD seed or
// advance the baseline but then FAILS the post-build gate must not mutate
// the tracker — otherwise the collector runs ahead of the fold (which
// rejects the failed frame) and a later delta applies against the wrong
// base. The mutation is returned as a commit and applied only on success;
// dropping the commit (simulating a failed gate) leaves the baseline intact.
// A clean frame after a run of partials advances the baseline (so later
// partials diff against the fresher authoritative read).
#[test]
fn clean_frame_after_partials_advances_baseline() {
    let mut tracker = CompactDiplomacyIntelOperativesLiveTracker::default();
    apply(rows(vec![], vec![op(1, 2, 5)]), CLEAN, &mut tracker);
    apply(rows(vec![], vec![op(1, 2, 9)]), PARTIAL, &mut tracker);
    assert_eq!(
        tracker.last_emitted.opinions.get(&(1, 2)).unwrap().opinion,
        5
    );

    // A non-first clean frame is a plain DELTA (delta vs baseline, no
    // PARTIAL bit) and DOES advance the baseline (unlike a partial).
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

// Full emit mode is untouched by the partial-delta change: it clears the
// baseline and returns the full read even on a partial frame.
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
