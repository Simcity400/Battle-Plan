//! Gates for the TRANSACTIONAL delta pin on d29 `ships`, and for the air-wing
//! transactional UNIT — d30 `air_wings`, d32 `air_wing_loadouts` and d42
//! `air_combat_losses_live`.
//!
//! D29 is a keyed diff with tombstones, and its consumer runs
//! `world_fold::replace_or_sparse_after_first`, which returns SPARSE for every
//! record after the first. The only thing that ever removes a key from the
//! folded world is therefore an explicit tombstone row.
//!
//! **D30/D32/D42 have the identical defect and none of them could be repaired
//! alone.** The fold keys all three on the same
//! `AirWingKey { owner_tag_db_id, wing_id, generation }`, and
//! `world_fold::prune_air_wing_loadout_orphans_for_current_hour` deletes any d32
//! row with no d30 counterpart. Before the unit the three numbers agreed only
//! because three independent counters re-minted densely from zero after the same
//! blanket reset, so rolling one pin back stopped that domain renumbering while
//! the others still did — and the prune then deleted every d32 row, on 358 of
//! t26's 359 gapped frames.
//!
//! The repair is `CompactAirWingGenerationAuthority` plus one joint rollback:
//! the generation belongs to the wing BODY rather than to the domain looking at
//! it, and all three pins advance and roll back together under
//! `AIR_WING_TRANSACTIONAL_UNIT`. The authority itself is deliberately NOT
//! rolled back — see `the_generation_authority_is_not_rolled_back_with_the_pins`.
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
use super::*;

const D29: u16 = COMPACT_DOMAIN_SHIPS;
/// The other two members of the air-wing transactional unit. `d30` is spelled
/// `COMPACT_DOMAIN_AIR_WINGS` at its use sites.
const D32: u16 = COMPACT_DOMAIN_AIR_WING_LOADOUTS;
const D42: u16 = COMPACT_DOMAIN_AIR_COMBAT_LOSSES_LIVE;
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

