//! Gates for the TRANSACTIONAL delta pin on d29 `ships`.
//!
//! D29 is a keyed diff with tombstones, and its consumer runs
//! `world_fold::replace_or_sparse_after_first`, which returns SPARSE for every
//! record after the first. The only thing that ever removes a key from the
//! folded world is therefore an explicit tombstone row.
//!
//! **D32 `air_wing_loadouts` has the identical defect and is deliberately NOT
//! repaired here** — its generation is shared with d30 through the fold's
//! orphan prune, and a one-sided repair deletes every d32 row on 358 of t26's
//! 359 gapped frames. `d32_is_not_on_the_transactional_pin_until_d30_joins_it`
//! and `d30_and_d32_renumber_identically_across_a_shared_rejection` carry that
//! decision and its measurement.
//!
//! The defect this module gates against: after the domain's OWN clock bracket
//! was rejected, `reset_trackers_after_rejected_bracket` used to DROP the pin.
//! With an empty pin `emit_full_snapshot` becomes true, and that branch skips
//! the tombstone sweep entirely — it has no baseline left to sweep. The fold
//! then merged that whole snapshot, so an entity destroyed while the domain was
//! gapped was in the world before the gap, absent from the post-gap snapshot,
//! never tombstoned, and never removed.
//!
//! Measured over `runs/live_unlock/t26/collector_stream.bin` (2,594,412,495
//! bytes) by replaying its 296 d29 `DOMAIN_DATA` and 348 `DOMAIN_GAP` records
//! through that fold model: 93 post-own-gap whole snapshots, and **11 ship
//! identities left in the folded world that no later record ever removes**
//! (instance ids 259, 1371, 1372, 1373, 1482, 1485, 1491, 2032, 4361, 4519,
//! 4525). All 296 of those records carry status word `0x0`, so none of them is
//! an incomplete read whose absences might mean "not measured": each one is a
//! complete claim about the live fleet. Under the pin this module gates, the
//! encoder emits exactly 11 tombstone rows for those 11 identities.
//!
//! The fix is a transaction, not a retention. A retained pin would be wrong in
//! the other direction: a row first observed on the REJECTED frame was never
//! delivered to the fold, so retaining it would suppress that row forever.
//! Rolling back to the last COMMITTED value is what puts producer and consumer
//! back in step, because the rejected frame's records were replaced with a
//! `DOMAIN_GAP` and the fold's mirror is exactly the pre-frame state.
//!
//! See `extractor/documents/DELTA_WIRE_DESIGN.md` §7.1.

use super::*;

const D29: u16 = COMPACT_DOMAIN_SHIPS;
/// Used only by the two gates that keep d32 OFF the transactional pin.
const D32: u16 = COMPACT_DOMAIN_AIR_WING_LOADOUTS;
/// A domain whose bracket is unrelated to d29's and d32's, used for the
/// controls that must NOT roll a pin back.
const UNRELATED: u16 = COMPACT_DOMAIN_WEATHER;

fn ship(instance_id: u32, strength_raw: u32) -> CompactShipRow {
    CompactShipRow {
        instance_id,
        deleted: false,
        task_force_id: Some(201),
        owner_tag_db_id: Some(1),
        definition: Some("destroyer_1".into()),
        category_enum: 1,
        strength_raw,
        organisation_raw: 1_000_000,
        experience_raw: 0,
        sunk_convoys: 0,
    }
}

fn loadout(wing_id: u32, body_addr: usize, amount_raw: i64) -> CompactAirWingLoadoutRow {
    CompactAirWingLoadoutRow {
        owner_tag_db_id: 1,
        wing_id,
        generation: 0,
        body_addr,
        deleted: false,
        air_base_id: 9001,
        equipment: vec![CompactAirWingLoadoutEntry {
            variant_id: 900,
            amount_raw,
        }],
    }
}

fn rejected(domains: &[u16]) -> BTreeSet<u16> {
    domains.iter().copied().collect()
}

