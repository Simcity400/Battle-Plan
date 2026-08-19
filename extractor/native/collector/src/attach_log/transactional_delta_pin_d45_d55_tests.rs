//! Gates for the TRANSACTIONAL delta pin on d45 and d55.
//!
//! # The defect
//!
//! Both domains advance a cross-frame delta pin while building rows, INSIDE the
//! reader's clock bracket, and both can then be badged failure-class by
//! hook-reservation custody that settles on the main thread AFTER that bracket
//! has closed (`D45DirectBatchCustodyConflict`,
//! `D55DetailEventCustodyConflict`). The consumer discards a failure-class
//! record whole — d55's fold clears the hour's detail events and returns; d45's
//! fold refuses the record outright once the direct source is downgraded after
//! activation. So the producer's pin had advanced to a baseline the fold never
//! applied, and every later delta patched against a baseline that did not exist
//! downstream.
//!
//! It happens on ORDINARY frames, not only rejected ones. The blanket
//! rejected-bracket reset was masking it by accident, and lane BN's narrowing
//! removed the accident — which is why BN excluded both domains from its
//! allowlist rather than inherit the bug.
//!
//! # Why it cannot be repaired after the fact
//!
//! A keyframe does not resynchronise. Once a domain has a delta baseline,
//! `replace_or_sparse_after_first` makes every later record — keyframes
//! included — a SPARSE patch, so an omitted row is retained and only an explicit
//! tombstone removes a key. Either the pin never diverges, or the stream is
//! wrong for its whole life.
//!
//! # What is pinned here
//!
//! 1. The stage is INVISIBLE to a reader: staging changes when a value becomes
//!    durable, never what the next read derives against inside its own frame.
//! 2. A rolled-back frame leaves the producer deriving from EXACTLY the baseline
//!    a consumer that applied nothing still holds — proven by equivalence
//!    against a run in which the discarded frame never happened at all.
//! 3. The commit is guarded by the SHARED contract predicate, at both sites and
//!    only at those sites, so a producer and its fold cannot drift apart.
//! 4. A d55 rebaseline that is rolled back stays PENDING, so the reset is
//!    announced again rather than believed on the consumer's behalf.

use super::*;

// ---------------------------------------------------------------------
// 1. The stage is invisible to a reader
// ---------------------------------------------------------------------

/// STAGING CHANGES DURABILITY, NOT VISIBILITY.
///
/// The d45 readers insert one value per key per frame and read the previous
/// value back through the same map. If the stage were write-only, a second row
/// for one key inside a frame would suddenly derive against the PRE-FRAME value
/// instead of the one just written, silently changing what the producer emits.
///
/// MUTATION RUN: change `StagedDeltaPin::get` to `self.committed.get(key)`.
/// Observed failure:
///   `assertion `left == right` failed: a staged successor must be what the
///    next read in the same frame derives against
///     left: Some(1)
///    right: Some(2)`.
/// (Verbatim, run 2026-08-05.)
#[test]
fn a_staged_successor_is_visible_to_the_rest_of_its_own_frame() {
    let mut pin: StagedDeltaPin<u32, u32> = StagedDeltaPin::default();
    pin.stage(7, 1);
    pin.commit();
    assert_eq!(pin.get(&7).copied(), Some(1));

    pin.stage(7, 2);
    assert_eq!(
        pin.get(&7).copied(),
        Some(2),
        "a staged successor must be what the next read in the same frame derives against"
    );

    // And a roll-back puts the reader back on the committed baseline.
    pin.roll_back();
    assert_eq!(pin.get(&7).copied(), Some(1));
    assert!(!pin.has_staged());
}

/// COMMIT INSTALLS, ROLL-BACK DISCARDS, AND NEITHER TOUCHES AN UNRELATED KEY.
///
/// MUTATION RUN: make `StagedDeltaPin::roll_back` call `self.commit()`.
/// Observed failure:
///   `assertion `left == right` failed: a rolled-back frame must leave the
///    baseline exactly where the consumer's is
///     left: Some(9)
///    right: Some(1)`.
#[test]
fn commit_installs_and_roll_back_leaves_the_consumers_baseline() {
    let mut pin: StagedDeltaPin<u32, u32> = StagedDeltaPin::default();
    pin.stage(1, 1);
    pin.stage(2, 2);
    pin.commit();

    pin.stage(1, 9);
    pin.roll_back();
    assert_eq!(
        pin.get(&1).copied(),
        Some(1),
        "a rolled-back frame must leave the baseline exactly where the consumer's is"
    );
    assert_eq!(pin.get(&2).copied(), Some(2), "and must not disturb its neighbours");

    pin.stage(1, 9);
    pin.commit();
    assert_eq!(pin.get(&1).copied(), Some(9));
    assert_eq!(pin.get(&2).copied(), Some(2));
}

// ---------------------------------------------------------------------
// 2. The equivalence that IS the acceptance bar
// ---------------------------------------------------------------------

struct FixedLandCombatMemory(Vec<land_detail55::LandCombatSnapshot>);

impl land_detail55::LandCombatDetailMemory for FixedLandCombatMemory {
    fn land_combat_snapshots(
        &self,
    ) -> Result<Vec<land_detail55::LandCombatSnapshot>, Vec<String>> {
        Ok(self.0.clone())
    }
}

fn land_combat_snapshot(
    combat_id: u32,
    day: u32,
    attacker_divisions: Vec<u32>,
) -> land_detail55::LandCombatSnapshot {
    land_detail55::LandCombatSnapshot {
        combat_id,
        generation: 1,
        location: Some(3236),
        day: Some(day),
        duration: Some(day),
        attacker_divisions,
        defender_divisions: vec![201],
        attacker_detail: None,
        defender_detail: None,
        attacker_attribution: Vec::new(),
        defender_attribution: Vec::new(),
        attacker_reserve_divisions: None,
        defender_reserve_divisions: None,
    }
}

/// THE PRODUCER'S PIN NEVER RUNS AHEAD OF WHAT THE FOLD APPLIED.
///
/// Stated as an equivalence rather than as an assertion about a private field,
/// because the field is not what matters — what matters is that the NEXT frame's
/// rows are exactly the rows a producer would emit to a consumer that applied
/// nothing on the discarded frame.
///
/// Two runs, same starting baseline:
///
///  * `withheld` — frame A is built and its record is then badged failure-class
///    by late custody, so the pin rolls back; frame B follows.
///  * `never_happened` — frame A is never built at all; frame B follows.
///
/// A consumer that discarded frame A whole is in the SAME state in both runs, so
/// the producer must be too, and frame B's transitions must be byte-identical.
/// Under the old unconditional advance they are not: the withheld run derives
/// B against A's snapshot, so every change A introduced is lost forever — no
/// keyframe can bring it back, because after a baseline exists every record is a
/// sparse patch.
///
/// MUTATION RUN: make `roll_back_staged_baseline` commit instead of
/// discarding, which is what an unguarded advance amounts to. Observed failure:
///   `assertion `left == right` failed: a frame the consumer discarded must
///    leave the producer deriving from the baseline the consumer still holds
///     left: [DetailEventRow { combat_id: 1, generation: 1, event_kind: 2, ...
///            new_side_count: 3, flags: 39 }, DetailEventRow { combat_id: 1,
///            generation: 1, event_kind: 4, side: Some(Attacker),
///            division_id: Some(102), ... }]
///    right: [DetailEventRow { combat_id: 1, generation: 1, event_kind: 1,
///            ... new_side_count: 3, flags: 39 }]`.
///
/// Read that failure carefully: the withheld run emitted event kinds 2 and 4
/// (an UPDATE and a division JOIN, derived against frame A's snapshot) where a
/// consumer that never received frame A needed kind 1, the CREATE. The create
/// is gone for good -- no later record reintroduces it.
#[test]
fn a_discarded_d55_frame_leaves_the_producer_on_the_consumers_baseline() {
    let frame_a = FixedLandCombatMemory(vec![land_combat_snapshot(1, 60, vec![101])]);
    let frame_b = FixedLandCombatMemory(vec![land_combat_snapshot(1, 61, vec![101, 102])]);

    // Run 1: frame A is built, then WITHHELD by late custody.
    let mut withheld = land_detail55::LandCombatDetailTracker::default();
    land_detail55::collect_transition_capture_frame(&frame_a, &mut withheld, 0, 60_787_539, 1)
        .expect("frame A collects");
    // This is the late custody decision, spelled through the SAME contract
    // function the production emission site uses. The status word is the one
    // `D55DetailEventCustodyConflict` derives: the read did not complete and its
    // answer is incomplete.
    let custody_failed = COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA;
    assert!(
        !compact_contract::compact_delta_pin_may_advance(
            COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
            custody_failed,
        ),
        "a custody-conflict word is one the consumer discards"
    );
    withheld.roll_back_staged_baseline();
    let after_withheld =
        land_detail55::collect_transition_capture_frame(&frame_b, &mut withheld, 0, 60_787_540, 1)
            .expect("frame B collects");

    // Run 2: frame A never happened.
    let mut never_happened = land_detail55::LandCombatDetailTracker::default();
    let after_never =
        land_detail55::collect_transition_capture_frame(
            &frame_b,
            &mut never_happened,
            0,
            60_787_540,
            1,
        )
        .expect("frame B collects");

    assert_eq!(
        after_withheld.rows, after_never.rows,
        "a frame the consumer discarded must leave the producer deriving from the baseline \
         the consumer still holds"
    );

    // The control that stops the assertion above from passing trivially: a
    // COMMITTED frame A really does change what frame B emits, so the two runs
    // are distinguishable and the equivalence is a fact about the roll-back and
    // not about the fixture.
    let mut committed = land_detail55::LandCombatDetailTracker::default();
    land_detail55::collect_transition_capture_frame(&frame_a, &mut committed, 0, 60_787_539, 1)
        .expect("frame A collects");
    committed.commit_staged_baseline();
    let after_committed =
        land_detail55::collect_transition_capture_frame(&frame_b, &mut committed, 0, 60_787_540, 1)
            .expect("frame B collects");
    assert_ne!(
        after_committed.rows, after_never.rows,
        "committing frame A must change frame B, or this fixture proves nothing"
    );
}