/// **The unit is ATOMIC in the source, not only in one test's frame sequence.**
///
/// D32 could never be repaired alone: its generation is shared with d30 through
/// the fold (both keyed on `AirWingKey { owner_tag_db_id, wing_id, generation }`,
/// with `world_fold::prune_air_wing_loadout_orphans_for_current_hour` deleting
/// any d32 row that has no d30 counterpart), and a one-sided rollback stopped
/// d32 renumbering while d30 still did — on 358 of t26's 359 gapped frames.
///
/// The repair is the UNIT. This gate holds the three structural halves of it,
/// because each one is individually removable without any behavioural test in
/// this file going red for the RIGHT reason:
///
///  1. all three domains are on `commit_delta_pin_transactions`;
///  2. all three roll back through the ONE joint carve-out keyed on
///     `AIR_WING_TRANSACTIONAL_UNIT`, not through three independent ones;
///  3. none of the three mints a generation from a private counter any more —
///     the only mint site is `generations.generation_for(..)`.
///
/// What production change makes this fail: dropping any one domain from the
/// commit or the carve-out, splitting the joint carve-out into per-domain ones,
/// or reintroducing a `tracker.next_generation` counter on any of the three.
///
/// MUTATION RUN: see `the_air_wing_unit_rolls_back_as_one_when_any_member_is_rejected`.
#[test]
fn the_air_wing_unit_is_one_transaction_in_the_source() {
    // CRLF: this repo checks out with CRLF and the patterns below span lines.
    let source = super::tests::collector_source_lf();
    for call in [
        "trackers.air_wings.commit_pin()",
        "trackers.air_wing_loadouts.commit_pin()",
        "trackers.air_combat_losses_live.commit_pin()",
    ] {
        assert!(
            source.contains(call),
            "`{call}` is missing from `commit_delta_pin_transactions`. All three members of \
             the air-wing unit commit together or none of them may: a member left uncommitted \
             is rolled back by a LATER frame's rejection, past state the fold already applied."
        );
    }
    assert!(
        source.contains(
            "\nconst AIR_WING_TRANSACTIONAL_UNIT: &[u16] = &[\n    \
             COMPACT_DOMAIN_AIR_WINGS,\n    COMPACT_DOMAIN_AIR_WING_LOADOUTS,\n    \
             COMPACT_DOMAIN_AIR_COMBAT_LOSSES_LIVE,\n];\n"
        ),
        "the air-wing unit's membership list changed. It is the set of domains the fold keys \
         on one `AirWingKey`; changing it is a fold-side claim, not a producer-side one."
    );
    // The ROLLBACK is PER-MEMBER, and that is not a weakening of the unit: the
    // shared authority is what makes the keys agree, so a member may roll back
    // alone. Rolling a member back because a SIBLING was rejected is the bug
    // `an_air_wing_member_that_published_keeps_its_pin` holds -- the fold
    // APPLIED that member's record, so rewinding leaves the pin BEHIND it.
    for member in [
        "COMPACT_DOMAIN_AIR_WINGS",
        "COMPACT_DOMAIN_AIR_WING_LOADOUTS",
        "COMPACT_DOMAIN_AIR_COMBAT_LOSSES_LIVE",
    ] {
        let one_line =
            format!("if !domain_scoped_tracker_tag_survived({member}, rejected_domains) {{");
        let wrapped = format!(
            "if !domain_scoped_tracker_tag_survived(\n            {member},\n            \
             rejected_domains,\n        ) {{"
        );
        assert!(
            source.contains(&one_line) || source.contains(&wrapped),
            "{member} no longer rolls back on ITS OWN rejection. Every member's pin must be \
             restored exactly when its own records became a `DOMAIN_GAP`, and never because a \
             sibling's did."
        );
    }
    // No private counters left. `next_generation` survives ONLY as the shared
    // authority's own field and accessor.
    let private_counter_sites = [
        "next_compact_air_wing_generation",
        "next_compact_air_wing_loadout_generation",
        "next_compact_air_strength_generation",
    ];
    for site in private_counter_sites {
        assert!(
            !source.contains(site),
            "`{site}` is back. A per-domain generation counter is what made d30/d32 agree by \
             coincidence; the agreement must come from `CompactAirWingGenerationAuthority`."
        );
    }
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

// ---------------------------------------------------------------------
// The air-wing transactional unit: d30 `air_wings`, d32 `air_wing_loadouts`,
// d42 `air_combat_losses_live`
// ---------------------------------------------------------------------

const WING_A: usize = HEAP_LO + 0x100;
const WING_B: usize = HEAP_LO + 0x200;

fn wing(wing_id: u32, body_addr: usize, count: u32) -> CompactAirWingRow {
    CompactAirWingRow {
        owner_tag_db_id: 1,
        wing_id,
        generation: 0,
        body_addr,
        deleted: false,
        count,
        mission_mask: 1,
        mission_region: Some(88),
        effective_planes: count,
    }
}

fn strength(wing_id: u32, body_addr: usize, count: u32) -> CompactAirStrengthSample {
    CompactAirStrengthSample {
        owner_tag_db_id: 1,
        wing_id,
        generation: 0,
        body_addr,
        count,
        mission_mask: 1,
        mission_region: Some(88),
        equipment_amount_sum: (count as i64) * COMPACT_AIR_STRENGTH_EQUIPMENT_AMOUNT_PER_PLANE,
        air_accidents: 0,
        experience_raw: 0,
    }
}

/// The exact triple the fold's `AirWingKey` is built from.
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

/// Run one frame's d30/d32/d42 reads against the trackers, in the order the
/// frame builder does.
fn air_unit_frame(
    trackers: &mut CompactPerfGateTrackers,
    wings: Vec<CompactAirWingRow>,
    loadouts: Vec<CompactAirWingLoadoutRow>,
    strengths: Vec<CompactAirStrengthSample>,
) -> (
    Vec<CompactAirWingRow>,
    Vec<CompactAirWingLoadoutRow>,
    Vec<CompactAirStrengthDeltaRow>,
) {
    // Exactly the split the frame builder takes: a shared borrow of the
    // authority alongside a `&mut` of each domain's own tracker. They are
    // disjoint field projections of one struct, which is what lets three
    // concurrent readers share it in production.
    let generations = &trackers.air_wing_generations;
    let (d30_rows, ..) = compact_air_wing_delta_rows(&mut trackers.air_wings, generations, wings, 0);
    let (d32_rows, ..) = compact_air_wing_loadout_delta_rows(
        &mut trackers.air_wing_loadouts,
        generations,
        loadouts,
        0,
    );
    let (d42_rows, ..) = compact_air_strength_delta_rows(
        &mut trackers.air_combat_losses_live,
        generations,
        strengths,
        0,
    );
    (d30_rows, d32_rows, d42_rows)
}

/// **The gate d32 never had.** A loadout that vanished while d32 was gapped is
/// TOMBSTONED by the first emission after the gap.
///
/// This is d29's defect on d32, and the reason it could not be fixed alone is in
/// `the_air_wing_unit_is_one_transaction_in_the_source`. With the whole unit on
/// one pin the rollback is available, and this is the row it buys: before it,
/// the post-gap record was a whole snapshot, `emit_full_snapshot` skipped the
/// tombstone sweep, and the fold — which merges every record after the first —
/// kept the vanished loadout forever.
///
/// What production change makes this fail: default-constructing d32's tracker
/// instead of rolling it back — which is precisely the shipped behaviour this
/// repairs, since d32 used to take the blanket reset.
///
/// MUTATION RUN: d32 dropped out of the joint carve-out (`let mut loadouts =
/// CompactAirWingLoadoutTracker::default();`), i.e. back on the blanket reset.
/// OBSERVED: FAILED --
/// "assertion `left == right` failed: the first d32 emission after its own gap
/// must tombstone the loadout that vanished during the gap; the pre-fix code
/// emitted a whole snapshot with no deletions at all: [CompactAirWingLoadoutRow
/// { owner_tag_db_id: 1, wing_id: 700, generation: 0, body_addr: 1099511628032,
/// deleted: false, air_base_id: 9001, equipment: [CompactAirWingLoadoutEntry {
/// variant_id: 900, amount_raw: 100000 }] }]
///   left: []
///  right: [701]"
///
/// Note the mutation that does NOT fail it: forcing `air_wing_unit_survived` to
/// `true`. A pin that is never rolled back is also never wiped, so the tombstone
/// sweep still runs. The two halves of the repair are independent and each has
/// its own gate.
#[test]
fn d32_tombstones_a_loadout_that_vanished_across_its_own_gap() {
    let mut trackers = CompactPerfGateTrackers::default();

    // Frame 1 — wings A and B are published, so the fold holds both.
    let (_, d32_rows, _) = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10), wing(701, WING_B, 20)],
        vec![loadout(700, WING_A, 100_000), loadout(701, WING_B, 50_000)],
        vec![strength(700, WING_A, 10), strength(701, WING_B, 20)],
    );
    assert_eq!(d32_keys(&d32_rows).len(), 2);
    settle_frame(&mut trackers, &rejected(&[]));

    // Frame 2 — the unit's brackets are rejected. Its records became gaps.
    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10), wing(701, WING_B, 20)],
        vec![loadout(700, WING_A, 100_000), loadout(701, WING_B, 50_000)],
        vec![strength(700, WING_A, 10), strength(701, WING_B, 20)],
    );
    settle_frame(
        &mut trackers,
        &rejected(&[COMPACT_DOMAIN_AIR_WINGS, D32, D42]),
    );

    // Frame 3 — the first accepted read after the gap, and wing B is gone.
    let (d30_rows, d32_rows, _) = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10)],
        vec![loadout(700, WING_A, 100_000)],
        vec![strength(700, WING_A, 10)],
    );

    let d32_deleted: Vec<u32> = d32_rows
        .iter()
        .filter(|row| row.deleted)
        .map(|row| row.wing_id)
        .collect();
    assert_eq!(
        d32_deleted,
        vec![701],
        "the first d32 emission after its own gap must tombstone the loadout that vanished \
         during the gap; the pre-fix code emitted a whole snapshot with no deletions at all: \
         {d32_rows:?}"
    );
    let d30_deleted: Vec<u32> = d30_rows
        .iter()
        .filter(|row| row.deleted)
        .map(|row| row.wing_id)
        .collect();
    assert_eq!(
        d30_deleted,
        vec![701],
        "and d30 must tombstone the same wing, or the fold's orphan prune is the only thing \
         removing the loadout row: {d30_rows:?}"
    );
}