/// One frame's worth of d29 lifecycle: read, then settle the frame.
///
/// `frame_rejected_domains` is what the frame's bracket verdict produced, so
/// this is the production sequence — `reset_trackers_after_rejected_bracket`
/// only when something was rejected, then the unconditional commit — and not a
/// convenience wrapper that could drift from it.
fn settle_frame(trackers: &mut CompactPerfGateTrackers, frame_rejected_domains: &BTreeSet<u16>) {
    if !frame_rejected_domains.is_empty() {
        reset_trackers_after_rejected_bracket(trackers, frame_rejected_domains);
    }
    commit_delta_pin_transactions(trackers);
}

fn deleted_ids(rows: &[CompactShipRow]) -> Vec<u32> {
    rows.iter()
        .filter(|row| row.deleted)
        .map(|row| row.instance_id)
        .collect()
}

fn live_ids(rows: &[CompactShipRow]) -> Vec<u32> {
    rows.iter()
        .filter(|row| !row.deleted)
        .map(|row| row.instance_id)
        .collect()
}

/// **The gate.** A ship destroyed while d29 was gapped is TOMBSTONED by the
/// first emission after the gap.
///
/// Frame 1 publishes ships 1 and 2, so both are in the fold. Frame 2's d29
/// bracket is rejected: its `DOMAIN_DATA` becomes a `DOMAIN_GAP` and the fold
/// never sees it. Frame 3 is the first accepted read after the gap, and ship 2
/// is gone. Only a tombstone can remove it from a merging fold.
///
/// What production change makes this fail: dropping the pin on a rejected
/// bracket instead of rolling it back (the pre-fix behaviour), skipping the
/// tombstone sweep whenever `emit_full_snapshot` is set, or removing the
/// staging in `compact_ship_delta_rows`.
///
/// MUTATION RUN: `rolled_back_ships` reverted to the pre-fix drop —
/// `let rolled_back_ships: Option<CompactShipTracker> = None;` in
/// `reset_trackers_after_rejected_bracket`, so a rejected d29 bracket takes the
/// blanket reset again.
/// OBSERVED: FAILED -- "assertion `left == right` failed: the first d29
/// emission after its own gap must tombstone the ship that vanished during the
/// gap; the pre-fix code emitted a whole snapshot with no deletions at all:
/// live [1], deleted []
///   left: []
///  right: [2]"
/// (3 of the module's other 7 tests failed on the same mutation:
/// `a_row_first_seen_on_the_rejected_frame_is_re_emitted_not_suppressed`,
/// `a_committed_pin_is_not_rolled_back_by_a_later_frames_rejection`, and
/// `a_withheld_frame_that_is_also_rejected_keeps_the_committed_pin`.)
#[test]
fn a_ship_destroyed_during_its_own_domains_gap_is_tombstoned_after_it() {
    let mut trackers = CompactPerfGateTrackers::default();

    // Frame 1 — the keyframe. Both ships reach the fold.
    let (rows, _changed, deleted) =
        compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    assert_eq!(live_ids(&rows), vec![1, 2], "the keyframe carries both ships");
    assert_eq!(deleted, 0, "nothing is deleted on the first read");
    settle_frame(&mut trackers, &rejected(&[]));

    // Frame 2 — d29's own bracket is rejected. The reader still RAN and staged
    // its advance before the verdict; ship 2 was destroyed in this window.
    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10)], 0);
    settle_frame(&mut trackers, &rejected(&[D29]));

    // Frame 3 — the first accepted read after the gap.
    let (rows, _changed, deleted) = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10)], 0);
    assert_eq!(
        deleted_ids(&rows),
        vec![2],
        "the first d29 emission after its own gap must tombstone the ship that vanished \
         during the gap; the pre-fix code emitted a whole snapshot with no deletions at all: \
         live {:?}, deleted {:?}",
        live_ids(&rows),
        deleted_ids(&rows),
    );
    assert_eq!(deleted, 1, "the deleted_rows header count must agree");
}