/// The same equivalence for D45, whose pin is three keyed maps rather than one
/// snapshot vector, and whose events are derived per country tag.
///
/// MUTATION RUN: make `StagedDeltaPin::roll_back` commit instead of clearing.
/// Observed failure:
///   `assertion `left == right` failed: a discarded D45 frame must leave the
///    producer deriving from the baseline the consumer still holds
///     left: 2
///    right: 0`.
///
/// The withheld run derived two focus events against frame A's pin; a consumer
/// that discarded frame A needed the zero events a fresh baseline produces,
/// and would have decoded the two it got against a state it never held.
#[test]
fn a_discarded_d45_frame_leaves_the_producer_on_the_consumers_baseline() {
    fn focus(completed: u32, current: &str) -> CountryFocusRow {
        CountryFocusRow {
            focus_tree: None,
            completed: None,
            completed_count: Some(completed),
            current: Some(current.to_string()),
            current_set_measured: true,
            current_continuous: None,
            current_continuous_set_measured: false,
            progress: None,
            paused: None,
        }
    }

    // An empty read, borrowed from the panic constructor so this test cannot
    // drift out of shape when the read gains a field.
    fn empty_read() -> CompactFrdDepthRead {
        let mut read = compact_frd_depth_worker_panic_read(0);
        read.status_flags = 0;
        read
    }

    let row = super::tests::test_country_row("GER", 7, 0x1000);
    let (row_a, focus_a) = (row.clone(), focus(1, "FOCUS_A"));
    let (row_b, focus_b) = (row, focus(3, "FOCUS_B"));

    // Run 1: frame A built, then withheld.
    let mut withheld = CompactFrdDepthTracker::default();
    let mut read_a = empty_read();
    compact_frd_apply_focus_row(&mut read_a, &mut withheld, 60_787_539, &row_a, &focus_a);
    withheld.roll_back_staged_pins();
    let mut read_b_withheld = empty_read();
    compact_frd_apply_focus_row(
        &mut read_b_withheld,
        &mut withheld,
        60_787_540,
        &row_b,
        &focus_b,
    );

    // Run 2: frame A never happened.
    let mut never_happened = CompactFrdDepthTracker::default();
    let mut read_b_never = empty_read();
    compact_frd_apply_focus_row(
        &mut read_b_never,
        &mut never_happened,
        60_787_540,
        &row_b,
        &focus_b,
    );

    assert_eq!(
        read_b_withheld.focus_events.len(),
        read_b_never.focus_events.len(),
        "a discarded D45 frame must leave the producer deriving from the baseline the \
         consumer still holds"
    );

    // Control: committing frame A really does change frame B.
    let mut committed = CompactFrdDepthTracker::default();
    let mut read_a2 = empty_read();
    compact_frd_apply_focus_row(&mut read_a2, &mut committed, 60_787_539, &row_a, &focus_a);
    committed.commit_staged_pins();
    let mut read_b_committed = empty_read();
    compact_frd_apply_focus_row(
        &mut read_b_committed,
        &mut committed,
        60_787_540,
        &row_b,
        &focus_b,
    );
    assert_ne!(
        read_b_committed.focus_events.len(),
        read_b_never.focus_events.len(),
        "committing frame A must change frame B, or this fixture proves nothing"
    );
}

/// A ROLLBACK IS AS DANGEROUS AS AN UNCONDITIONAL COMMIT, IN THE OTHER
/// DIRECTION.
///
/// If the producer rolls back a frame the FOLD APPLIED, the producer is BEHIND:
/// the fold holds B, the producer holds A. The next time the state returns to A
/// the producer compares A to A, emits nothing, and the fold keeps B forever.
/// No keyframe repairs it, for the same reason nothing repairs the other
/// direction.
///
/// D45 reaches exactly that shape on ordinary frames: it sets `PARTIAL_DATA`
/// for an unrelated targeted-decision omission while every focus row in the
/// same record stays valid, and its fold applies them. This pins the sequence
/// end to end.
///
/// MUTATION RUN: make `StagedDeltaPin::roll_back` commit instead of clearing --
/// which is what any guard that rolls back an APPLIED frame amounts to.
/// Observed failure:
///   `assertion `left != right` failed: a frame the fold APPLIED must leave the
///    producer's pin where the fold's is; rolling it back loses the next
///    reversion for good
///     left: 1
///    right: 1`.
///
/// The companion property -- that production does not USE the status word for
/// D45 -- is pinned structurally by
/// `every_pin_commit_is_guarded_by_the_predicate_its_own_fold_uses`, whose
/// mutation run swaps the guard back and fails with
/// `commit_staged_pins() is called but is not guarded by
/// d45_late_custody_conflict`.
#[test]
fn an_ordinary_d45_partial_does_not_roll_the_pin_back_behind_the_fold() {
    fn focus(completed: u32, current: &str) -> CountryFocusRow {
        CountryFocusRow {
            focus_tree: None,
            completed: None,
            completed_count: Some(completed),
            current: Some(current.to_string()),
            current_set_measured: true,
            current_continuous: None,
            current_continuous_set_measured: false,
            progress: None,
            paused: None,
        }
    }
    fn empty_read() -> CompactFrdDepthRead {
        let mut read = compact_frd_depth_worker_panic_read(0);
        read.status_flags = 0;
        read
    }

    // The word D45 publishes on an ordinary frame whose targeted-decision read
    // was partial. `compact_delta_pin_may_advance` says do not advance...
    let ordinary_partial = COMPACT_STATUS_DELTA_DATA | COMPACT_STATUS_PARTIAL_DATA;
    assert!(
        !compact_contract::compact_delta_pin_may_advance(
            COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
            ordinary_partial,
        ),
        "the shared predicate refuses this word, which is why D45 must not use it"
    );

    // ...but `fold_focus_research_decisions_depth` never consults the status
    // word: it applies the focus rows regardless. So the producer must commit.
    let fold = super::tests::collector_source_lf();
    let _ = fold;

    let row = super::tests::test_country_row("GER", 7, 0x1000);
    let a = focus(1, "FOCUS_A");
    let b = focus(1, "FOCUS_B");

    // Frame 1 observes A -> B and the fold applies it. Frame 2 sees the focus
    // revert to A.
    let mut committed = CompactFrdDepthTracker::default();
    let mut read1 = empty_read();
    compact_frd_apply_focus_row(&mut read1, &mut committed, 60_787_539, &row, &a);
    committed.commit_staged_pins();
    let mut read2 = empty_read();
    compact_frd_apply_focus_row(&mut read2, &mut committed, 60_787_540, &row, &b);
    committed.commit_staged_pins();
    let mut read3 = empty_read();
    compact_frd_apply_focus_row(&mut read3, &mut committed, 60_787_541, &row, &a);
    let reversion_seen = read3.focus_events.len();
    assert_ne!(
        reversion_seen, 0,
        "the fixture must actually produce a reversion event, or it proves nothing"
    );

    // The same three frames, with frame 2 rolled back as a status-word guard
    // would have done. The reversion is now invisible: the producer compares A
    // to A and says nothing, while a fold that applied frame 2 still holds B.
    let mut rolled_back = CompactFrdDepthTracker::default();
    let mut read1 = empty_read();
    compact_frd_apply_focus_row(&mut read1, &mut rolled_back, 60_787_539, &row, &a);
    rolled_back.commit_staged_pins();
    let mut read2 = empty_read();
    compact_frd_apply_focus_row(&mut read2, &mut rolled_back, 60_787_540, &row, &b);
    rolled_back.roll_back_staged_pins();
    let mut read3 = empty_read();
    compact_frd_apply_focus_row(&mut read3, &mut rolled_back, 60_787_541, &row, &a);
    assert_ne!(
        read3.focus_events.len(),
        reversion_seen,
        "a frame the fold APPLIED must leave the producer's pin where the fold's is; \
         rolling it back loses the next reversion for good"
    );
    assert_eq!(
        read3.focus_events.len(),
        0,
        "and the shape of the loss is silence, which is why no later record repairs it"
    );
}