/// **THE MEMBER THAT PUBLISHED KEEPS ITS PIN.** A rollback is scoped to the
/// member whose OWN records became a `DOMAIN_GAP`, never to a sibling's.
///
/// This lane first shipped the opposite rule — all three pins rolled back if ANY
/// member was rejected — on the reasoning that the unit must describe one frame.
/// That was a correctness bug, and this is the sequence that shows it: only d42
/// is rejected, so d30 and d32 PUBLISHED and the fold APPLIED their records.
/// Rewinding their pins leaves them BEHIND the fold, and the next time the value
/// returns to what the rewound pin holds the producer compares equal, emits
/// nothing, and the fold keeps the superseded value for the rest of the capture.
/// (Codex review round 1, finding A1.)
///
/// The unit survives the narrowing because what makes it a unit is the SHARED
/// GENERATION AUTHORITY, not lockstep rewinding: a generation belongs to the
/// wing body, so two members can roll back independently and still emit
/// identical keys. `d30_and_d32_publish_identical_keys_across_a_shared_rejection`
/// is the arm that holds that.
///
/// What production change makes this fail: rolling every member back when any
/// member is rejected.
///
/// MUTATION RUN: the three per-member guards replaced with the original
/// `if !air_wing_unit_survived { ...roll all three... }`.
/// OBSERVED: FAILED -- "assertion `left == right` failed: d32 published this
/// value and the fold applied it, so d32's pin must not be rewound by d42's
/// rejection; a rewound pin calls the reversion unchanged and the fold keeps 20
/// forever: []
///   left: []
///  right: [701]"
#[test]
fn an_air_wing_member_that_published_keeps_its_pin() {
    let mut trackers = CompactPerfGateTrackers::default();

    // Frame 1 — everyone publishes wing B at 20 planes.
    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(701, WING_B, 20)],
        vec![loadout(701, WING_B, 50_000)],
        vec![strength(701, WING_B, 20)],
    );
    settle_frame(&mut trackers, &rejected(&[]));

    // Frame 2 — wing B moves to 30. d42's bracket is rejected; d30 and d32
    // publish, and the fold applies their 30.
    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(701, WING_B, 30)],
        vec![loadout(701, WING_B, 60_000)],
        vec![strength(701, WING_B, 30)],
    );
    settle_frame(&mut trackers, &rejected(&[D42]));

    // Frame 3 — wing B goes back to 20, the value frame 1 published.
    let (d30_rows, d32_rows, _) = air_unit_frame(
        &mut trackers,
        vec![wing(701, WING_B, 20)],
        vec![loadout(701, WING_B, 50_000)],
        vec![strength(701, WING_B, 20)],
    );

    assert_eq!(
        d30_rows
            .iter()
            .map(|row| row.wing_id)
            .collect::<Vec<_>>(),
        vec![701],
        "d30 published 30 and the fold applied it, so its pin must hold 30 and the reversion \
         to 20 must be emitted; a pin rewound by d42's rejection would call it unchanged and \
         the fold would keep 30 forever: {d30_rows:?}"
    );
    assert_eq!(
        d32_rows
            .iter()
            .map(|row| row.wing_id)
            .collect::<Vec<_>>(),
        vec![701],
        "d32 published this value and the fold applied it, so d32's pin must not be rewound by \
         d42's rejection; a rewound pin calls the reversion unchanged and the fold keeps 20 \
         forever: {d32_rows:?}"
    );
}