/// **D32 has the identical defect and is deliberately NOT repaired here.** This
/// gate exists so that stays a decision rather than an oversight.
///
/// D32 cannot be moved onto the transactional pin on its own. Its generation is
/// shared with d30 through the fold: both are keyed on `AirWingKey {
/// owner_tag_db_id, wing_id, generation }`, and
/// `world_fold::prune_air_wing_loadout_orphans_for_current_hour` DELETES any
/// d32 row whose key has no d30 counterpart. The two values agree only because
/// both trackers take the SAME blanket reset on a rejected bracket and both
/// then renumber from zero together.
///
/// Measured over `runs/live_unlock/t26/collector_stream.bin`: d30 and d32 are
/// gapped TOGETHER in 358 of 359 frames (**0** frames gap d32 alone), every
/// post-gap record from both renumbers densely from generation 0, their maximum
/// generations are equal (652), and the fold prunes 0 orphan rows. Rolling only
/// d32's pin back stops it renumbering while d30 still does, so every d32 row
/// misses its d30 counterpart and the prune deletes all of them — on 358 of 359
/// gapped frames. That is far worse than the defect it would fix, which is why
/// the d32 arm of this lane was reverted.
///
/// What production change makes this fail: adding d32 to the reset's
/// transactional carve-out or to `commit_delta_pin_transactions` without also
/// bringing d30 (and d42, which mints from the same wing bodies) onto a shared
/// generation authority.
#[test]
fn d32_is_not_on_the_transactional_pin_until_d30_joins_it() {
    // CRLF: this repo checks out with CRLF and the patterns below span lines.
    let source = super::tests::collector_source_lf();
    assert!(
        !source.contains("trackers.air_wing_loadouts.commit_pin()"),
        "d32 joined `commit_delta_pin_transactions`. Read this test's comment first: without \
         d30 on the same transaction, the fold's orphan prune deletes every d32 row after a \
         shared gap."
    );
    assert!(
        !source.contains("rolled_back_air_wing_loadouts"),
        "d32 joined the rejected-bracket rollback carve-out. Read this test's comment first."
    );
    // And the positive half: d32 still advances its pin the old way, so the
    // blanket reset is still what it takes.
    assert!(
        source.contains("    // NOT staged: d32 is off the transactional pin. See\n    // `CompactAirWingLoadoutTracker`.\n    tracker.previous = current;\n"),
        "`compact_air_wing_loadout_delta_rows` no longer advances its pin unstaged; if it is \
         now transactional, d30 must be too"
    );
}

/// The rollback is a TRANSACTION, not a retention: a row first observed on the
/// rejected frame was never delivered to the fold, so the next accepted frame
/// must carry it as a change.
///
/// This is the arm that rules out the cheaper "just keep the pin" fix. Keeping
/// the pin would leave ship 3 in it, the diff would call ship 3 unchanged, and
/// a ship the fold never received would be suppressed for the rest of the
/// campaign.
///
/// What production change makes this fail: replacing the rollback with a plain
/// retention, or staging the advance with an unconditional `*uncommitted =
/// Some(previous)` that overwrites the committed value on a second advance.
#[test]
fn a_row_first_seen_on_the_rejected_frame_is_re_emitted_not_suppressed() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10)], 0);
    settle_frame(&mut trackers, &rejected(&[]));

    // Frame 2 — rejected, and it saw a NEW ship 3 the fold therefore never got.
    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(3, 30)], 0);
    settle_frame(&mut trackers, &rejected(&[D29]));

    let (rows, ..) = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(3, 30)], 0);
    assert_eq!(
        live_ids(&rows),
        vec![3],
        "ship 3 was only ever staged on a frame the fold discarded, so the next accepted \
         frame must deliver it: {rows:?}"
    );
    assert!(
        deleted_ids(&rows).is_empty(),
        "nothing left the world, so nothing may be tombstoned: {rows:?}"
    );
}

/// A rejection of a DIFFERENT domain must not roll d29's pin back. The pin's
/// domain-scoped retention (`for_each_domain_scoped_tracker`) already says so;
/// this is the arm that stops the new carve-out from quietly widening it.
///
/// What production change makes this fail: rolling back unconditionally instead
/// of only when `domain_scoped_tracker_tag_survived` says d29 did not survive.
#[test]
fn an_unrelated_domains_rejection_does_not_roll_the_d29_pin_back() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10)], 0);
    settle_frame(&mut trackers, &rejected(&[]));

    // d29 survives; something else did not. Ship 2 appears and IS published.
    let (rows, ..) = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    assert_eq!(live_ids(&rows), vec![2]);
    settle_frame(&mut trackers, &rejected(&[UNRELATED]));

    let (rows, ..) = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    assert!(
        rows.is_empty(),
        "d29 survived its own bracket, so its pin still holds both ships and nothing changed: \
         {rows:?}"
    );
}