/// WHETHER THE FOLD APPLIES A D45 RECORD IS NOT A PROPERTY OF THE RECORD.
///
/// `fold_focus_research_decisions_depth` refuses a record with no validated
/// direct trailer ONLY once it has already accepted one
/// (`frd_direct_baseline_ready`). Before that it applies the record and ignores
/// the missing trailer. So the same bytes are applied in one stream position and
/// refused in another, and a producer predicate that looks only at the record --
/// "did custody conflict this frame?" -- is wrong in the pre-activation case:
/// the fold APPLIES, the producer rolls back, and the producer falls BEHIND.
///
/// The producer therefore mirrors the two pieces of fold state that decide it.
/// This pins the mirror against the fold's three regimes.
///
/// MUTATION RUN: drop `d45_tracker.direct_baseline_published` from
/// `d45_fold_refuses_this_record`, leaving the bare custody test. Observed
/// failure:
///   `before the fold has ever accepted a validated source it APPLIES an
///    untrailered record, so the producer must commit`.
#[test]
fn the_d45_pin_predicate_mirrors_the_folds_activation_state() {
    let mut tracker = CompactFrdDepthTracker::default();

    // PRODUCTION'S predicate, not a paraphrase. A local copy passed this test
    // while production followed a different rule, which is the failure mode a
    // mirror test is most likely to have.
    use super::d45_fold_refuses_record as fold_refuses;

    // REGIME 1 -- before activation. The fold has never seen a validated direct
    // source, so it applies an untrailered record and ignores the absence. A
    // custody conflict here produces exactly such a record.
    assert!(
        !fold_refuses(&tracker, false, false),
        "before the fold has ever accepted a validated source it APPLIES an untrailered \
         record, so the producer must commit"
    );

    // REGIME 2 -- activation. A validated record is applied and arms the fold.
    assert!(!fold_refuses(&tracker, true, false));
    tracker.direct_baseline_published = true;

    // REGIME 3 -- after activation, the same untrailered record is REFUSED
    // (`direct hook source downgraded after activation`).
    assert!(
        fold_refuses(&tracker, false, false),
        "after activation a downgraded record is refused whole, so the pin must not advance"
    );

    // And the refusal is ABSORBING on both arms: a validated record afterwards
    // hits `already invalidated`, an untrailered one hits the downgrade again.
    tracker.fold_history_failed = true;
    assert!(fold_refuses(&tracker, true, false));
    assert!(fold_refuses(&tracker, false, false));

    // REGIME 4 -- the direct-history reset badge, which is the ONLY thing that ends the
    // absorbing regime above. The fold discards its `frd_direct_*` extent
    // before it asks either question, so both grounds are answered at once and
    // the record is applied whatever it carries.
    assert!(
        !fold_refuses(&tracker, true, true),
        "a badged validated record restarts the history and is applied"
    );
    assert!(
        !fold_refuses(&tracker, false, true),
        "and so is a badged untrailered one: after the reset there is no baseline left for it \
         to have downgraded, which is why the badge is not gated on the trailer"
    );
}

/// A RECORD THAT NEVER REACHED THE FOLD MUST TEACH THE MIRROR NOTHING.
///
/// The mirror is read and staged at D45's emission site, which runs BEFORE
/// bracket rejection is known. A rejected D45 bracket then has its records
/// REPLACED WITH A GAP: the fold receives no D45 record for that hour and
/// learns nothing from it.
///
/// Arming the mirror there was a real divergence, in the direction that loses
/// data. An unvalidated record after activation would set `fold_history_failed`
/// on a frame whose record was withdrawn, the fold would never fail, and the
/// producer would then roll back the NEXT record — one the fold applies —
/// leaving it permanently behind.
///
/// MUTATION RUN: drop the `d45_published_its_record` condition from the
/// frame-end resolution, committing the staged mirror unconditionally. Observed
/// failure:
///   `a record the frame replaced with a gap never reached the fold, so it must
///    not arm the producer's mirror of the fold`.
#[test]
fn a_withdrawn_d45_record_does_not_arm_the_fold_mirror() {
    // What the emission site stages when the fold would refuse the record.
    let mut tracker = CompactFrdDepthTracker::default();
    tracker.direct_baseline_published = true;
    assert!(
        super::d45_fold_refuses_record(&tracker, false, false),
        "an untrailered record after activation is refused, so this frame stages a failure"
    );
    tracker.staged_mirror_fails_history = true;

    // The frame then rejects D45's bracket, so its records are replaced with a
    // gap. Modelled exactly as the frame-end resolution spells it.
    let d45_published_its_record = false;
    if d45_published_its_record && tracker.staged_mirror_fails_history {
        tracker.fold_history_failed = true;
    }
    let staged_was_discarded = !tracker.fold_history_failed;
    tracker.staged_mirror_fails_history = false;

    assert!(
        staged_was_discarded,
        "a record the frame replaced with a gap never reached the fold, so it must not arm \
         the producer's mirror of the fold"
    );

    // BUT THE HOUR STILL KILLED THE HISTORY, for a reason that has nothing to
    // do with the withdrawn record. `world_fold::reset_hour_state` fails the
    // direct history whenever the fold was `frd_direct_baseline_ready` and no
    // D45 record arrived that hour, and a gap IS no record arriving. The seal
    // arms the mirror from that, separately from the staged half above.
    //
    // This used to read "the next validated record commits, exactly as the fold
    // applies it", which was false about the fold: after a gapped hour the fold
    // refuses the next record with `already invalidated`. The producer believed
    // otherwise and kept committing pins against a fold that was applying
    // nothing -- the AHEAD divergence, arrived at from the gap side.
    // The t26 capture measured 362 such gaps.
    if !d45_published_its_record && tracker.direct_baseline_published {
        tracker.fold_history_failed = true;
    }
    assert!(
        tracker.fold_history_failed,
        "a gapped D45 hour after activation kills the fold's direct history, so the mirror \
         must say so -- this is the arming the staged half above is deliberately NOT doing"
    );
    assert!(
        super::d45_fold_refuses_record(&tracker, true, false),
        "and the consequence: even a VALIDATED next record is refused by the fold, so the \
         producer must not commit a pin against it"
    );
    assert!(
        !super::d45_fold_refuses_record(&tracker, true, true),
        "the direct-history reset badge is the one record that reopens the domain"
    );

    // The control: a frame that DID publish arms it, or the assertion above
    // would pass under a mirror that never arms at all.
    let mut published = CompactFrdDepthTracker::default();
    published.direct_baseline_published = true;
    published.staged_mirror_fails_history = true;
    let d45_published_its_record = true;
    if d45_published_its_record && published.staged_mirror_fails_history {
        published.fold_history_failed = true;
    }
    assert!(
        published.fold_history_failed,
        "a published refusal must arm the mirror, or the mirror is inert"
    );
}