/// **AND THE REJECTED MEMBER STILL ROLLS BACK.** The control that stops the test
/// above from passing under "nothing ever rolls back".
///
/// What production change makes this fail: dropping the per-member rollback
/// entirely, or default-constructing a rejected member's tracker.
#[test]
fn the_rejected_air_wing_member_rolls_its_own_pin_back() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10)],
        vec![loadout(700, WING_A, 100_000)],
        vec![strength(700, WING_A, 10)],
    );
    settle_frame(&mut trackers, &rejected(&[]));

    // d42 sees wing B for the first time and is rejected: the fold never got it.
    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10), wing(701, WING_B, 20)],
        vec![loadout(700, WING_A, 100_000), loadout(701, WING_B, 50_000)],
        vec![strength(700, WING_A, 10), strength(701, WING_B, 20)],
    );
    settle_frame(&mut trackers, &rejected(&[D42]));

    let (_, _, d42_rows) = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10), wing(701, WING_B, 20)],
        vec![loadout(700, WING_A, 100_000), loadout(701, WING_B, 50_000)],
        vec![strength(700, WING_A, 10), strength(701, WING_B, 20)],
    );
    assert_eq!(
        d42_rows.iter().map(|row| row.wing_id).collect::<Vec<_>>(),
        vec![701],
        "wing B was only ever staged on a frame d42 never delivered, so d42's next accepted \
         frame owes the fold its baseline row: {d42_rows:?}"
    );
}

/// **The shared key.** d30 and d32 publish IDENTICAL `AirWingKey` triples across
/// a shared rejection, so every d32 loadout still has a d30 wing to hang on and
/// `world_fold::prune_air_wing_loadout_orphans_for_current_hour` deletes
/// nothing.
///
/// Before the unit this held only by coincidence — two independent counters
/// re-minting from zero in the same body order after the same blanket reset.
/// Now it holds by construction, and this gate is what says so: the numbers come
/// from one authority keyed on the wing BODY.
///
/// What production change makes this fail: giving either domain its own
/// generation counter again, or keying the authority on anything the two
/// domains do not both see.
#[test]
fn d30_and_d32_publish_identical_keys_across_a_shared_rejection() {
    let mut trackers = CompactPerfGateTrackers::default();

    // Frame 1 — wing A, accepted.
    let (d30_rows, d32_rows, _) = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10)],
        vec![loadout(700, WING_A, 100_000)],
        vec![strength(700, WING_A, 10)],
    );
    assert_eq!(
        d30_keys(&d30_rows),
        d32_keys(&d32_rows),
        "the two domains must agree from the start, or nothing below means anything"
    );
    settle_frame(&mut trackers, &rejected(&[]));

    // Frame 2 — wing B appears and the whole unit is gapped. This is the shape
    // t26 shows 358 times.
    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10), wing(701, WING_B, 20)],
        vec![loadout(700, WING_A, 100_000), loadout(701, WING_B, 50_000)],
        vec![strength(700, WING_A, 10), strength(701, WING_B, 20)],
    );
    settle_frame(
        &mut trackers,
        &rejected(&[COMPACT_DOMAIN_AIR_WINGS, D32, D42]),
    );

    // Frame 3 — the first accepted frame after the shared gap.
    let (d30_rows, d32_rows, _) = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10), wing(701, WING_B, 20)],
        vec![loadout(700, WING_A, 100_000), loadout(701, WING_B, 50_000)],
        vec![strength(700, WING_A, 10), strength(701, WING_B, 20)],
    );

    let after_d30 = d30_keys(&d30_rows);
    let after_d32 = d32_keys(&d32_rows);
    assert_eq!(
        after_d30.len(),
        1,
        "only wing B moved across the gap, so only wing B is re-published: {d30_rows:?}"
    );
    assert_eq!(
        after_d30, after_d32,
        "d30 and d32 disagree on the post-gap generations, so every d32 loadout would be \
         deleted by `prune_air_wing_loadout_orphans_for_current_hour`: d30 {after_d30:?} vs \
         d32 {after_d32:?}"
    );
}

