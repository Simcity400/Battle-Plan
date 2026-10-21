use super::*;

fn plan(ledger: &mut Ledger<(u32, i32)>, ticket: Ticket, epoch: u64, hour: i32, before: &[Ticket]) {
    assert!(ledger.reserve(ticket, epoch, (1, hour), before.iter().copied().collect()));
}
fn admit_all(ledger: &mut Ledger<(u32, i32)>, ticket: Ticket) {
    for part in 0..8 {
        assert!(ledger.accepted(ticket, part));
    }
}
fn finish_all(ledger: &mut Ledger<(u32, i32)>, ticket: Ticket) {
    for part in (0..8).rev() {
        assert!(ledger.completed(ticket, part));
    }
}

#[test]
fn a_ready_hour_is_not_held_by_a_later_decode() {
    let mut ledger = Ledger::new(2);
    plan(&mut ledger, 1, 0, 10, &[]);
    assert!(ledger.decoded(1, 0));
    admit_all(&mut ledger, 1);
    plan(&mut ledger, 2, 0, 11, &[1]); // deliberately never decode it yet
    finish_all(&mut ledger, 1);
    assert_eq!(ledger.ready(), [1]);
    assert!(ledger.release(1)); // delivery is possible before releasing decode2
    assert_eq!(ledger.get(2).unwrap().phase, Phase::Decode);
    assert!(ledger.decoded(2, 0));
    admit_all(&mut ledger, 2);
    finish_all(&mut ledger, 2);
    assert_eq!(ledger.ready(), [2]);
}

#[test]
fn full_pool_does_not_prevent_older_completion_or_retarget() {
    let mut ledger = Ledger::new(2);
    plan(&mut ledger, 1, 0, 10, &[]);
    assert!(ledger.decoded(1, 0));
    admit_all(&mut ledger, 1);
    plan(&mut ledger, 2, 0, 11, &[1]);
    assert!(ledger.decoded(2, 0));
    // Adapter receives Full for this closure and keeps it without calling accepted.
    assert_eq!(ledger.next_admission(), Some((2, 0)));
    finish_all(&mut ledger, 1);
    assert_eq!(ledger.ready(), [1]);
    // A new Want can supersede wholly unaccepted2 while1 remains deliverable.
    assert_eq!(ledger.retarget(|_, key| key.1 == 10), [2]);
    assert_eq!(ledger.ready(), [1]);
    assert_eq!(ledger.next_admission(), None);
}

#[test]
fn every_pool_accepted_target_retains_held_jobs_until_terminal() {
    let mut ledger = Ledger::new(2);
    plan(&mut ledger, 1, 0, 10, &[]);
    assert!(ledger.decoded(1, 0));
    assert!(ledger.accepted(1, 0));
    assert!(ledger.retarget(|_, _| false).is_empty());
    assert!(ledger.get(1).unwrap().retired);
    assert!(
        !ledger.can_plan(),
        "seven held predecessor jobs must be admitted first"
    );
    for part in 1..8 {
        assert!(ledger.accepted(1, part));
    }
    plan(&mut ledger, 2, 1, 100, &[]);
    finish_all(&mut ledger, 1);
    assert!(ledger.ready().is_empty());
    assert_eq!(ledger.terminal_retired(), [1]);
    assert!(ledger.release(1));
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger.get(2).unwrap().phase, Phase::Decode);
}

#[test]
fn reused_predecessor_is_not_superseded_while_a_kept_target_depends_on_it() {
    let mut ledger = Ledger::new(2);
    plan(&mut ledger, 1, 0, 10, &[]);
    assert!(ledger.decoded(1, ALL_PARTS));
    plan(&mut ledger, 2, 0, 12, &[1]);
    assert!(ledger.retarget(|_, key| key.1 == 12).is_empty());
    assert!(ledger.get(1).unwrap().retired);
    assert!(ledger.get(2).is_some());
    assert_eq!(ledger.terminal_retired(), [1]);
    // Its immutable value was captured by2. Releasing a terminal predecessor
    // does not create a dependency on a target which has not been admitted.
    assert!(ledger.release(1));
    assert!(ledger.decoded(2, 0));
    admit_all(&mut ledger, 2);
}

#[test]
fn completed_unsent_targets_keep_credits_without_a_backlog() {
    let mut ledger = Ledger::new(2);
    for ticket in 1..=2 {
        plan(&mut ledger, ticket, 0, 9 + ticket as i32, &[]);
        assert!(ledger.gap(ticket));
    }
    assert_eq!(ledger.ready(), [1, 2]);
    assert!(!ledger.can_plan());
    assert!(!ledger.reserve(3, 0, (1, 12), BTreeSet::new()));
    // An attempted Full send changes no metadata and does not release credit.
    assert_eq!(ledger.len(), 2);
    assert!(ledger.release(1)); // only after explicit output acceptance
    assert!(ledger.can_plan());
    plan(&mut ledger, 3, 0, 12, &[]);
}