/// THE MIRROR MUST SURVIVE THE REJECTED-BRACKET RESET.
///
/// The two mirror flags record what this STREAM has already published, and no
/// reset can unpublish it. Dropping them makes the producer believe the fold had
/// never activated, and it then commits a pin on the next record the fold
/// refuses -- the ahead-divergence, reintroduced by a reset.
///
/// The second half of this test used to read "the PIN is still dropped by the
/// reset". It no longer is: d45 takes PRE-FRAME RESTORATION, so its own
/// rejection UNDOES this frame's commits rather than wiping the pin. The
/// assertion below is unchanged in form and stronger in meaning -- the key
/// committed inside this frame is gone because the restoration undid it, which
/// is also what proves the reset is not a no-op.
///
/// MUTATION RUN: delete the two `direct_baseline_published` / `fold_history
/// _failed` carry-overs from `reset_trackers_after_rejected_bracket`. Observed
/// failure:
///   `the fold cannot un-activate, so a producer reset must not forget that it
///    did`.
#[test]
fn the_d45_fold_mirror_is_carried_across_a_rejected_bracket() {
    let mut trackers = CompactPerfGateTrackers::default();
    trackers
        .focus_research_decisions_depth_live
        .direct_baseline_published = true;
    trackers.focus_research_decisions_depth_live.fold_history_failed = true;
    // A pin the reset SHOULD drop, so this test cannot pass by the reset having
    // become a no-op.
    trackers
        .focus_research_decisions_depth_live
        .focus_by_tag
        .stage(7, CompactFrdFocusState {
            tag_db_id: 7,
            completed_count: Some(1),
            progress_raw: None,
            current: None,
            current_continuous: None,
            completed_tokens: Vec::new(),
            paused: None,
        });
    trackers
        .focus_research_decisions_depth_live
        .focus_by_tag
        .commit();

    let rejected: BTreeSet<u16> = [COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE]
        .into_iter()
        .collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);

    assert!(
        trackers
            .focus_research_decisions_depth_live
            .direct_baseline_published,
        "the fold cannot un-activate, so a producer reset must not forget that it did"
    );
    assert!(
        trackers.focus_research_decisions_depth_live.fold_history_failed,
        "nor can a fold un-fail; the refusal is absorbing for the life of the stream"
    );
    assert!(
        trackers
            .focus_research_decisions_depth_live
            .focus_by_tag
            .get(&7)
            .is_none(),
        "the commit this frame made must be UNDONE by d45's own rejection, or this test proves \
         nothing about scoping"
    );
}

// ---------------------------------------------------------------------
// 3. A rolled-back rebaseline stays pending
// ---------------------------------------------------------------------

/// A REBASELINE THE CONSUMER NEVER SAW IS STILL OWED.
///
/// d55 announces a transition-baseline reset with
/// `TRANSITION_BASELINE_RESET | PARTIAL_DATA`, and the fold APPLIES that record.
/// But a rebaseline frame that ALSO loses hook custody carries extra failure
/// bits, and the fold then discards it like any other — so the reset it
/// announced never happened downstream. Clearing `pending_rebaseline` on that
/// frame would leave the producer emitting deltas against a baseline the
/// consumer never established.
///
/// MUTATION RUN: make `LandCombatDetailTracker::roll_back_staged_baseline`
/// commit instead of discarding — which is what an unguarded advance amounts
/// to. Observed failure:
///   `the rebaseline must be announced again after the consumer discarded it`.
#[test]
fn a_rolled_back_rebaseline_is_announced_again() {
    let frame = FixedLandCombatMemory(vec![land_combat_snapshot(1, 60, vec![101])]);

    // The clean rebaseline word IS applied by the fold, so it commits.
    let rebaseline_only =
        COMPACT_STATUS_DELTA_DATA | COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_TRANSITION_BASELINE_RESET;
    assert!(
        compact_contract::compact_delta_pin_may_advance(
            COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
            rebaseline_only,
        ),
        "the fold applies a plain rebaseline, so refusing to commit it would loop forever"
    );

    // A rebaseline that also lost custody is discarded whole.
    let rebaseline_and_custody_failure = rebaseline_only | COMPACT_STATUS_SOURCE_UNAVAILABLE;
    assert!(
        !compact_contract::compact_delta_pin_may_advance(
            COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
            rebaseline_and_custody_failure,
        ),
        "a rebaseline that also failed is not a tolerated word"
    );

    let mut tracker = land_detail55::LandCombatDetailTracker::default();
    tracker.reset_transition_baseline();
    let first =
        land_detail55::collect_transition_capture_frame(&frame, &mut tracker, 0, 60_787_539, 1)
            .expect("rebaseline frame collects");
    assert!(first.rows.is_empty(), "a rebaseline emits no synthetic events");
    assert!(
        tracker.last_frame_rebaselined(),
        "and the reader can see the reset it just performed"
    );
    tracker.roll_back_staged_baseline();

    // Still owed: the next frame rebaselines again rather than assuming the
    // consumer took the first one.
    let second =
        land_detail55::collect_transition_capture_frame(&frame, &mut tracker, 0, 60_787_540, 1)
            .expect("second frame collects");
    assert!(
        tracker.last_frame_rebaselined(),
        "the rebaseline must be announced again after the consumer discarded it"
    );
    assert!(second.rows.is_empty());

    // And once it IS committed, the domain leaves rebaseline mode.
    tracker.commit_staged_baseline();
    let third =
        land_detail55::collect_transition_capture_frame(&frame, &mut tracker, 0, 60_787_541, 1)
            .expect("third frame collects");
    assert!(
        !tracker.last_frame_rebaselined(),
        "a committed rebaseline is not owed twice"
    );
    let _ = third;
}

// ---------------------------------------------------------------------
// 4. The guard is the SHARED predicate, at both sites and only there
// ---------------------------------------------------------------------

/// EACH PIN COMMIT IS GUARDED, AND BY THE PREDICATE ITS OWN FOLD USES.
///
/// The two domains need DIFFERENT predicates, and getting that wrong is a
/// defect in both directions:
///
///  * D55's fold discards on the status word, so the status word is the right
///    question and both sides ask it through `compact_delta_pin_may_advance`.
///  * D45's fold does NOT consult the status word. It applies the focus,
///    research and decision rows whatever `PARTIAL_DATA` says — and D45 sets
///    `PARTIAL_DATA` routinely for an unrelated targeted-decision omission. A
///    status-word guard there rolled the pin back on ordinary frames the fold
///    had APPLIED, leaving the producer BEHIND the fold, which loses a change
///    for good the moment the state returns to the producer's stale value.
///    Its guard is `d45_fold_refuses_this_record`, which mirrors the fold's own
///    rule: a record without a validated direct trailer is refused only once a
///    validated one has ALREADY been accepted, and the refusal is absorbing.
///
/// Written against this crate's own source text so an edit that reintroduces an
/// unconditional advance — the regression this lane exists to prevent — fails a
/// test rather than shipping.
///
/// MUTATION RUN: delete the `if d45_fold_refuses_this_record` guard around the
/// D45 arms, leaving `commit_staged_pins()` unconditional. Observed failure:
///   `commit_staged_pins() is called but is not guarded by
///    d45_fold_refuses_this_record`.
#[test]
fn every_pin_commit_is_guarded_by_the_predicate_its_own_fold_uses() {
    let source = super::tests::collector_source_lf();

    for (commit, roll_back, guard) in [
        (
            "commit_staged_pins()",
            "roll_back_staged_pins()",
            // THE GUARD STATEMENT, not the identifier. A test that only asked
            // whether the name appeared nearby passed a mutation that replaced
            // the condition with `if false` and left the binding standing.
            "if d45_fold_refuses_this_record {",
        ),
        (
            "commit_staged_baseline()",
            "roll_back_staged_baseline()",
            "if compact_contract::compact_delta_pin_may_advance(",
        ),
    ] {
        let commits = source.matches(commit).count();
        assert_eq!(
            commits, 1,
            "{commit} must have exactly one call site in production; found {commits}"
        );

        // The guard and its two arms, as one shape. `find` on the guard and then
        // a bounded look-behind is what makes "guarded" mean adjacency rather
        // than mere co-presence in a 100k-line file.
        let at = source
            .find(commit)
            .expect("the commit call site was just counted");
        let window_start = at.saturating_sub(900);
        let window = &source[window_start..at];
        assert!(
            window.contains(guard),
            "{commit} is called but is not guarded by {guard}"
        );

        // And the other arm exists: a guard with no roll-back would leave the
        // stage alive into the next frame, which is the same divergence from
        // the other direction.
        let around = &source[window_start..(at + 900).min(source.len())];
        assert!(
            around.contains(roll_back),
            "{commit} has no {roll_back} arm; an unconsumed stage would survive the frame"
        );
    }

    // The FRAME-END MIRROR RESOLUTION is guarded too, and by D45's own
    // publication rather than by anything the emission site knew. A record the
    // frame replaced with a gap never reached the fold, so it must teach the
    // mirror nothing -- and the emission site cannot know that yet.
    // The gate function must resolve the mirrors from D45's OWN publication and
    // from nothing else. The transitions themselves live in
    // `CompactFrdDepthTracker::resolve_frame_seal`, which is production and is
    // exercised directly by `the_d45_frame_seal_is_the_reset_debts_whole_life`;
    // what is checked HERE is that the gate function still calls it, and still
    // computes the argument from the rejected set.
    let resolution = source
        .find("let d45_published_its_record =")
        .expect("the frame-end mirror resolution");
    let resolution_body = {
        let rest = &source[resolution..];
        let end = rest
            .find("\n    append_compact_frame_end_record(")
            .expect("the mirror resolution precedes the frame-end record");
        &rest[..end]
    };
    assert!(
        resolution_body
            .contains("!rejected_domains.contains(&COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE)"),
        "D45's publication must be read from the rejected set at the seal; the emission site \
         cannot know it yet"
    );
    assert!(
        resolution_body.contains(".resolve_frame_seal(d45_published_its_record);"),
        "the seal must be resolved through the production transition, and with THIS frame's \
         publication as its argument"
    );

    // D45's guard must NOT be the status word. This is the P1 regression stated
    // as a property: a status-word guard on D45 rolls back records its fold
    // applied.
    let d45_at = source
        .find("commit_staged_pins()")
        .expect("the D45 commit site");
    let d45_window = &source[d45_at.saturating_sub(900)..d45_at];
    assert!(
        !d45_window.contains("compact_delta_pin_may_advance("),
        "D45's commit must not be gated on the status word: its fold applies a PARTIAL_DATA \
         record, so a status-word guard leaves the producer BEHIND the fold"
    );

    // The frame-end backstop: every path out of the builder discards whatever is
    // still staged, so no stage can cross a frame boundary even on a path that
    // never reaches an emission block.
    assert_eq!(
        source.matches("roll_back_staged_pins()").count(),
        2,
        "D45 needs its emission-site roll-back AND the frame-end backstop"
    );
    assert_eq!(
        source.matches("roll_back_staged_baseline()").count(),
        2,
        "D55 needs its emission-site roll-back AND the frame-end backstop"
    );
}