/// **The key-count leak.** A wing that survives a rejected frame keeps the SAME
/// generation, so the fold overwrites its row in place instead of accumulating a
/// second copy under a fresh number.
///
/// This is the arm that shrinks t26's 2,310 `air_wings` keys. Before the shared
/// authority, every post-gap record renumbered densely from zero and the fold —
/// which only ever removes a key that is explicitly tombstoned — merged a
/// freshly-numbered copy of every wing on top of the old ones.
///
/// The wing set is deliberately REORDERED between the two frames, because dense
/// re-minting is order-dependent and a test that always presents the same order
/// would pass under the old code too.
///
/// What production change makes this fail: THE SHIPPED BEHAVIOUR — d30's pin
/// default-constructed on a rejected bracket while the generation counter also
/// restarts. Either half alone is survivable (a restored pin carries the
/// generations in `old.generation`; a persistent authority re-mints the same
/// numbers into an emptied pin), which is why the mutation below removes both:
/// it is the pre-fix encoder, exactly.
///
/// MUTATION RUN: `air_wing_generations` dropped from the reset's carry-over AND
/// d30's tracker default-constructed instead of rolled back.
/// OBSERVED: FAILED --
/// "assertion `left == right` failed: a wing that survived the gap was handed a
/// NEW generation, so the fold keeps its old row forever: before [(1, 700, 0),
/// (1, 701, 1)] after [(1, 700, 1), (1, 701, 0)]
///   left: [(1, 700, 1), (1, 701, 0)]
///  right: [(1, 700, 0), (1, 701, 1)]"
///
/// That is the renumbering itself: the two wings swapped generations because the
/// walk returned them in the other order, and the fold — which removes only a
/// tombstoned key — would then hold four keys for two wings.
#[test]
fn a_wing_keeps_its_generation_across_a_rejected_frame() {
    let mut trackers = CompactPerfGateTrackers::default();

    let (d30_rows, ..) = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10), wing(701, WING_B, 20)],
        vec![loadout(700, WING_A, 100_000), loadout(701, WING_B, 50_000)],
        vec![strength(700, WING_A, 10), strength(701, WING_B, 20)],
    );
    let mut before = d30_keys(&d30_rows);
    before.sort_unstable();
    settle_frame(&mut trackers, &rejected(&[]));

    // The unit is gapped. The next walk returns the same wings in the OPPOSITE
    // order — the case a dense re-mint gets wrong — and with moved counts, so
    // both rows are a change and both must appear on frame 3.
    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(701, WING_B, 21), wing(700, WING_A, 11)],
        vec![loadout(701, WING_B, 50_001), loadout(700, WING_A, 100_001)],
        vec![strength(701, WING_B, 21), strength(700, WING_A, 11)],
    );
    settle_frame(
        &mut trackers,
        &rejected(&[COMPACT_DOMAIN_AIR_WINGS, D32, D42]),
    );

    let (d30_rows, ..) = air_unit_frame(
        &mut trackers,
        vec![wing(701, WING_B, 21), wing(700, WING_A, 11)],
        vec![loadout(701, WING_B, 50_001), loadout(700, WING_A, 100_001)],
        vec![strength(701, WING_B, 21), strength(700, WING_A, 11)],
    );
    let mut after = d30_keys(&d30_rows);
    after.sort_unstable();
    assert_eq!(
        after, before,
        "a wing that survived the gap was handed a NEW generation, so the fold keeps its old \
         row forever: before {before:?} after {after:?}"
    );
}