#[test]
fn stale_duplicate_and_unaccepted_part_events_cannot_complete_a_target() {
    let mut ledger = Ledger::new(2);
    plan(&mut ledger, 1, 0, 10, &[]);
    assert_eq!(ledger.retarget(|_, _| false), [1]);
    plan(&mut ledger, 2, 0, 10, &[]);
    assert!(ledger.decoded(2, 0));
    assert!(!ledger.completed(1, 0));
    assert!(!ledger.completed(2, 0));
    assert!(!ledger.completed(2, 8));
    assert!(ledger.accepted(2, 0));
    assert!(ledger.completed(2, 0));
    assert!(!ledger.completed(2, 0));
    assert!(!ledger.accepted(2, 0));
    assert!(ledger.ready().is_empty());
    for part in 1..8 {
        assert!(ledger.accepted(2, part));
        assert!(ledger.completed(2, part));
    }
    assert_eq!(ledger.ready(), [2]);
}

#[test]
fn reused_parts_occupy_their_fixed_positions_without_new_jobs() {
    let mut ledger = Ledger::new(1);
    plan(&mut ledger, 1, 0, 10, &[]);
    // Rails, buildings, movers, fog and snow are already present.
    assert!(ledger.decoded(1, 0b1111_1000));
    for part in 0..3 {
        assert_eq!(ledger.next_admission(), Some((1, part)));
        assert!(ledger.accepted(1, part));
    }
    assert_eq!(ledger.next_admission(), None);
    for part in [2, 0, 1] {
        assert!(ledger.completed(1, part));
    }
    assert_eq!(ledger.ready(), [1]);
}

#[test]
fn identical_ready_sets_have_dispatch_order_regardless_of_arrival_order() {
    let mut baseline = None;
    for first in [1, 2, 3] {
        let mut ledger = Ledger::new(3);
        for ticket in 1..=3 {
            plan(&mut ledger, ticket, 0, 9 + ticket as i32, &[]);
            assert!(ledger.decoded(ticket, 0));
            admit_all(&mut ledger, ticket);
        }
        let order = [first, first % 3 + 1, (first + 1) % 3 + 1];
        for ticket in order {
            finish_all(&mut ledger, ticket);
        }
        let ready = ledger.ready();
        assert_eq!(ready, [1, 2, 3]);
        if let Some(baseline) = &baseline {
            assert_eq!(&ready, baseline);
        } else {
            baseline = Some(ready);
        }
    }
}

#[test]
fn invalid_dependencies_and_duplicate_target_reservations_are_refused() {
    let mut ledger = Ledger::new(3);
    assert!(!ledger.reserve(1, 0, (1, 10), BTreeSet::from([1])));
    assert!(!ledger.reserve(1, 0, (1, 10), BTreeSet::from([7])));
    plan(&mut ledger, 1, 0, 10, &[]);
    assert!(
        !ledger.reserve(2, 0, (1, 11), BTreeSet::new()),
        "one decode plan"
    );
    assert!(ledger.gap(1));
    assert!(
        !ledger.reserve(2, 0, (1, 10), BTreeSet::new()),
        "same live target"
    );
    plan(&mut ledger, 2, 1, 10, &[]); // new epoch can rebuild same recorded hour
    assert!(ledger.gap(2));
    assert_eq!(ledger.retarget(|epoch, _| epoch == 1), [1]);
    assert_eq!(ledger.ready(), [2]);
}

#[test]
fn cancelled_source_reads_keep_their_credit_until_their_terminal_reply() {
    let mut ledger = Ledger::new(1);
    plan(&mut ledger, 1, 0, 10, &[]);
    assert!(ledger.source_started(1));
    assert!(ledger.retarget(|_, _| false).is_empty());
    assert_eq!(ledger.obsolete_reads(), [1]);
    assert!(!ledger.can_plan());
    assert!(!ledger.decoded(1, 0));
    // A queued cancellation can reply immediately; an executing read replies
    // at its ordinary terminal/cancellation boundary. Neither frees early.
    assert!(ledger.source_finished(1));
    assert_eq!(ledger.retarget(|_, _| false), [1]);
    assert!(ledger.can_plan());
    plan(&mut ledger, 2, 1, 100, &[]);
    assert!(!ledger.source_finished(1));
}