/// The COMMIT is load-bearing. Without it the uncommitted slot from frame 1
/// survives every later frame, and a rejection any number of frames later rolls
/// the pin back past state the fold has already applied.
///
/// What production change makes this fail: deleting the
/// `commit_delta_pin_transactions` call at the end of the frame builder, or
/// making it conditional on the frame having rejected nothing.
#[test]
fn a_committed_pin_is_not_rolled_back_by_a_later_frames_rejection() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10)], 0);
    settle_frame(&mut trackers, &rejected(&[]));

    // Frame 2 — accepted, and it adds ship 2. The fold HAS this.
    let (rows, ..) = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    assert_eq!(live_ids(&rows), vec![2]);
    settle_frame(&mut trackers, &rejected(&[]));

    // Frame 3 — d29 rejected, and its reader never ran (a bracket torn before
    // the read is the common shape). Nothing is staged, so nothing rolls back.
    settle_frame(&mut trackers, &rejected(&[D29]));

    let (rows, ..) = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    assert!(
        rows.is_empty(),
        "frame 2 was committed, so the frame-3 rejection must not roll the pin back to \
         frame 1's state and re-send ship 2: {rows:?}"
    );
}

/// The very first frame of a capture, rejected. There is no committed pin yet,
/// so the rollback must return the tracker to EMPTY and the next accepted frame
/// must emit a keyframe — the fold has been given nothing to merge into.
///
/// What production change makes this fail: seeding `uncommitted_pin` with the
/// staged value rather than the pre-advance one, which would leave the first
/// frame's rows pinned and make the next frame emit a patch the fold would
/// refuse to reconstruct.
#[test]
fn a_rejected_first_frame_rolls_the_pin_back_to_empty() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    settle_frame(&mut trackers, &rejected(&[D29]));
    assert!(
        trackers.ships.previous.is_empty(),
        "nothing was ever committed, so the rollback target is the empty pin"
    );

    let (rows, _changed, deleted) =
        compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    assert_eq!(live_ids(&rows), vec![1, 2], "the next frame is a keyframe");
    assert_eq!(deleted, 0, "a keyframe carries no tombstones");
}

/// Two advances inside ONE frame must roll back to the state BEFORE the first
/// of them, not to the state between them.
///
/// The frame builder reads each domain once today, so this is the guard on the
/// guard: `stage_delta_pin_advance` fills its slot only when it is empty, and
/// an unconditional assignment would make the second advance the rollback
/// target — a state the fold never received either.
///
/// What production change makes this fail: dropping the `if
/// uncommitted.is_none()` condition in `stage_delta_pin_advance` — the only
/// stager, now that d32 is off the pin.
///
/// MUTATION RUN: both guards removed, leaving a bare
/// `*uncommitted = Some(previous);`.
/// OBSERVED: FAILED -- "assertion `left == right` failed: the rollback target
/// is the last COMMITTED pin, so both ships staged inside the rejected frame
/// must be delivered: [...]
///   left: [3]
///  right: [2, 3]"
#[test]
fn two_advances_in_one_frame_roll_back_past_both() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10)], 0);
    settle_frame(&mut trackers, &rejected(&[]));

    // One frame, two reads. Neither is published.
    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    let _ = compact_ship_delta_rows(
        &mut trackers.ships,
        vec![ship(1, 10), ship(2, 20), ship(3, 30)],
        0,
    );
    settle_frame(&mut trackers, &rejected(&[D29]));

    let (rows, ..) = compact_ship_delta_rows(
        &mut trackers.ships,
        vec![ship(1, 10), ship(2, 20), ship(3, 30)],
        0,
    );
    assert_eq!(
        live_ids(&rows),
        vec![2, 3],
        "the rollback target is the last COMMITTED pin, so both ships staged inside the \
         rejected frame must be delivered: {rows:?}"
    );
}