/// The contract predicate itself: a clean word advances, a failure word does
/// not, and the two documented tolerances are the only exceptions.
///
/// MUTATION RUN: make `compact_delta_pin_may_advance` return `true`
/// unconditionally. Observed failure:
///   `a custody-conflict word must not advance a pin`.
#[test]
fn the_pin_predicate_matches_what_the_folds_actually_apply() {
    use compact_contract::compact_delta_pin_may_advance as may_advance;

    assert!(may_advance(COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE, 0));
    assert!(may_advance(
        COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
        COMPACT_STATUS_DELTA_DATA
    ));
    assert!(
        !may_advance(
            COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
            COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA
        ),
        "a custody-conflict word must not advance a pin"
    );

    // d55's rebaseline is tolerated; d45 has no such tolerance, so the same bits
    // on d45 are still a discard. The exception is domain-scoped, not global.
    let rebaseline =
        COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_TRANSITION_BASELINE_RESET;
    assert!(may_advance(COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE, rebaseline));
    assert!(
        !may_advance(COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE, rebaseline),
        "the rebaseline tolerance belongs to d55 alone"
    );

    // An expected-partial domain's tolerated flag IS applied, as a sparse patch.
    assert!(
        may_advance(COMPACT_DOMAIN_ARMY_ORDERS, COMPACT_STATUS_PARTIAL_DATA),
        "an expected-partial record is applied sparsely, so its pin advances"
    );
    assert!(
        !may_advance(
            COMPACT_DOMAIN_ARMY_ORDERS,
            COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_WORKER_PANIC
        ),
        "and a tolerance never covers a second, untolerated failure"
    );
}

// ---------------------------------------------------------------------
// 5. PRE-FRAME RESTORATION: the domain's OWN rejection
// ---------------------------------------------------------------------
//
// `reset_trackers_after_rejected_bracket` used to wipe both trackers to
// `Default`. That is BEHIND the fold's retained state, and behind loses data
// exactly as ahead does: once a domain has a baseline the fold treats every
// later record as a sparse patch, so a pin restarted from empty emits a whole
// payload the fold MERGES and anything that vanished meanwhile is never
// tombstoned. An allowlist entry cannot fix it either -- the survival predicate
// is false for the domain's OWN rejection, which is precisely the case.
//
// The repair is restoration: undo this frame's commits, leave the pin where the
// frame found it, which is where the fold's mirror is when the frame's records
// were replaced with a `DOMAIN_GAP`.
//
// Every gate below is an EQUIVALENCE against a run in which the rejected frame
// never happened, because that run is by definition what the consumer holds.

/// The production frame seal, in order. Not a convenience wrapper: the order is
/// load-bearing (the reset restores, then the commit closes the window), and a
/// test that drove them the other way round would pass on code that cannot.
fn settle(trackers: &mut CompactPerfGateTrackers, rejected: &[u16]) {
    let rejected: BTreeSet<u16> = rejected.iter().copied().collect();
    if !rejected.is_empty() {
        reset_trackers_after_rejected_bracket(trackers, &rejected);
    }
    commit_delta_pin_transactions(trackers);
}

fn frd_focus(completed: u32, current: &str) -> CountryFocusRow {
    CountryFocusRow {
        focus_tree: None,
        completed: None,
        completed_count: Some(completed),
        current: Some(current.to_string()),
        current_set_measured: true,
        current_continuous: None,
        current_continuous_set_measured: false,
        progress: None,
        paused: None,
    }
}

fn frd_empty_read() -> CompactFrdDepthRead {
    let mut read = compact_frd_depth_worker_panic_read(0);
    read.status_flags = 0;
    read
}

/// One d45 frame: read the row, commit the pin as the emission site does, and
/// settle the frame with the given rejection set.
fn d45_frame(
    trackers: &mut CompactPerfGateTrackers,
    hour: i32,
    row: &CountryObjectRow,
    focus: &CountryFocusRow,
    rejected: &[u16],
) -> usize {
    let mut read = frd_empty_read();
    compact_frd_apply_focus_row(
        &mut read,
        &mut trackers.focus_research_decisions_depth_live,
        hour,
        row,
        focus,
    );
    // The emission site commits when the fold will apply the record. A frame
    // whose bracket is later torn still reaches this point -- that is the whole
    // reason restoration is needed.
    trackers
        .focus_research_decisions_depth_live
        .commit_staged_pins();
    settle(trackers, rejected);
    read.focus_events.len()
}

const D45: u16 = COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE;
const D55: u16 = COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE;
/// A domain whose bracket is unrelated to d45's and d55's.
const OTHER: u16 = COMPACT_DOMAIN_WEATHER;

/// **OWN GAP, THEN ACCEPT.** After d45's own bracket is rejected, the next
/// accepted frame must derive against the baseline the fold still holds — which
/// is the baseline of the run where the rejected frame never happened.
///
/// The pre-fix wipe put the pin at `Default` instead, so the next frame saw
/// every country as brand new and emitted a seeding record the fold then MERGED.
///
/// What production change makes this fail: wiping d45's tracker on its own
/// rejection instead of restoring it.
///
/// MUTATION RUN: `tracker.restore_pins_to_frame_entry()` replaced with
/// `tracker = CompactFrdDepthTracker::default()` -- the shipped wipe.
/// OBSERVED: FAILED --
/// "assertion `left == right` failed: after its own gap, d45 must derive against
/// the baseline the fold still holds
///   left: 0
///  right: 2"
///
/// MUTATION RUN: `close_pin_frame_transaction()` deleted from
/// `commit_delta_pin_transactions`, so the undo journal is cumulative.
/// OBSERVED: FAILED with the same assertion -- the rejection rewound past the
/// frame the fold had already applied, leaving the pin empty.
///
/// MUTATION RUN: the journal seeded with `self.staged.get(key).cloned()` instead
/// of `self.committed.get(key).cloned()`.
/// OBSERVED: FAILED with the same assertion.
#[test]
fn d45_restores_its_pin_after_its_own_bracket_is_rejected() {
    let row = super::tests::test_country_row("GER", 7, 0x1000);
    let a = frd_focus(1, "FOCUS_A");
    let b = frd_focus(2, "FOCUS_B");

    // The reference run: the middle frame never happened at all.
    let mut reference = CompactPerfGateTrackers::default();
    let _ = d45_frame(&mut reference, 60_787_539, &row, &a, &[]);
    let reference_events = d45_frame(&mut reference, 60_787_541, &row, &b, &[]);

    // The real run: the middle frame built and committed, then its own bracket
    // was rejected and its records became a `DOMAIN_GAP`.
    let mut gapped = CompactPerfGateTrackers::default();
    let _ = d45_frame(&mut gapped, 60_787_539, &row, &a, &[]);
    let _ = d45_frame(&mut gapped, 60_787_540, &row, &b, &[D45]);
    let gapped_events = d45_frame(&mut gapped, 60_787_541, &row, &b, &[]);

    assert_eq!(
        gapped_events, reference_events,
        "after its own gap, d45 must derive against the baseline the fold still holds"
    );
    // The control that stops this passing trivially: the middle frame really did
    // change the pin, so a run that KEEPS it emits something different.
    let mut kept = CompactPerfGateTrackers::default();
    let _ = d45_frame(&mut kept, 60_787_539, &row, &a, &[]);
    let _ = d45_frame(&mut kept, 60_787_540, &row, &b, &[]);
    let kept_events = d45_frame(&mut kept, 60_787_541, &row, &b, &[]);
    assert_ne!(
        kept_events, reference_events,
        "committing the middle frame must change the last one, or this fixture proves nothing"
    );
}