/// **The deliberate asymmetry.** The pins roll back; the generation authority
/// does not.
///
/// A generation is an opaque identity token — nothing downstream reads its value,
/// only its equality — so a number reserved on a frame that became a
/// `DOMAIN_GAP` costs one unused integer. Rolling the assignment back would cost
/// correctness: the body would be re-minted on the next accepted frame in
/// whatever order that walk produced, and a row the fold already holds under the
/// old number would be orphaned with no tombstone to remove it.
///
/// What production change makes this fail: adding the authority to the unit's
/// rollback in `reset_trackers_after_rejected_bracket`, or dropping its
/// carry-over so it takes the blanket reset.
///
/// MUTATION RUN: `trackers.air_wing_generations = air_wing_generations;`
/// replaced with `let _ = air_wing_generations;` in
/// `reset_trackers_after_rejected_bracket`.
/// OBSERVED: FAILED --
/// "assertion `left == right` failed: wing B's assignment survives the
/// rejection: the number is bound to the BODY, not to the frame that first saw
/// it
///   left: 0
///  right: 2"
#[test]
fn the_generation_authority_is_not_rolled_back_with_the_pins() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10)],
        vec![loadout(700, WING_A, 100_000)],
        vec![strength(700, WING_A, 10)],
    );
    settle_frame(&mut trackers, &rejected(&[]));
    assert_eq!(trackers.air_wing_generations.assigned_count(), 1);

    // Wing B is seen on a frame the whole unit then loses.
    let _ = air_unit_frame(
        &mut trackers,
        vec![wing(700, WING_A, 10), wing(701, WING_B, 20)],
        vec![loadout(700, WING_A, 100_000), loadout(701, WING_B, 50_000)],
        vec![strength(700, WING_A, 10), strength(701, WING_B, 20)],
    );
    settle_frame(
        &mut trackers,
        &rejected(&[COMPACT_DOMAIN_AIR_WINGS, D32, D42]),
    );

    assert_eq!(
        trackers.air_wing_generations.assigned_count(),
        2,
        "wing B's assignment survives the rejection: the number is bound to the BODY, not to \
         the frame that first saw it"
    );
    assert!(
        trackers.air_wing_generations.next_generation() >= 2,
        "and the counter does not rewind, so no later body can be handed a number an earlier \
         one already used"
    );
}

// ---------------------------------------------------------------------
// D31's producer use of COMPACT_STATUS_BASELINE_CLEAR
// ---------------------------------------------------------------------
//
// D31 is a merge-after-first domain with NO transactional pin, so its own
// rejection wipes a pin the fold has a baseline for, and the whole payload it
// emits next is MERGED -- a bucket that emptied during the gap is never removed.
// The wire primitive is the right answer for this rather than a transaction:
// d31's rows are
// AGGREGATES recomputed from the whole wing walk, so "here is the whole truth,
// replace what you have" is a claim it can always make.

const D31: u16 = COMPACT_DOMAIN_AIR_REGION_ACTIVITY;

fn d31_bucket(region_id: u32, plane_count: u32) -> CompactAirRegionActivityRow {
    CompactAirRegionActivityRow {
        region_id,
        owner_tag_db_id: 1,
        mission_mask: 7,
        deleted: false,
        wing_count: 1,
        plane_count,
        effective_planes: plane_count,
    }
}