/// The tests above drive `reset_trackers_after_rejected_bracket` and
/// `commit_delta_pin_transactions` directly, so none of them would notice the
/// frame builder losing its call to the commit. This is what notices.
///
/// The commit MUST be unconditional and MUST follow the conditional reset: the
/// reset rolls the rejected domains back and clears their slots, and an
/// uncommitted slot left behind is the state a LATER rejection would roll back
/// to, over frames the fold has already applied.
///
/// What production change makes this fail: deleting the call, moving it above
/// the reset, or wrapping it in the `if rejected_any` block.
///
/// MUTATION RUN: the call moved INSIDE the `if rejected_any` block, directly
/// after `reset_trackers_after_rejected_bracket`.
/// OBSERVED: FAILED -- "the frame builder must call
/// `commit_delta_pin_transactions` unconditionally".
#[test]
fn the_frame_builder_commits_the_delta_pin_transactions_after_the_reset() {
    // CRLF: this repo checks out with CRLF and the patterns below span lines.
    let source = super::tests::collector_source_lf();
    // Both patterns are anchored on the preceding newline so the indentation
    // is exact: without the anchor, `"    commit..."` also matches the
    // eight-space form that would sit INSIDE the `if rejected_any` block.
    let reset_call =
        "\n        reset_trackers_after_rejected_bracket(trackers, &rejected_domains);\n    }\n";
    let commit_call = "\n    commit_delta_pin_transactions(trackers);\n";

    let reset_at = source
        .find(reset_call)
        .expect("the frame builder still closes the `if rejected_any` block with the reset");
    let commit_at = source
        .find(commit_call)
        .expect("the frame builder must call `commit_delta_pin_transactions` unconditionally");
    assert_eq!(
        source.matches(commit_call).count(),
        1,
        "exactly one unconditional commit call, or a frame commits twice"
    );
    assert!(
        commit_at > reset_at,
        "the commit must run AFTER the rejected-bracket reset, or a rolled-back pin is \
         re-committed from the value the reset just discarded"
    );
    // Between the two there must be nothing but the closing brace of the
    // `if rejected_any` block and comments — in particular the commit must not
    // have been pulled INSIDE that block.
    let between = &source[reset_at + reset_call.len()..commit_at];
    assert!(
        !between.contains("if ") && !between.contains("return"),
        "the commit is no longer unconditionally reached from the reset: {between:?}"
    );
}

/// A withheld frame stages nothing, so a rejection in the same frame has
/// nothing to undo and the pin the fold agrees with survives untouched.
///
/// `compact_delta_baseline_withheld` returns before the pin is touched, which
/// means `uncommitted_pin` stays `None`; if the rollback ever cleared the pin
/// unconditionally instead of only when an advance is staged, this would lose a
/// baseline that was never in doubt.
///
/// What production change makes this fail: making `roll_back_delta_pin` assign
/// `BTreeMap::new()` when `uncommitted` is `None`.
#[test]
fn a_withheld_frame_that_is_also_rejected_keeps_the_committed_pin() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    settle_frame(&mut trackers, &rejected(&[]));

    let (rows, ..) = compact_ship_delta_rows(
        &mut trackers.ships,
        vec![ship(1, 10)],
        COMPACT_STATUS_PARTIAL_DATA,
    );
    assert!(rows.is_empty(), "a withheld frame emits nothing");
    settle_frame(&mut trackers, &rejected(&[D29]));

    let (rows, ..) = compact_ship_delta_rows(&mut trackers.ships, vec![ship(1, 10), ship(2, 20)], 0);
    assert!(
        rows.is_empty(),
        "the withheld frame never advanced the pin, so the committed pin still holds both \
         ships and nothing changed: {rows:?}"
    );
}