/// **DOUBLE REJECTION.** Two consecutive own rejections restore ONE frame each,
/// not the whole stream — the window is closed by
/// `commit_delta_pin_transactions` at every frame seal, so a rejection can never
/// reach back past a frame the fold applied.
///
/// This is the arm that catches an undo log that is never cleared: with a
/// cumulative log the second rejection would rewind past the first frame as
/// well, and the producer would fall arbitrarily far behind the fold.
///
/// What production change makes this fail: dropping
/// `close_pin_frame_transaction` from `commit_delta_pin_transactions`, or making
/// it conditional on the frame having rejected nothing.
///
/// MUTATION RUN: `close_pin_frame_transaction()` deleted from
/// `commit_delta_pin_transactions`.
/// OBSERVED: FAILED --
/// "assertion `left == right` failed: two rejections must rewind two frames, not
/// the whole stream: the frame the fold APPLIED is not reachable by any later
/// rejection
///   left: None
///  right: Some(Some(1))"
///
/// MUTATION RUN: the journal seeded from `staged` instead of `committed`.
/// OBSERVED: FAILED --
/// "  left: Some(Some(3))
///  right: Some(Some(1))"
/// -- the restoration became a no-op and the pin kept the last rejected frame's
/// value.
#[test]
fn two_consecutive_d45_rejections_rewind_one_frame_each() {
    let row = super::tests::test_country_row("GER", 7, 0x1000);
    let a = frd_focus(1, "FOCUS_A");
    let b = frd_focus(2, "FOCUS_B");
    let c = frd_focus(3, "FOCUS_C");

    // Reference: the two rejected frames never happened.
    let mut reference = CompactPerfGateTrackers::default();
    let _ = d45_frame(&mut reference, 60_787_539, &row, &a, &[]);

    let mut gapped = CompactPerfGateTrackers::default();
    let _ = d45_frame(&mut gapped, 60_787_539, &row, &a, &[]);
    let _ = d45_frame(&mut gapped, 60_787_540, &row, &b, &[D45]);
    let _ = d45_frame(&mut gapped, 60_787_541, &row, &c, &[D45]);

    // The assertion is on the PIN, not on an emission, because the emission
    // after a total rewind is indistinguishable from the one after a correct
    // rewind: `compact_frd_apply_focus_row` emits no event for a country it is
    // seeing for the first time, so an emptied pin looks quiet.
    let pinned = gapped
        .focus_research_decisions_depth_live
        .focus_by_tag
        .get(&7)
        .cloned();
    let reference_pin = reference
        .focus_research_decisions_depth_live
        .focus_by_tag
        .get(&7)
        .cloned();
    assert!(
        reference_pin.is_some(),
        "the reference must hold frame 1's pin, or this fixture proves nothing"
    );
    assert_eq!(
        pinned.as_ref().map(|state| state.completed_count),
        reference_pin.as_ref().map(|state| state.completed_count),
        "two rejections must rewind two frames, not the whole stream: the frame the fold \
         APPLIED is not reachable by any later rejection"
    );
}

/// **REJECTION DURING THE FIRST BASELINE.** The pin must go back to EMPTY, not
/// stay seeded: the fold has no baseline for a domain whose only record so far
/// became a gap, so a producer holding one would emit a patch against a state
/// that does not exist downstream.
///
/// `reject_delta_before_baseline` is the fold's own check for exactly this, and
/// it types the result as a domain-unknown hour — a cost the producer avoids
/// only by having no baseline either.
///
/// What production change makes this fail: seeding `frame_undo` with the
/// POST-commit value instead of the pre-frame one, which makes the restoration a
/// no-op on a first commit.
///
/// MUTATION RUN: `frame_undo` seeded with `self.staged.get(key).cloned()`.
/// OBSERVED: FAILED --
/// "a domain whose first and only record became a gap has no baseline
/// downstream, so it must hold none itself"
#[test]
fn a_d45_rejection_during_the_first_baseline_leaves_no_pin_at_all() {
    let row = super::tests::test_country_row("GER", 7, 0x1000);
    let a = frd_focus(1, "FOCUS_A");

    let mut gapped = CompactPerfGateTrackers::default();
    let _ = d45_frame(&mut gapped, 60_787_539, &row, &a, &[D45]);

    assert!(
        gapped
            .focus_research_decisions_depth_live
            .focus_by_tag
            .get(&7)
            .is_none(),
        "a domain whose first and only record became a gap has no baseline downstream, so it \
         must hold none itself"
    );

    // And the next accepted frame therefore seeds, exactly as a fresh stream does.
    let mut fresh = CompactPerfGateTrackers::default();
    let fresh_events = d45_frame(&mut fresh, 60_787_540, &row, &a, &[]);
    let gapped_events = d45_frame(&mut gapped, 60_787_540, &row, &a, &[]);
    assert_eq!(
        gapped_events, fresh_events,
        "the first accepted frame after a rejected seed must behave like a first frame"
    );
}

/// **AN UNRELATED REJECTION LEAVES THE COMMIT STANDING.** Restoration is scoped
/// to the domain's OWN rejection, and this is the arm that stops it widening.
///
/// A frame in which some other reader crossed an hour boundary still PUBLISHED
/// d45's record, and the fold applied it. Restoring there would put the producer
/// behind the fold — the same defect, reached from the other side.
///
/// What production change makes this fail: restoring unconditionally instead of
/// on `rejected_domains.contains(&COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE)`.
///
/// MUTATION RUN: `d45_owns_this_rejection` forced to `true`.
/// OBSERVED: FAILED --
/// "assertion `left == right` failed: d45's record was published on that frame
/// and the fold applied it, so its pin must stay where the commit put it
///   left: 2
///  right: 0"
#[test]
fn an_unrelated_rejection_does_not_restore_the_d45_pin() {
    let row = super::tests::test_country_row("GER", 7, 0x1000);
    let a = frd_focus(1, "FOCUS_A");
    let b = frd_focus(2, "FOCUS_B");

    // The middle frame publishes B and something ELSE is rejected. The last
    // frame re-observes B, so a pin that is where the fold is emits nothing.
    let mut trackers = CompactPerfGateTrackers::default();
    let _ = d45_frame(&mut trackers, 60_787_539, &row, &a, &[]);
    let _ = d45_frame(&mut trackers, 60_787_540, &row, &b, &[OTHER]);
    let events = d45_frame(&mut trackers, 60_787_541, &row, &b, &[]);

    assert_eq!(
        events, 0,
        "d45's record was published on that frame and the fold applied it, so its pin must \
         stay where the commit put it"
    );
}