/// **THE PRODUCER USE.** After d31's own rejection wiped a pin it had emitted
/// from, the next record carries `BASELINE_CLEAR`.
///
/// This is what makes the whole payload a replacement instead of a merge, and it
/// is the only thing that removes the bucket that emptied while the domain was
/// gapped: d31 has no baseline left to sweep for tombstones on that frame, which
/// is exactly why the tombstone route is unavailable here.
///
/// What production change makes this fail: not arming `resync_owed` in
/// `reset_trackers_after_rejected_bracket`, or not reading it at the emission
/// site.
///
/// MUTATION RUN: `d31_resync_owed` forced to `false`.
/// OBSERVED: FAILED -- "the wipe destroyed a pin the fold has a baseline for;
/// that is what owes a resync" (the debt is never armed, so the emission-site
/// arm below is never reached either).
#[test]
fn d31_asks_for_a_resync_after_its_own_bracket_was_rejected() {
    let mut trackers = CompactPerfGateTrackers::default();

    // Frame 1: two buckets are published, so the fold holds both.
    let (rows, ..) = compact_air_region_activity_delta_rows(
        &mut trackers.air_region_activity,
        vec![d31_bucket(88, 100), d31_bucket(99, 50)],
        0,
    );
    assert_eq!(rows.len(), 2);
    commit_delta_pin_transactions(&mut trackers);

    // Frame 2: d31's own bracket is rejected. Its record became a `DOMAIN_GAP`
    // and its pin is wiped, so it now owes a resync.
    let rejected: BTreeSet<u16> = [D31].into_iter().collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
    commit_delta_pin_transactions(&mut trackers);
    assert!(
        trackers.air_region_activity.baseline_published,
        "frame 1 published, so the fold holds a d31 baseline"
    );
    assert!(
        trackers.air_region_activity.resync_owed,
        "the wipe destroyed a pin the fold has a baseline for; that is what owes a resync"
    );

    // Frame 3: the first accepted read after the gap, and bucket 99 is gone.
    let status = d31_status_after_read(&mut trackers, vec![d31_bucket(88, 140)]);
    assert_ne!(
        status & compact_contract::COMPACT_STATUS_BASELINE_CLEAR,
        0,
        "the first record after d31's own gap must ask the fold to REPLACE, or the bucket \
         that emptied during the gap is merged back in and never removed: status {status:#x}"
    );
    assert_eq!(
        status & COMPACT_STATUS_DELTA_DATA,
        0,
        "a resync is a whole payload; `BASELINE_CLEAR | DELTA_DATA` is refused by the contract"
    );
    assert!(
        !trackers.air_region_activity.resync_owed,
        "the debt is discharged when the bit reaches the wire"
    );

    // And the next ordinary frame does NOT repeat it: rule 2 of the bit is that
    // a routine keyframe is not a resync, and neither is a routine delta.
    let status = d31_status_after_read(&mut trackers, vec![d31_bucket(88, 141)]);
    assert_eq!(
        status & compact_contract::COMPACT_STATUS_BASELINE_CLEAR,
        0,
        "the resync is one record, not a mode: {status:#x}"
    );
}

/// **RULE 2, THE HALF THAT COSTS SOMETHING.** A stream whose d31 records have
/// never REACHED the fold owes no resync, however full its pin looks.
///
/// The witness is `baseline_published`, not "the pin is non-empty", and the
/// difference is a real sequence rather than a nicety: the reader advances
/// `previous` during the read, so a stream's FIRST d31 read that is then
/// REJECTED leaves a non-empty pin behind while the fold has received nothing at
/// all. Arming on the pin would badge the next record with a bit older readers
/// hard-reject, to clear a baseline that does not exist. (Codex review round 1,
/// finding C3.)
///
/// What production change makes this fail: arming `resync_owed` from the pin's
/// emptiness instead of from the publication witness.
///
/// MUTATION RUN: `d31_resync_owed`'s witness reverted to
/// `!trackers.air_region_activity.previous.is_empty()`.
/// OBSERVED: FAILED -- "a stream whose only d31 read was REJECTED has published
/// nothing, so there is no fold baseline to clear: status 0x8000"
#[test]
fn a_first_d31_record_is_a_keyframe_and_not_a_resync() {
    let mut trackers = CompactPerfGateTrackers::default();

    // The stream's FIRST d31 read builds a full pin -- and is then rejected, so
    // the fold receives nothing. This is the case the pin cannot distinguish.
    let _ = compact_air_region_activity_delta_rows(
        &mut trackers.air_region_activity,
        vec![d31_bucket(88, 100)],
        0,
    );
    assert!(
        !trackers.air_region_activity.previous.is_empty(),
        "the read advanced the pin, or this fixture proves nothing"
    );
    let rejected: BTreeSet<u16> = [D31].into_iter().collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
    commit_delta_pin_transactions(&mut trackers);
    assert!(
        !trackers.air_region_activity.baseline_published,
        "a rejected record reached nothing, so it published nothing"
    );
    assert!(!trackers.air_region_activity.resync_owed);

    let status = d31_status_after_read(&mut trackers, vec![d31_bucket(88, 100)]);
    assert_eq!(
        status & compact_contract::COMPACT_STATUS_BASELINE_CLEAR,
        0,
        "a stream whose only d31 read was REJECTED has published nothing, so there is no fold \
         baseline to clear: status {status:#x}"
    );
}