/// **The property the d32 exclusion protects**: d30 and d32 renumber their
/// generations IDENTICALLY across a shared rejection, so every d32 loadout
/// still has a d30 wing to hang on.
///
/// This is the case that actually happens. Measured over
/// `runs/live_unlock/t26/collector_stream.bin`, d30 and d32 are gapped together
/// in 358 of 359 frames and never separately, both renumber densely from
/// generation 0 on every post-gap record, and the fold prunes 0 orphan rows.
/// The lockstep is not a coincidence: both trackers take the same blanket reset
/// and both then re-mint from an empty pin in the same body order.
///
/// A one-sided repair breaks exactly this. It is written as a positive gate
/// rather than an absence check so the pairing is asserted, not just described.
///
/// What production change makes this fail: putting d32 (or d30) alone on the
/// transactional pin, or otherwise making one of the two keep generations the
/// other re-mints.
///
/// MUTATION RUN: d32 put back on a one-sided rollback carve-out in
/// `reset_trackers_after_rejected_bracket`.
/// OBSERVED: FAILED -- "assertion `left == right` failed: d30 and d32 disagree
/// on the post-gap generations, so every d32 loadout would be deleted by
/// `prune_air_wing_loadout_orphans_for_current_hour`: d30 [(1, 700, 0),
/// (1, 701, 1)] vs d32 []
///   left: [(1, 700, 0), (1, 701, 1)]
///  right: []"
/// (`d32_is_not_on_the_transactional_pin_until_d30_joins_it` failed on the same
/// mutation with "d32 joined the rejected-bracket rollback carve-out.")
#[test]
fn d30_and_d32_renumber_identically_across_a_shared_rejection() {
    let mut trackers = CompactPerfGateTrackers::default();
    let wing_a = HEAP_LO + 0x100;
    let wing_b = HEAP_LO + 0x200;

    fn d30_row(wing_id: u32, body_addr: usize) -> CompactAirWingRow {
        CompactAirWingRow {
            owner_tag_db_id: 1,
            wing_id,
            generation: 0,
            body_addr,
            deleted: false,
            count: 77,
            mission_mask: 1,
            mission_region: Some(88),
            effective_planes: 70,
        }
    }
    let d30_a = d30_row(700, wing_a);
    let d30_b = d30_row(701, wing_b);
    let d32_a = loadout(700, wing_a, 100_000);
    let d32_b = loadout(701, wing_b, 50_000);

    // The exact triple the fold's `AirWingKey` is built from, on both sides.
    fn d30_keys(rows: &[CompactAirWingRow]) -> Vec<(u32, u32, u64)> {
        rows.iter()
            .map(|row| (row.owner_tag_db_id, row.wing_id, row.generation))
            .collect()
    }
    fn d32_keys(rows: &[CompactAirWingLoadoutRow]) -> Vec<(u32, u32, u64)> {
        rows.iter()
            .map(|row| (row.owner_tag_db_id, row.wing_id, row.generation))
            .collect()
    }

    // Frame 1 — both see wing A, both publish, both are accepted.
    let (d30_rows, ..) = compact_air_wing_delta_rows(&mut trackers.air_wings, vec![d30_a.clone()], 0);
    let (d32_rows, ..) = compact_air_wing_loadout_delta_rows(
        &mut trackers.air_wing_loadouts,
        vec![d32_a.clone()],
        0,
    );
    assert_eq!(
        d30_keys(&d30_rows),
        d32_keys(&d32_rows),
        "the two domains must agree from the start, or nothing below means anything"
    );
    settle_frame(&mut trackers, &rejected(&[]));

    // Frame 2 — wing B appears and BOTH brackets are rejected. This is the
    // shape t26 shows 358 times.
    let _ = compact_air_wing_delta_rows(
        &mut trackers.air_wings,
        vec![d30_a.clone(), d30_b.clone()],
        0,
    );
    let _ = compact_air_wing_loadout_delta_rows(
        &mut trackers.air_wing_loadouts,
        vec![d32_a.clone(), d32_b.clone()],
        0,
    );
    settle_frame(&mut trackers, &rejected(&[COMPACT_DOMAIN_AIR_WINGS, D32]));

    // Frame 3 — the first accepted frame after the shared gap.
    let (d30_rows, ..) =
        compact_air_wing_delta_rows(&mut trackers.air_wings, vec![d30_a, d30_b], 0);
    let (d32_rows, ..) =
        compact_air_wing_loadout_delta_rows(&mut trackers.air_wing_loadouts, vec![d32_a, d32_b], 0);

    let after_d30 = d30_keys(&d30_rows);
    let after_d32 = d32_keys(&d32_rows);
    assert_eq!(after_d30.len(), 2, "d30 re-publishes both wings after the gap");
    assert_eq!(
        after_d30, after_d32,
        "d30 and d32 disagree on the post-gap generations, so every d32 loadout would be \
         deleted by `prune_air_wing_loadout_orphans_for_current_hour`: d30 {after_d30:?} vs \
         d32 {after_d32:?}"
    );
}