/// The same property for d55, whose pin is one snapshot vector rather than three
/// keyed maps, and whose emission is a transition set.
///
/// The equivalence is on the EMITTED transitions, because those are what the
/// fold receives. A d55 tracker that was wiped reports every live combat as
/// newly observed; one that was restored reports the transitions since the
/// baseline the fold holds.
///
/// What production change makes this fail: wiping d55's tracker on its own
/// rejection instead of calling `restore_frame_entry_baseline`.
///
/// MUTATION RUN: `tracker.restore_frame_entry_baseline()` replaced with
/// `tracker = land_detail55::LandCombatDetailTracker::default()`.
/// OBSERVED: FAILED --
/// "assertion `left == right` failed: after its own gap, d55 must derive against
/// the baseline the fold still holds
///   left: 2
///  right: 1"
#[test]
fn d55_restores_its_baseline_after_its_own_bracket_is_rejected() {
    fn frame(
        trackers: &mut CompactPerfGateTrackers,
        memory: &FixedLandCombatMemory,
        hour: i32,
        rejected: &[u16],
    ) -> usize {
        let capture = land_detail55::collect_transition_capture_frame(
            memory,
            &mut trackers.land_combat_detail_events_live,
            0,
            hour,
            1,
        )
        .expect("the fixture memory always collects");
        trackers
            .land_combat_detail_events_live
            .commit_staged_baseline();
        settle(trackers, rejected);
        capture.rows.len()
    }

    let one = FixedLandCombatMemory(vec![land_combat_snapshot(1, 60, vec![101])]);
    let two = FixedLandCombatMemory(vec![
        land_combat_snapshot(1, 60, vec![101]),
        land_combat_snapshot(2, 61, vec![102]),
    ]);

    // Reference: the rejected frame never happened.
    let mut reference = CompactPerfGateTrackers::default();
    let _ = frame(&mut reference, &one, 60_787_539, &[]);
    let reference_rows = frame(&mut reference, &two, 60_787_541, &[]);

    let mut gapped = CompactPerfGateTrackers::default();
    let _ = frame(&mut gapped, &one, 60_787_539, &[]);
    let _ = frame(&mut gapped, &two, 60_787_540, &[D55]);
    let gapped_rows = frame(&mut gapped, &two, 60_787_541, &[]);

    assert_eq!(
        gapped_rows, reference_rows,
        "after its own gap, d55 must derive against the baseline the fold still holds"
    );

    // Control: the rejected frame really did move the baseline.
    let mut kept = CompactPerfGateTrackers::default();
    let _ = frame(&mut kept, &one, 60_787_539, &[]);
    let _ = frame(&mut kept, &two, 60_787_540, &[]);
    let kept_rows = frame(&mut kept, &two, 60_787_541, &[]);
    assert_ne!(
        kept_rows, reference_rows,
        "committing the middle frame must change the last one, or this fixture proves nothing"
    );
}

/// **D29 AND THE AIR-WING UNIT DO NOT HAVE THIS DEFECT**, and this gate records
/// the assessment rather than leaving it as a claim in a report.
///
/// The ruling asked whether d29's pin takes the same own-rejection wipe. It does
/// not: `reset_trackers_after_rejected_bracket` takes d29's tracker, calls
/// `roll_back_uncommitted_pin` on it, and puts it BACK — restoration by another
/// name, reached through `stage_delta_pin_advance`'s uncommitted slot rather
/// than through a commit-time undo log. The two shapes differ because d29's pin
/// becomes durable at the READ (so the pre-frame value is captured there), while
/// d45's and d55's are staged at the read and become durable at EMISSION, which
/// is where the value that must be undone appears.
///
/// The air-wing unit takes d29's shape.
///
/// What production change makes this fail: any of these domains being
/// default-constructed on its own rejection instead of rolled back or restored.
///
/// MUTATION RUN: d45's and d55's restorations both replaced with
/// `= <Tracker>::default()`.
/// OBSERVED: FAILED -- "d45 no longer restores its pin on its own rejection
/// (`tracker.restore_pins_to_frame_entry();` is gone). A wiped pin is BEHIND the
/// fold's retained state, and behind loses data exactly as ahead does."
#[test]
fn no_transactional_domain_is_default_constructed_on_its_own_rejection() {
    let source = super::tests::collector_source_lf();
    let reset = source
        .split("\nfn reset_trackers_after_rejected_bracket(")
        .nth(1)
        .expect("the reset function is declared at column zero")
        .split("\n}\n")
        .next()
        .expect("its body ends at the first column-zero brace");

    for (what, needle) in [
        ("d29", "tracker.roll_back_uncommitted_pin();"),
        ("the air-wing unit", "wings.roll_back_uncommitted_pin();"),
        ("d45", "tracker.restore_pins_to_frame_entry();"),
        ("d55", "tracker.restore_frame_entry_baseline();"),
        // d41 joined the set with its POST-GAP pin. Its needle is the binding
        // rather than the call, because the call is spelled exactly as d29's.
        ("d41", "let rolled_back_projects ="),
    ] {
        assert!(
            reset.contains(needle),
            "{what} no longer restores its pin on its own rejection (`{needle}` is gone). A \
             wiped pin is BEHIND the fold's retained state, and behind loses data exactly as \
             ahead does."
        );
    }
    for forbidden in [
        "CompactFrdDepthTracker::default()",
        "LandCombatDetailTracker::default()",
        "CompactShipTracker::default()",
        "CompactAirWingTracker::default()",
        "CompactAirWingLoadoutTracker::default()",
        "CompactAirStrengthDeltaTracker::default()",
        "CompactProjectsTracker::default()",
    ] {
        assert!(
            !reset.contains(forbidden),
            "`{forbidden}` appears in the rejected-bracket reset. None of the transactional \
             domains may reach `Default` there."
        );
    }
}

/// D45 cannot absorb its
/// absorbing direct-history failure through `COMPACT_STATUS_BASELINE_CLEAR`.
///
/// Once `frd_direct_history_failed` is
/// set the fold refuses every later D45 record whatever it carries, and on t26
/// one of d45's 361 post-first-data gaps is enough to set it for the rest of the
/// capture -- and then prescribes the global resync bit as the remedy. The
/// remedy does not survive contact with the contract. This test states the three
/// INDEPENDENT grounds on which `compact_baseline_clear_requested` refuses a D45
/// record, so the direct-history repair cannot quietly stop being true.
///
/// The transport is NOT the obstacle and this test deliberately does not claim
/// it is: FRD2 schema v3 onwards carries an authoritative global status word at
/// D45 payload offset 56, which `parse_frd_depth_header` simply does not read.
///
/// What production change makes this fail -- which is the point, because each
/// would be a real move toward the design:
///
/// * stopping `read_compact_frd_depth_from_country_reads` from setting
///   `DELTA_DATA` unconditionally (ground 1);
/// * D45 ceasing to raise `PARTIAL_DATA` on the targeted-decision and
///   decision-cooldown paths (ground 2);
/// * adding a `CompactBaselineClearAdmission` for domain 45 (ground 3).
///
/// Any of those is an owner decision about the wire contract, not a lane's, and
/// the assertion messages say which one moved.
#[test]
fn d45_cannot_carry_the_global_resync_bit() {
    use compact_contract::{
        compact_baseline_clear_admissible, compact_baseline_clear_requested,
        COMPACT_STATUS_BASELINE_CLEAR,
    };

    // GROUND 1 -- rule 1, whole payloads only. Every D45 read is constructed
    // with DELTA_DATA and nothing clears it, so this is the status word the
    // production reader produces, taken from the production reader.
    let mut tracker = CompactFrdDepthTracker::default();
    let read = read_compact_frd_depth_from_country_reads(&[], 0, 0, &mut tracker);
    assert_ne!(
        read.status_flags & COMPACT_STATUS_DELTA_DATA,
        0,
        "D45's reader sets DELTA_DATA unconditionally; if that changed, the first direct-history          blocker is gone and this test is the place to record it"
    );
    assert!(
        compact_baseline_clear_requested(D45, read.status_flags | COMPACT_STATUS_BASELINE_CLEAR)
            .is_err(),
        "BASELINE_CLEAR on a DELTA_DATA record is a contradiction the contract refuses"
    );

    // GROUND 2 -- rule 3, never on a failure-class record. D45 raises
    // PARTIAL_DATA for a targeted-decision structural failure and for a missing
    // decision-cooldown source, both routine.
    //
    // STATED AS WHAT IT IS: a FREQUENCY argument about the contract, not an
    // invariant about D45. It disqualifies the hours the bit fires on, not every
    // hour -- the empty production read above is delta-without-partial -- and it
    // is asserted against the PREDICATE rather than against a production status
    // word for exactly that reason. Grounds 1 and 3 are unconditional and each
    // independently sustains the conclusion; this one narrows the remaining gap.
    assert!(
        compact_baseline_clear_requested(
            D45,
            COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_BASELINE_CLEAR
        )
        .is_err(),
        "a read that did not complete cannot describe the whole extent it asks the consumer          to replace"
    );

    // GROUND 3 -- rule 4, admissibility. D45's fold ACCUMULATES: focus metrics
    // are entered per tag with `or_default()` and `frd_direct_transitions` is
    // EXTENDED with the hour's drain batch, so a clean D45 record is an
    // increment and never a complete claim about the domain's whole extent.
    assert!(
        !compact_baseline_clear_admissible(D45),
        "D45 is not on COMPACT_BASELINE_CLEAR_ADMISSIONS: a resync there recovers the FUTURE,          never the hole, which the sentence rule does not admit"
    );
    assert!(
        compact_baseline_clear_requested(D45, COMPACT_STATUS_BASELINE_CLEAR).is_err(),
        "even a status word with neither DELTA_DATA nor a failure bit is refused, so the          admission is the third and last ground rather than a consequence of the first two"
    );

    // The control: the one domain the primitive IS for accepts the same word.
    assert_eq!(
        compact_baseline_clear_requested(
            COMPACT_DOMAIN_AIR_REGION_ACTIVITY,
            COMPACT_STATUS_BASELINE_CLEAR
        ),
        Ok(true),
        "d31 is admitted, so the refusals above are about D45 and not about the predicate"
    );
}

