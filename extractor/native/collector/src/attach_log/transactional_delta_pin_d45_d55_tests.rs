use super::*;

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

    pin.roll_back();
    assert_eq!(pin.get(&7).copied(), Some(1));
    assert!(!pin.has_staged());
}

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
    assert_eq!(
        pin.get(&2).copied(),
        Some(2),
        "and must not disturb its neighbours"
    );

    pin.stage(1, 9);
    pin.commit();
    assert_eq!(pin.get(&1).copied(), Some(9));
    assert_eq!(pin.get(&2).copied(), Some(2));
}

struct FixedLandCombatMemory(Vec<land_detail55::LandCombatSnapshot>);

impl land_detail55::LandCombatDetailMemory for FixedLandCombatMemory {
    fn land_combat_snapshots(&self) -> Result<Vec<land_detail55::LandCombatSnapshot>, Vec<String>> {
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

#[test]
fn a_discarded_d55_frame_leaves_the_producer_on_the_consumers_baseline() {
    let frame_a = FixedLandCombatMemory(vec![land_combat_snapshot(1, 60, vec![101])]);
    let frame_b = FixedLandCombatMemory(vec![land_combat_snapshot(1, 61, vec![101, 102])]);

    let mut withheld = land_detail55::LandCombatDetailTracker::default();
    land_detail55::collect_transition_capture_frame(&frame_a, &mut withheld, 0, 60_787_539, 1)
        .expect("frame A collects");
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

    let mut never_happened = land_detail55::LandCombatDetailTracker::default();
    let after_never = land_detail55::collect_transition_capture_frame(
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

    fn empty_read() -> CompactFrdDepthRead {
        let mut read = compact_frd_depth_worker_panic_read(0);
        read.status_flags = 0;
        read
    }

    let row = super::tests::test_country_row("GER", 7, 0x1000);
    let (row_a, focus_a) = (row.clone(), focus(1, "FOCUS_A"));
    let (row_b, focus_b) = (row, focus(3, "FOCUS_B"));

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

    let ordinary_partial = COMPACT_STATUS_DELTA_DATA | COMPACT_STATUS_PARTIAL_DATA;
    assert!(
        !compact_contract::compact_delta_pin_may_advance(
            COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
            ordinary_partial,
        ),
        "the shared predicate refuses this word, which is why D45 must not use it"
    );

    let fold = super::tests::collector_source_lf();
    let _ = fold;

    let row = super::tests::test_country_row("GER", 7, 0x1000);
    let a = focus(1, "FOCUS_A");
    let b = focus(1, "FOCUS_B");

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

#[test]
fn the_d45_pin_predicate_mirrors_the_folds_activation_state() {
    let mut tracker = CompactFrdDepthTracker::default();

    use super::d45_fold_refuses_record as fold_refuses;

    assert!(
        !fold_refuses(&tracker, false, false),
        "before the fold has ever accepted a validated source it APPLIES an untrailered \
         record, so the producer must commit"
    );

    assert!(!fold_refuses(&tracker, true, false));
    tracker.direct_baseline_published = true;

    assert!(
        fold_refuses(&tracker, false, false),
        "after activation a downgraded record is refused whole, so the pin must not advance"
    );

    tracker.fold_history_failed = true;
    assert!(fold_refuses(&tracker, true, false));
    assert!(fold_refuses(&tracker, false, false));

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

#[test]
fn a_withdrawn_d45_record_does_not_arm_the_fold_mirror() {
    let mut tracker = CompactFrdDepthTracker::default();
    tracker.direct_baseline_published = true;
    assert!(
        super::d45_fold_refuses_record(&tracker, false, false),
        "an untrailered record after activation is refused, so this frame stages a failure"
    );
    tracker.staged_mirror_fails_history = true;

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

#[test]
fn the_d45_fold_mirror_is_carried_across_a_rejected_bracket() {
    let mut trackers = CompactPerfGateTrackers::default();
    trackers
        .focus_research_decisions_depth_live
        .direct_baseline_published = true;
    trackers
        .focus_research_decisions_depth_live
        .fold_history_failed = true;
    trackers
        .focus_research_decisions_depth_live
        .focus_by_tag
        .stage(
            7,
            CompactFrdFocusState {
                tag_db_id: 7,
                completed_count: Some(1),
                progress_raw: None,
                current: None,
                current_continuous: None,
                completed_tokens: Vec::new(),
                paused: None,
            },
        );
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
        trackers
            .focus_research_decisions_depth_live
            .fold_history_failed,
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

#[test]
fn a_rolled_back_rebaseline_is_announced_again() {
    let frame = FixedLandCombatMemory(vec![land_combat_snapshot(1, 60, vec![101])]);

    let rebaseline_only = COMPACT_STATUS_DELTA_DATA
        | COMPACT_STATUS_PARTIAL_DATA
        | COMPACT_STATUS_TRANSITION_BASELINE_RESET;
    assert!(
        compact_contract::compact_delta_pin_may_advance(
            COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
            rebaseline_only,
        ),
        "the fold applies a plain rebaseline, so refusing to commit it would loop forever"
    );

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
    assert!(
        first.rows.is_empty(),
        "a rebaseline emits no synthetic events"
    );
    assert!(
        tracker.last_frame_rebaselined(),
        "and the reader can see the reset it just performed"
    );
    tracker.roll_back_staged_baseline();

    let second =
        land_detail55::collect_transition_capture_frame(&frame, &mut tracker, 0, 60_787_540, 1)
            .expect("second frame collects");
    assert!(
        tracker.last_frame_rebaselined(),
        "the rebaseline must be announced again after the consumer discarded it"
    );
    assert!(second.rows.is_empty());

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

#[test]
fn every_pin_commit_is_guarded_by_the_predicate_its_own_fold_uses() {
    let source = super::tests::collector_source_lf();

    for (commit, roll_back, guard) in [
        (
            "commit_staged_pins()",
            "roll_back_staged_pins()",
            "if d45_fold_refuses_this_record {",
        ),
        (
            "commit_staged_baseline()",
            "roll_back_staged_baseline()",
            "if read.frame.hour_custody.snapshot.admits(game_hour_raw)",
        ),
    ] {
        let commits = source.matches(commit).count();
        assert_eq!(
            commits, 1,
            "{commit} must have exactly one call site in production; found {commits}"
        );

        let at = source
            .find(commit)
            .expect("the commit call site was just counted");
        let window_start = at.saturating_sub(900);
        let window = &source[window_start..at];
        assert!(
            window.contains(guard),
            "{commit} is called but is not guarded by {guard}"
        );

        if commit == "commit_staged_baseline()" {
            assert!(window.contains("&& compact_contract::compact_delta_pin_may_advance("));
        }

        let around = &source[window_start..(at + 900).min(source.len())];
        assert!(
            around.contains(roll_back),
            "{commit} has no {roll_back} arm; an unconsumed stage would survive the frame"
        );
    }

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
        resolution_body.contains(
            "!rejected_domains.contains(&COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE)"
        ),
        "D45's publication must be read from the rejected set at the seal; the emission site \
         cannot know it yet"
    );
    assert!(
        resolution_body.contains(".resolve_frame_seal(d45_published_its_record);"),
        "the seal must be resolved through the production transition, and with THIS frame's \
         publication as its argument"
    );

    let d45_at = source
        .find("commit_staged_pins()")
        .expect("the D45 commit site");
    let d45_window = &source[d45_at.saturating_sub(900)..d45_at];
    assert!(
        !d45_window.contains("compact_delta_pin_may_advance("),
        "D45's commit must not be gated on the status word: its fold applies a PARTIAL_DATA \
         record, so a status-word guard leaves the producer BEHIND the fold"
    );

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

#[test]
fn the_pin_predicate_matches_what_the_folds_actually_apply() {
    use compact_contract::compact_delta_pin_may_advance as may_advance;

    assert!(may_advance(
        COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
        0
    ));
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

    let rebaseline = COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_TRANSITION_BASELINE_RESET;
    assert!(may_advance(
        COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
        rebaseline
    ));
    assert!(
        !may_advance(
            COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
            rebaseline
        ),
        "the rebaseline tolerance belongs to d55 alone"
    );

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

fn settle(trackers: &mut CompactPerfGateTrackers, rejected: &[u16]) {
    let rejected: BTreeSet<u16> = rejected.iter().copied().collect();
    if !rejected.is_empty() {
        reset_trackers_after_rejected_bracket(trackers, &rejected);
    }
    commit_delta_pin_transactions(trackers, &mut Vec::new());
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
    trackers
        .focus_research_decisions_depth_live
        .commit_staged_pins();
    settle(trackers, rejected);
    read.focus_events.len()
}

const D45: u16 = COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE;
const D55: u16 = COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE;
const OTHER: u16 = COMPACT_DOMAIN_WEATHER;

#[test]
fn d45_restores_its_pin_after_its_own_bracket_is_rejected() {
    let row = super::tests::test_country_row("GER", 7, 0x1000);
    let a = frd_focus(1, "FOCUS_A");
    let b = frd_focus(2, "FOCUS_B");

    let mut reference = CompactPerfGateTrackers::default();
    let _ = d45_frame(&mut reference, 60_787_539, &row, &a, &[]);
    let reference_events = d45_frame(&mut reference, 60_787_541, &row, &b, &[]);

    let mut gapped = CompactPerfGateTrackers::default();
    let _ = d45_frame(&mut gapped, 60_787_539, &row, &a, &[]);
    let _ = d45_frame(&mut gapped, 60_787_540, &row, &b, &[D45]);
    let gapped_events = d45_frame(&mut gapped, 60_787_541, &row, &b, &[]);

    assert_eq!(
        gapped_events, reference_events,
        "after its own gap, d45 must derive against the baseline the fold still holds"
    );
    let mut kept = CompactPerfGateTrackers::default();
    let _ = d45_frame(&mut kept, 60_787_539, &row, &a, &[]);
    let _ = d45_frame(&mut kept, 60_787_540, &row, &b, &[]);
    let kept_events = d45_frame(&mut kept, 60_787_541, &row, &b, &[]);
    assert_ne!(
        kept_events, reference_events,
        "committing the middle frame must change the last one, or this fixture proves nothing"
    );
}

#[test]
fn two_consecutive_d45_rejections_rewind_one_frame_each() {
    let row = super::tests::test_country_row("GER", 7, 0x1000);
    let a = frd_focus(1, "FOCUS_A");
    let b = frd_focus(2, "FOCUS_B");
    let c = frd_focus(3, "FOCUS_C");

    let mut reference = CompactPerfGateTrackers::default();
    let _ = d45_frame(&mut reference, 60_787_539, &row, &a, &[]);

    let mut gapped = CompactPerfGateTrackers::default();
    let _ = d45_frame(&mut gapped, 60_787_539, &row, &a, &[]);
    let _ = d45_frame(&mut gapped, 60_787_540, &row, &b, &[D45]);
    let _ = d45_frame(&mut gapped, 60_787_541, &row, &c, &[D45]);

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

    let mut fresh = CompactPerfGateTrackers::default();
    let fresh_events = d45_frame(&mut fresh, 60_787_540, &row, &a, &[]);
    let gapped_events = d45_frame(&mut gapped, 60_787_540, &row, &a, &[]);
    assert_eq!(
        gapped_events, fresh_events,
        "the first accepted frame after a rejected seed must behave like a first frame"
    );
}

#[test]
fn an_unrelated_rejection_does_not_restore_the_d45_pin() {
    let row = super::tests::test_country_row("GER", 7, 0x1000);
    let a = frd_focus(1, "FOCUS_A");
    let b = frd_focus(2, "FOCUS_B");

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

    let mut kept = CompactPerfGateTrackers::default();
    let _ = frame(&mut kept, &one, 60_787_539, &[]);
    let _ = frame(&mut kept, &two, 60_787_540, &[]);
    let kept_rows = frame(&mut kept, &two, 60_787_541, &[]);
    assert_ne!(
        kept_rows, reference_rows,
        "committing the middle frame must change the last one, or this fixture proves nothing"
    );
}

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

#[test]
fn d45_cannot_carry_the_global_resync_bit() {
    use compact_contract::{
        compact_baseline_clear_admissible, compact_baseline_clear_requested,
        COMPACT_STATUS_BASELINE_CLEAR,
    };

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

    assert!(
        compact_baseline_clear_requested(
            D45,
            COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_BASELINE_CLEAR
        )
        .is_err(),
        "a read that did not complete cannot describe the whole extent it asks the consumer          to replace"
    );

    assert!(
        !compact_baseline_clear_admissible(D45),
        "D45 is not on COMPACT_BASELINE_CLEAR_ADMISSIONS: a resync there recovers the FUTURE,          never the hole, which the sentence rule does not admit"
    );
    assert!(
        compact_baseline_clear_requested(D45, COMPACT_STATUS_BASELINE_CLEAR).is_err(),
        "even a status word with neither DELTA_DATA nor a failure bit is refused, so the          admission is the third and last ground rather than a consequence of the first two"
    );

    assert_eq!(
        compact_baseline_clear_requested(
            COMPACT_DOMAIN_AIR_REGION_ACTIVITY,
            COMPACT_STATUS_BASELINE_CLEAR
        ),
        Ok(true),
        "d31 is admitted, so the refusals above are about D45 and not about the predicate"
    );
}

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

fn d45_gapped_frame(tracker: &mut CompactFrdDepthTracker) {
    tracker.resolve_frame_seal(false);
}

#[test]
fn the_d45_frame_seal_is_the_reset_debts_whole_life() {
    const BADGE: u32 = COMPACT_FRD_STATUS_DIRECT_HISTORY_RESET;
    let mut tracker = CompactFrdDepthTracker::default();

    let flags = d45_producer_frame(&mut tracker, true, true);
    assert_eq!(flags & BADGE, 0, "a healthy stream owes no reset");
    assert!(tracker.direct_baseline_published);
    assert!(!tracker.fold_history_failed);
    assert!(!tracker.direct_history_reset_owed);

    d45_gapped_frame(&mut tracker);
    assert!(
        tracker.fold_history_failed,
        "a gapped hour after activation kills the fold's history and the mirror must say so"
    );
    assert!(
        tracker.direct_history_reset_owed,
        "and a dead history owes a badge"
    );

    let flags = d45_producer_frame(&mut tracker, true, false);
    assert_ne!(
        flags & BADGE,
        0,
        "the owed badge is written onto the record"
    );
    assert!(
        tracker.direct_history_reset_owed,
        "but a record the frame withdrew paid nothing: discharging it here would leave the \
         domain dead for the rest of the capture, and unlike d31's `resync_owed` nothing \
         would ever re-arm it"
    );
    assert!(tracker.fold_history_failed);

    let flags = d45_producer_frame(&mut tracker, true, true);
    assert_ne!(
        flags & BADGE,
        0,
        "the surviving debt is paid by this record"
    );
    assert!(!tracker.direct_history_reset_owed, "and discharged by it");
    assert!(
        !tracker.fold_history_failed,
        "the fold discarded its dead extent, so the absorbing state is over"
    );
    assert!(
        tracker.direct_baseline_published,
        "this record carried a validated trailer, so it established the NEW baseline"
    );

    let flags = d45_producer_frame(&mut tracker, true, true);
    assert_eq!(
        flags & BADGE,
        0,
        "a second badge would tell the fold to discard the history the first one restarted"
    );
    assert!(!tracker.direct_history_reset_owed);
    assert!(!tracker.fold_history_failed);
}

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
    assert!(
        !d45_fold_refuses_record(&tracker, false, false),
        "with no baseline the fold applies an untrailered record, so a producer that thought \
         otherwise would roll back a record the fold kept"
    );
}