/// **AN EMPTY CLEAN WHOLE PAYLOAD STILL CARRIES THE RESYNC.** "Every bucket is
/// gone" is a complete replacement state, and it is the case where clearing
/// matters most.
///
/// The first shipped condition also required `changed_rows > 0`, which suppressed
/// the bit exactly when the post-gap walk found nothing: the fold then sparsely
/// merged an empty payload onto a stale bucket and kept it forever. A row count
/// is not a completeness signal on this domain; the status word is. (Codex
/// review round 1, finding C4.)
///
/// What production change makes this fail: reinstating a `changed_rows > 0` (or
/// `!rows.is_empty()`) condition on the resync.
///
/// MUTATION RUN: `&& changed_rows > 0` added back to the resync condition.
/// OBSERVED: FAILED -- "an empty post-gap walk is a complete claim that every
/// bucket is gone, and it is exactly the payload the fold would otherwise merge
/// into nothing: status 0x0"
#[test]
fn an_empty_post_gap_d31_payload_still_asks_for_the_resync() {
    let mut trackers = CompactPerfGateTrackers::default();

    let _ = d31_status_after_read(&mut trackers, vec![d31_bucket(88, 100)]);
    assert!(trackers.air_region_activity.baseline_published);

    let rejected: BTreeSet<u16> = [D31].into_iter().collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
    commit_delta_pin_transactions(&mut trackers);
    assert!(trackers.air_region_activity.resync_owed);

    // The post-gap walk finds no buckets at all.
    let status = d31_status_after_read(&mut trackers, Vec::new());
    assert_ne!(
        status & compact_contract::COMPACT_STATUS_BASELINE_CLEAR,
        0,
        "an empty post-gap walk is a complete claim that every bucket is gone, and it is \
         exactly the payload the fold would otherwise merge into nothing: status {status:#x}"
    );
}

/// **AN UNRELATED REJECTION OWES NOTHING.** d31's pin is on the domain-scoped
/// allowlist, so a frame that rejected somebody else leaves it standing — and a
/// pin that was never wiped cannot have drifted from the fold.
///
/// What production change makes this fail: arming `resync_owed` on every
/// rejection rather than on d31's own.
#[test]
fn an_unrelated_rejection_does_not_make_d31_owe_a_resync() {
    let mut trackers = CompactPerfGateTrackers::default();
    let _ = compact_air_region_activity_delta_rows(
        &mut trackers.air_region_activity,
        vec![d31_bucket(88, 100)],
        0,
    );
    commit_delta_pin_transactions(&mut trackers);

    let rejected: BTreeSet<u16> = [COMPACT_DOMAIN_WEATHER].into_iter().collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
    commit_delta_pin_transactions(&mut trackers);

    assert!(
        !trackers.air_region_activity.resync_owed,
        "d31 survived its own bracket, so its pin still agrees with the fold's"
    );
    assert!(
        !trackers.air_region_activity.previous.is_empty(),
        "and the allowlist retained it, or this fixture proves nothing"
    );
}

/// The reader's status word for one d31 frame, driven through the SAME
/// emission-site logic production uses.
///
/// `read_compact_air_region_activity` needs a live `CollectorState`, which a unit
/// test has no way to build, so this reproduces only the status decision that
/// follows the delta call — and
/// `the_d31_resync_decision_has_exactly_one_production_spelling` pins that this
/// reproduction is the one production runs, rather than a paraphrase of it.
fn d31_status_after_read(
    trackers: &mut CompactPerfGateTrackers,
    rows: Vec<CompactAirRegionActivityRow>,
) -> u32 {
    let mut status_flags = 0u32;
    let tracker = &mut trackers.air_region_activity;
    let delta_frame = !tracker.previous.is_empty();
    let _ = compact_air_region_activity_delta_rows(tracker, rows, status_flags);
    if delta_frame {
        status_flags |= COMPACT_STATUS_DELTA_DATA;
    } else if tracker.resync_owed && !compact_contract::compact_status_is_failure(status_flags) {
        status_flags |= compact_contract::COMPACT_STATUS_BASELINE_CLEAR;
        tracker.resync_owed = false;
    }
    commit_delta_pin_transactions(trackers);
    status_flags
}

/// The helper above is a COPY, and a copy drifts. This pins it against the one
/// production spelling, so a change to the emission site fails here rather than
/// leaving the behavioural gates testing a fossil.
///
/// What production change makes this fail: any edit to the resync condition in
/// `read_compact_air_region_activity` that is not mirrored into
/// `d31_status_after_read`.
#[test]
fn the_d31_resync_decision_has_exactly_one_production_spelling() {
    let source = super::tests::collector_source_lf();
    let condition = "    } else if tracker.resync_owed && \
                     !compact_contract::compact_status_is_failure(status_flags) {\n";
    assert_eq!(
        source.matches(condition).count(),
        1,
        "the d31 resync condition is not where this module's helper copied it from. Update \
         `d31_status_after_read` in the same edit, or the gates above stop describing \
         production."
    );
    assert_eq!(
        source
            .matches("status_flags |= compact_contract::COMPACT_STATUS_BASELINE_CLEAR;")
            .count(),
        1,
        "`BASELINE_CLEAR` is set at exactly one producer site. A second site is a second \
         policy, and the bit's rule 2 -- never on a routine keyframe -- is only checkable \
         where there is one place to check."
    );
}