// ---------------------------------------------------------------------
// 6. The D45-LOCAL direct-history reset badge
// ---------------------------------------------------------------------

/// One frame of the D45 producer, driven entirely through PRODUCTION calls:
/// write the badge if one is owed, resolve the fold-refusal predicate, stage
/// what the emission site stages, then seal against this frame's publication.
///
/// Every step is the function the gate function itself calls. Nothing here
/// re-spells a rule, which is the whole point: a hand-written seal kept passing
/// while production's own transitions were free to change underneath it.
fn d45_producer_frame(
    tracker: &mut CompactFrdDepthTracker,
    record_carries_validated_source: bool,
    published_its_record: bool,
) -> u32 {
    let mut read = read_compact_frd_depth_from_country_reads(&[], 0, 0, tracker);
    let badged = tracker.stage_direct_history_reset_badge(&mut read);
    if d45_fold_refuses_record(tracker, record_carries_validated_source, badged) {
        tracker.staged_mirror_fails_history = true;
        tracker.roll_back_staged_pins();
    } else {
        tracker.commit_staged_pins();
        if record_carries_validated_source {
            tracker.staged_mirror_publishes_baseline = true;
        }
    }
    tracker.resolve_frame_seal(published_its_record);
    read.depth_status_flags
}

/// A gapped D45 hour after activation must not be one D45 emits a record for,
/// so a frame that publishes nothing is spelled with no record at all.
fn d45_gapped_frame(tracker: &mut CompactFrdDepthTracker) {
    tracker.resolve_frame_seal(false);
}

/// THE WHOLE LIFE OF THE RESET DEBT, through the production seal.
///
/// The fold's `frd_direct_history_failed` is
/// ABSORBING: once set, every later D45 record is refused whatever it carries,
/// and on the t26 capture 361 of d45's 362 gaps fall after its first data
/// record, so d45's direct history died early and stayed dead. The producer
/// owes the stream one reset badge per absorption; the next record it actually
/// PUBLISHES carries bit 7 of `depth_status_flags` and the debt is discharged.
///
/// "Exactly once" is a data-loss property, not tidiness: a second badge tells
/// the fold to discard a history the first badge already restarted, destroying
/// measured rows on the strength of a wire flag.
///
/// What production change makes this fail, per phase:
///  * dropping the gapped-hour arm of `resolve_frame_seal` -- phase 2 leaves
///    the mirror healthy and no badge is ever owed, which is the defect;
///  * discharging the debt at the write instead of at the seal -- phase 3's
///    rejected badge is lost and phase 4 never re-badges;
///  * dropping `direct_history_reset_owed = false` from the payment -- phase 5
///    badges again, which is the second-badge data loss above;
///  * making `d45_fold_refuses_record` refuse a badged record -- phase 4 rolls
///    the pin back on the very record that reopens the domain.
#[test]
fn the_d45_frame_seal_is_the_reset_debts_whole_life() {
    const BADGE: u32 = COMPACT_FRD_STATUS_DIRECT_HISTORY_RESET;
    let mut tracker = CompactFrdDepthTracker::default();

    // PHASE 1 -- activation. A validated record is applied and arms the fold.
    // Nothing is owed, so nothing is badged; without this control a method that
    // badged unconditionally would satisfy every later phase.
    let flags = d45_producer_frame(&mut tracker, true, true);
    assert_eq!(flags & BADGE, 0, "a healthy stream owes no reset");
    assert!(tracker.direct_baseline_published);
    assert!(!tracker.fold_history_failed);
    assert!(!tracker.direct_history_reset_owed);

    // PHASE 2 -- THE GAPPED HOUR. No D45 record reaches the fold, so
    // `world_fold::reset_hour_state` kills the direct history. The producer
    // must learn this from the gap alone: nothing was emitted for it to learn
    // from, which is exactly why the seal has to ask.
    d45_gapped_frame(&mut tracker);
    assert!(
        tracker.fold_history_failed,
        "a gapped hour after activation kills the fold's history and the mirror must say so"
    );
    assert!(
        tracker.direct_history_reset_owed,
        "and a dead history owes a badge"
    );

    // PHASE 3 -- the badge is written, and the frame is then REJECTED. Its
    // records are replaced with a gap, so the fold never saw the badge and the
    // debt must survive.
    let flags = d45_producer_frame(&mut tracker, true, false);
    assert_ne!(flags & BADGE, 0, "the owed badge is written onto the record");
    assert!(
        tracker.direct_history_reset_owed,
        "but a record the frame withdrew paid nothing: discharging it here would leave the \
         domain dead for the rest of the capture, and unlike d31's `resync_owed` nothing \
         would ever re-arm it"
    );
    assert!(tracker.fold_history_failed);

    // PHASE 4 -- the next record IS published, so the badge reaches the fold.
    let flags = d45_producer_frame(&mut tracker, true, true);
    assert_ne!(flags & BADGE, 0, "the surviving debt is paid by this record");
    assert!(
        !tracker.direct_history_reset_owed,
        "and discharged by it"
    );
    assert!(
        !tracker.fold_history_failed,
        "the fold discarded its dead extent, so the absorbing state is over"
    );
    assert!(
        tracker.direct_baseline_published,
        "this record carried a validated trailer, so it established the NEW baseline"
    );

    // PHASE 5 -- the very next record must NOT badge again.
    let flags = d45_producer_frame(&mut tracker, true, true);
    assert_eq!(
        flags & BADGE,
        0,
        "a second badge would tell the fold to discard the history the first one restarted"
    );
    assert!(!tracker.direct_history_reset_owed);
    assert!(!tracker.fold_history_failed);
}

/// A reset paid by an UNTRAILERED record leaves no baseline behind.
///
/// The badge is deliberately not gated on the trailer: one of the two ways the
/// history dies is a source DOWNGRADE, and the record able to pay is then an
/// untrailered one. The fold applies it -- after the reset there is no baseline
/// left for it to have downgraded -- but it establishes none either, so the
/// mirror must come back down. `direct_baseline_published` is monotonic in what
/// a stream has published everywhere except here, and this is the case that
/// proves the exception is needed.
///
/// What production change makes this fail: OR-ing rather than assigning
/// `direct_baseline_published` on the payment path. The producer would then
/// believe the fold was activated, and the next untrailered record would be
/// rolled back as a downgrade the fold never saw.
#[test]
fn a_reset_paid_by_an_untrailered_record_clears_the_baseline_mirror() {
    let mut tracker = CompactFrdDepthTracker::default();
    d45_producer_frame(&mut tracker, true, true);
    d45_gapped_frame(&mut tracker);
    assert!(tracker.direct_history_reset_owed);

    let flags = d45_producer_frame(&mut tracker, false, true);
    assert_ne!(
        flags & COMPACT_FRD_STATUS_DIRECT_HISTORY_RESET,
        0,
        "an untrailered record still pays the debt; refusing to badge it would leave the \
         downgrade shape dead until the hooks came back"
    );
    assert!(!tracker.fold_history_failed);
    assert!(!tracker.direct_history_reset_owed);
    assert!(
        !tracker.direct_baseline_published,
        "and it established NO baseline, because it carried no validated source"
    );
    // The proof that the mirror is now right: the fold applies an untrailered
    // record while it holds no baseline, so the producer must commit its pin.
    assert!(
        !d45_fold_refuses_record(&tracker, false, false),
        "with no baseline the fold applies an untrailered record, so a producer that thought \
         otherwise would roll back a record the fold kept"
    );
}
