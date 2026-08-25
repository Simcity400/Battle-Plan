//! Lane TC: the five small `rc_vanilla4` blockers.
//!
//! One closed defect class across four of them — *"domain N published a
//! failing PERF status and its producer recorded no outcome of its own"*,
//! predicate `outcome_substantiated_from_record [0x00020011]` — on 719 (d29),
//! 74 (d42), 53 (d64) and 4 (d93) frames. The fifth, d38, already routed its
//! word through the ledger and published a reason whose every number said the
//! frame was whole.
//!
//! Each section below pins the property its domain's fix rests on. The
//! emit-site ROUTING itself is pinned once, centrally, by
//! `degraded_frame_tests::d18_d65_d66_and_d91_emit_sites_route_their_status_word_through_the_ledger`,
//! so nothing here re-asserts it.

use super::*;

// --- d38 air_superiority_inputs -----------------------------------------

/// THE GUARD, replaced: the roster set difference, not the row-count proxy.
///
/// `rc_vanilla4`'s one degraded d38 frame reported 661 air-wing rows observed,
/// 0 rejected, 662 loadout rows, 0 variants rejected. It was `PARTIAL_DATA`
/// because 662 > 661, which the sentence contained and did not state.
///
/// The two counts are the sizes of two `BTreeMap`s keyed on the SAME wing body
/// address, so the comparison was a proxy for a set difference — and a
/// strictly weaker one. This pins the direction of that claim from the reason
/// function, which is the only place either number is spoken:
///
/// kills: reinstating `observed_loadout_rows > observed_air_wing_rows` as the
/// guard (the first case below has equal totals and a non-empty difference, so
/// the count comparison publishes it as whole); dropping the
/// `wing_rows_without_a_loadout` half (a later case has FEWER loadouts than
/// wings, which the old guard was silent about while the join still lost that
/// wing's equipment); dropping the guard NAMES, which is what turns nine
/// numbers into a diagnosis; and describing the difference as MISSING BODIES,
/// which is false whenever a body was reallocated rather than removed.
#[test]
fn d38_names_the_guard_that_fired_and_speaks_for_both_sides_of_the_roster() {
    let read = |loadouts_without_wing: u32, wings_without_loadout: u32, invalid: u32| {
        CompactAirSuperiorityInputRead {
            rows: Vec::new(),
            read_time_us: 0,
            changed_rows: 0,
            status_flags: COMPACT_STATUS_PARTIAL_DATA,
            observed_air_wing_rows: 661,
            observed_loadout_rows: 661,
            observed_equipment_rows: 2141,
            observed_variant_rows: 210,
            invalid_air_wing_rows: invalid,
            invalid_variant_rows: 0,
            deleted_rows: 0,
            loadout_rows_without_a_wing: loadouts_without_wing,
            wing_rows_without_a_loadout: wings_without_loadout,
        }
    };

    // EQUAL TOTALS, non-empty difference: one identity created and one
    // destroyed between the two passes. The retired count guard called this
    // frame whole.
    let swap = compact_air_superiority_input_failure_reason(&read(1, 1, 0));
    assert!(
        swap.contains(D38_GUARD_ROSTER_DISAGREEMENT),
        "the roster guard must be named: {swap}"
    );
    assert!(
        swap.contains("1 loadout row(s) whose roster identity (owner, wing id, body) \
                       matched no wing row")
            && swap.contains("1 wing row(s) whose identity matched no loadout row"),
        "both sides of the difference must be counted: {swap}"
    );
    // AND IT MUST NOT SAY "BODY". The same-body identity-change case — a body
    // freed and reallocated to a different wing between the passes — is
    // present in BOTH passes and unmatched all the same, so a sentence about
    // a missing body would name a cause the frame does not have (Codex round
    // 7). The helper's own test drives that case; this is the same numbers
    // reaching the wire.
    let identity_change = compact_air_superiority_roster_disagreement(
        &[CompactAirWingRow {
            owner_tag_db_id: 3,
            wing_id: 100,
            generation: 0,
            body_addr: 0x1000,
            deleted: false,
            count: 4,
            mission_mask: 1,
            mission_region: Some(7),
            effective_planes: 4,
        }],
        &[CompactAirWingLoadoutRow {
            owner_tag_db_id: 3,
            wing_id: 999,
            generation: 0,
            body_addr: 0x1000,
            deleted: false,
            air_base_id: 11,
            equipment: Vec::new(),
        }],
    );
    assert_eq!(identity_change, (1, 1), "the fixture must be the same-body case");
    let reason = compact_air_superiority_input_failure_reason(&read(
        identity_change.0,
        identity_change.1,
        0,
    ));
    assert!(
        !reason.contains("body(ies)"),
        "a body present in both passes must not be reported as missing: {reason}"
    );
    assert!(
        reason.contains("roster identity (owner, wing id, body)"),
        "the sentence must name what actually disagreed: {reason}"
    );

    // WINGS EXCEED LOADOUTS: silent under the retired guard, and exactly as
    // much equipment lost — the join iterates loadouts, so a wing with no
    // loadout row never contributes.
    let wings_only = compact_air_superiority_input_failure_reason(&read(0, 3, 0));
    assert!(
        wings_only.contains(D38_GUARD_ROSTER_DISAGREEMENT),
        "a wing with no loadout counterpart is a loss and must be named: {wings_only}"
    );

    // The other guard, alone, must not borrow the roster guard's name.
    let invalid_only = compact_air_superiority_input_failure_reason(&read(0, 0, 4));
    assert!(
        invalid_only.contains(D38_GUARD_INVALID_ROWS)
            && !invalid_only.contains(D38_GUARD_ROSTER_DISAGREEMENT),
        "a rejected-row frame is not a roster disagreement: {invalid_only}"
    );

    // A failing word with NO guard fired is the sampler refusing before either
    // ran. Saying "no guard fired" would read as a contradiction of the bit.
    let sampler = compact_air_superiority_input_failure_reason(&read(0, 0, 0));
    assert!(
        sampler.contains("refused before either row guard ran"),
        "a word with no guard behind it must name the sampler branch: {sampler}"
    );
}

/// THE GUARD IS THE JOIN'S OWN KEY, driven.
///
/// The test above is handed its facts, so it cannot see the comparison. This
/// drives `compact_air_superiority_roster_disagreement` itself.
///
/// A guard weaker than the join it protects is not a guard, and the first two
/// versions of this one were: row COUNTS, then BODY ADDRESSES. A body freed
/// and reallocated to a different wing between the two passes keeps its
/// address, so the body sets stay equal, the guard stays clean, and
/// `active_wings.get` misses anyway — partial data sealing as complete, which
/// is the one outcome this domain may never produce (Codex round 6).
///
/// kills: keying either set on `body_addr` alone (the third case below has
/// IDENTICAL body sets and a changed identity); dropping the owner or the wing
/// id from either side; computing both counters from one difference, which
/// would make one of them always zero; and reversing a difference.
#[test]
fn d38_roster_disagreement_is_measured_on_the_joins_own_key() {
    fn wing(owner: u32, wing_id: u32, body: usize) -> CompactAirWingRow {
        CompactAirWingRow {
            owner_tag_db_id: owner,
            wing_id,
            generation: 0,
            body_addr: body,
            deleted: false,
            count: 4,
            mission_mask: 1,
            mission_region: Some(7),
            effective_planes: 4,
        }
    }
    fn loadout(owner: u32, wing_id: u32, body: usize) -> CompactAirWingLoadoutRow {
        CompactAirWingLoadoutRow {
            owner_tag_db_id: owner,
            wing_id,
            generation: 0,
            body_addr: body,
            deleted: false,
            air_base_id: 11,
            equipment: Vec::new(),
        }
    }

    // AGREEING ROSTERS: nothing lost, nothing claimed.
    let wings = vec![wing(3, 100, 0x1000), wing(3, 101, 0x2000)];
    let loadouts = vec![loadout(3, 100, 0x1000), loadout(3, 101, 0x2000)];
    assert_eq!(
        compact_air_superiority_roster_disagreement(&wings, &loadouts),
        (0, 0)
    );

    // A LOADOUT WITH NO WING, and a WING WITH NO LOADOUT, at once: equal
    // TOTALS, which the retired count comparison published as whole.
    let wings = vec![wing(3, 100, 0x1000), wing(3, 101, 0x2000)];
    let loadouts = vec![loadout(3, 100, 0x1000), loadout(3, 102, 0x3000)];
    assert_eq!(
        compact_air_superiority_roster_disagreement(&wings, &loadouts),
        (1, 1),
        "one body created and one destroyed between the passes loses equipment twice"
    );

    // THE ROUND-6 CASE: IDENTICAL body sets, changed identity. The body-address
    // version of this guard saw nothing here, and the join misses anyway.
    let wings = vec![wing(3, 100, 0x1000)];
    let loadouts = vec![loadout(3, 999, 0x1000)];
    assert_eq!(
        compact_air_superiority_roster_disagreement(&wings, &loadouts),
        (1, 1),
        "a reallocated body keeps its address; only the join key sees it"
    );
    let loadouts = vec![loadout(9, 100, 0x1000)];
    assert_eq!(
        compact_air_superiority_roster_disagreement(&wings, &loadouts),
        (1, 1),
        "and the OWNER is part of the key too"
    );

    // The two counters are two DIFFERENT differences, not one reported twice.
    let wings = vec![wing(3, 100, 0x1000), wing(3, 101, 0x2000)];
    let loadouts = vec![loadout(3, 100, 0x1000)];
    assert_eq!(
        compact_air_superiority_roster_disagreement(&wings, &loadouts),
        (0, 1),
        "wings exceed loadouts: silent under the retired count guard, and exactly as \
         much equipment lost, because the join iterates loadouts"
    );
    let wings = vec![wing(3, 100, 0x1000)];
    let loadouts = vec![loadout(3, 100, 0x1000), loadout(3, 101, 0x2000)];
    assert_eq!(
        compact_air_superiority_roster_disagreement(&wings, &loadouts),
        (1, 0)
    );

    // THE CONSUMER'S INVARIANT, which this guard is now the only thing
    // upholding.
    //
    // `recorder_host`'s DATA-record authority
    // (`domain_data_authority.rs`) rejects, as a PARSE ERROR, any d38 record
    // whose `observed_loadout_rows` exceeds `observed_air_wing_rows` while the
    // status word carries no failure bit. That is the consumer-side twin of
    // the row-count guard this helper replaced, and it is in another crate: if
    // the reader ever stopped flagging that shape, the recorder would emit
    // records its own host refuses to parse.
    //
    // The implication holds because each roster is ONE ROW PER BODY -- both
    // passes de-duplicate into a `BTreeMap` keyed on `body_addr` -- so
    // `observed_loadout_rows == |loadout_keys|` and likewise for wings, and a
    // set cannot be a subset of a strictly smaller one. More loadout rows than
    // wing rows therefore always leaves at least one loadout key unmatched.
    //
    // kills: any future narrowing of the guard that admits a frame with more
    // loadout rows than wing rows and a clean word.
    for (wing_count, loadout_count) in [(0usize, 1usize), (1, 2), (2, 5), (3, 4)] {
        let wings = (0..wing_count)
            .map(|i| wing(3, 100 + i as u32, 0x1000 + i * 0x10))
            .collect::<Vec<_>>();
        // Bodies chosen so the loadout roster is a SUPERSET where it overlaps:
        // the worst case for the implication, because every shared identity
        // matches and only the surplus can fail.
        let loadouts = (0..loadout_count)
            .map(|i| loadout(3, 100 + i as u32, 0x1000 + i * 0x10))
            .collect::<Vec<_>>();
        let (loadouts_unmatched, _) =
            compact_air_superiority_roster_disagreement(&wings, &loadouts);
        assert!(
            loadouts_unmatched > 0,
            "{loadout_count} loadout rows against {wing_count} wing rows must leave at \
             least one unmatched, or the recorder can emit a CLEAN record that \
             recorder_host's domain_data_authority rejects as a parse error"
        );
    }
}

/// The reader uses that helper, and the helper's key IS the join's key.
///
/// The driven test above cannot see either fact: it could pass while the
/// reader kept a guard of its own, or while the join moved to a different
/// tuple. These are the two source pins that close it.
///
/// kills: reinstating the row-count proxy in the reader; bypassing the helper;
/// and changing the join key on either side without changing the helper.
#[test]
fn d38_reader_delegates_to_the_helper_and_the_keys_match() {
    let source = collector_source_lf();
    let start = source
        .find("fn read_compact_air_superiority_inputs(")
        .expect("the d38 reader is in this file");
    let end = start
        + source[start..]
            .find("\n/// THE ROSTER DISAGREEMENT, over the JOIN'S OWN KEY.")
            .expect("the d38 reader ends before the roster helper");
    let body = &source[start..end];

    // The negative below is about CODE. The reader's own comments name the
    // retired guards so the next reader knows what was replaced and why, and a
    // pin that a comment can break is a pin that gets satisfied by deleting
    // the explanation.
    assert!(
        !body.contains("/*"),
        "this sanitiser models line comments only; a block comment in the d38 reader means \
         it can no longer be trusted and must be extended"
    );
    let code_only: String = body
        .lines()
        .map(|line| match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code_only.contains("observed_loadout_rows > observed_air_wing_rows"),
        "the row-count proxy must not come back: it is silent on a swap and on \
         wings-exceed-loadouts, and both lose equipment"
    );
    assert!(
        code_only.contains(
            "let (loadout_rows_without_a_wing, wing_rows_without_a_loadout) =\n        \
             compact_air_superiority_roster_disagreement(&wing_native.rows, \
             &loadout_native.rows);"
        ),
        "the reader must take both counters from the helper this suite drives"
    );
    assert!(
        code_only.contains(
            "if loadout_rows_without_a_wing > 0 || wing_rows_without_a_loadout > 0 {\n        \
             status_flags |= COMPACT_STATUS_PARTIAL_DATA;"
        ),
        "both directions must set the bit; one alone reinstates the silent half"
    );

    // THE KEYS. The join's two sites and the helper's two sets must build the
    // same tuple, or the guard is measuring something the join does not use.
    assert!(
        code_only.contains("(wing.owner_tag_db_id, wing.wing_id, wing.body_addr),")
            && code_only.contains(
                "active_wings.get(&(loadout.owner_tag_db_id, loadout.wing_id, \
                 loadout.body_addr))"
            ),
        "the join key must still be (owner, wing id, body); if it moved, the helper \
         must move with it"
    );
    let helper_start = source
        .find("fn compact_air_superiority_roster_disagreement(")
        .expect("the roster helper is in this file");
    let helper_end = helper_start
        + source[helper_start..]
            .find("\nfn compact_air_superiority_inputs_source_unavailable_read(")
            .expect("the helper ends before the unavailable read");
    let helper = &source[helper_start..helper_end];
    assert_eq!(
        helper
            .matches("(row.owner_tag_db_id, row.wing_id, row.body_addr)")
            .count(),
        2,
        "both rosters are keyed on the JOIN's key and nothing else"
    );
    assert!(
        !helper.contains(".map(|row| row.body_addr)"),
        "keying on the body address alone is what let a reallocated body pass"
    );
}

// --- d29 ships -----------------------------------------------------------

/// The seven arms `NotATaskForce` conflated, each reachable and each counted.
///
/// `rc_vanilla4` published `naval_oob_walk_not_a_task_force = 17` on all 719
/// frames with every other refusal counter at zero. Lane TC split that into
/// four; `rc_vanilla5` then said `body identity = 17` on all 690 frames, which
/// was still three findings in one bucket -- a class word that disagreed, an
/// id word that would not read, and an id word that is not an identity -- so
/// lane SJ split that arm again. Seven arms now, and `ForeignIdSpace` is
/// deliberately NOT one of them: it is not a `NotATaskForce` answer at all.
///
/// kills: collapsing any two arms onto one variant (the seven map to seven
/// distinct counters here); dropping the total (`not_a_task_force` is still
/// the sum, and `compact_task_force_omissions` does not read the split); and
/// deleting an arm's counter write, which would leave the total unexplained by
/// its own parts.
#[test]
fn d29_not_a_task_force_arms_are_distinct_counters_that_sum_to_the_total() {
    use std::collections::BTreeSet;

    let arms = [
        CompactTaskForceMiss::BodyNotHeap,
        CompactTaskForceMiss::BodyVtable,
        CompactTaskForceMiss::BodyClassUndecided,
        CompactTaskForceMiss::BodyIdUnreadable,
        CompactTaskForceMiss::BodyIdUnusable,
        CompactTaskForceMiss::ParentUnreadable,
        CompactTaskForceMiss::ParentNotAFleetRoot,
    ];

    // Each arm lands in its OWN counter, and in the total. This drives the
    // PRODUCTION mapping (`count_not_a_task_force`), which is why it is a
    // method: a test that restated the match here would pass over any edit to
    // it.
    let mut names = BTreeSet::new();
    for arm in arms {
        let mut walk = CompactTaskForceWalkAccounting::default();
        walk.count_not_a_task_force(arm);

        let split = [
            walk.not_a_task_force_body_not_heap,
            walk.not_a_task_force_body_vtable,
            walk.not_a_task_force_body_class_undecided,
            walk.not_a_task_force_body_id_unreadable,
            walk.not_a_task_force_body_id_unusable,
            walk.not_a_task_force_parent_unreadable,
            walk.not_a_task_force_parent_not_a_fleet_root,
        ];
        assert_eq!(
            split.iter().sum::<u64>(),
            walk.not_a_task_force,
            "{arm:?} must be counted exactly once in the split and once in the total"
        );
        assert_eq!(
            split.iter().filter(|value| **value != 0).count(),
            1,
            "{arm:?} must land in exactly ONE arm counter"
        );
        names.insert(format!("{arm:?}"));
    }
    assert_eq!(names.len(), 7, "the seven arms are seven distinct findings");

    // And each arm is BOUND TO ITS GATE, not merely present once.
    //
    // Counting tokens is not enough: swapping `BodyVtable` with either parent
    // arm leaves every count at one and every arm distinct, and the capture
    // would then send the next lane to RE the lift when the finding was a
    // missing fleet root (Codex round 1, finding 5). The seven gates run in a
    // fixed order, so the pin is the INTERLEAVING — gate, its arm, the next
    // gate, its arm, with no other arm in between.
    let source = collector_source_lf();
    let start = source
        .find("fn validate_compact_task_force_body(")
        .expect("the validator is in this file");
    let end = start
        + source[start..]
            .find("\nfn read_validated_task_force_ship_members_with_window(")
            .expect("the validator ends before the ship-member reader");
    let body = &source[start..end];

    let gated = [
        (
            "if !is_map_control_heap_pointer(body_ptr) {",
            "CompactTaskForceMiss::BodyNotHeap",
        ),
        (
            "CompactTaskForceIdentity::NotTheClass => {",
            "CompactTaskForceMiss::BodyVtable",
        ),
        (
            "CompactTaskForceIdentity::ClassUndecided => {",
            "CompactTaskForceMiss::BodyClassUndecided",
        ),
        (
            "CompactTaskForceIdentity::IdUnreadable => {",
            "CompactTaskForceMiss::BodyIdUnreadable",
        ),
        (
            "CompactTaskForceIdentity::IdUnusable => {",
            "CompactTaskForceMiss::BodyIdUnusable",
        ),
        (
            "snapshot_ptr_or_read(task_force_win, body_ptr, TASK_FORCE_OFF_FLEET_PARENT)",
            "CompactTaskForceMiss::ParentUnreadable",
        ),
        (
            "let Some(fleet_root) = fleet_roots_by_ptr.get(&parent_ptr).copied() else {",
            "CompactTaskForceMiss::ParentNotAFleetRoot",
        ),
    ];
    let mut previous = 0usize;
    for (gate, arm) in gated {
        assert_eq!(
            body.matches(arm).count(),
            1,
            "{arm} must be produced by exactly one gate; two would make the counter unusable"
        );
        let gate_at = body
            .find(gate)
            .unwrap_or_else(|| panic!("the gate `{gate}` must still be in the validator"));
        let arm_at = body.find(arm).expect("counted above");
        assert!(
            previous <= gate_at && gate_at < arm_at,
            "{arm} must be the answer of the gate immediately above it, and the gates \
             must stay in their order"
        );
        // Nothing between this gate and its arm may be another arm.
        let between = &body[gate_at..arm_at];
        assert_eq!(
            between.matches("CompactTaskForceMiss::").count(),
            0,
            "{arm} is separated from its gate by another arm, so the binding is not \
             the one this test reads it as"
        );
        previous = arm_at;
    }
    assert!(
        !body.contains("CompactTaskForceCandidate::NotATaskForce;"),
        "no gate may answer NotATaskForce without naming its arm"
    );

    // And the WALK routes the arm into the counters through that one method,
    // rather than counting the total and dropping the arm on the floor.
    let start = source
        .find("fn collect_compact_task_force_roots_from_fleet_vector(")
        .expect("the vector walk is in this file");
    let end = start
        + source[start..]
            .find("\nfn discover_compact_fleet_roots_from_countries_with_windows(")
            .expect("the vector walk ends before the fleet discovery");
    let walk_body = &source[start..end];
    assert!(
        walk_body.contains("CompactTaskForceCandidate::NotATaskForce(miss) =>")
            && walk_body.contains("walk.count_not_a_task_force(miss);"),
        "the walk must bind the arm and count it; discarding it with `NotATaskForce(_)` \
         restores the single undifferentiated bucket"
    );


    // AND THE GATE BINDING IS NOT THE WHOLE PIN. This test reads source text,
    // which cannot see what `read_compact_task_force_identity` actually reads
    // out of the body; `d29_task_force_identity_reads_the_class_and_the_id_registry_apart`
    // in `tests.rs` drives that function over bytes and is what catches a
    // swapped handle half or a mis-answered unreadable word (Codex round 1,
    // finding 3).
    // AND `ForeignIdSpace` IS NOT ONE OF THE ARMS. It is a separate candidate
    // answer with its own counter, because the elements it names ARE task
    // forces: the class word agreed and only the clausewitz id registry
    // differed (lane SJ, `runs/sj_d29_identity/RE_TASK_FORCE_ID_SPACES.md`).
    // Routing it back through `count_not_a_task_force` would restore the false
    // statement this lane removed.
    assert_eq!(
        body.matches("CompactTaskForceCandidate::ForeignIdSpace").count(),
        1,
        "exactly one gate may answer ForeignIdSpace"
    );
    // ...and the four LOCATOR words are populated where the population is
    // counted. Without this, deleting the four assignments leaves every other
    // assertion green and production advertises a foreign population with no
    // sample to find it by (Codex round 4, finding 5). The bound is positional:
    // each assignment must sit between the arm and the counter increment, which
    // is the only place the FIRST offender of the frame is still known.
    let arm_at = walk_body
        .find("CompactTaskForceCandidate::ForeignIdSpace {")
        .expect("the walk must bind the arm");
    let count_at = walk_body
        .find("walk.foreign_id_space = walk.foreign_id_space.saturating_add(1);")
        .expect("the walk must count the population");
    assert!(arm_at < count_at, "the arm must precede its counter");
    let arm_body = &walk_body[arm_at..count_at];
    // The sample is the FIRST offender of the frame, and the guard is what
    // makes that true: without it a later element overwrites the one the
    // outcome sentence quotes, and every assignment below still matches
    // (Codex round 5, finding 3).
    const GUARD: &str = "if walk.foreign_id_space == 0 {";
    let guard_at = arm_body
        .find(GUARD)
        .expect("the four locator words must be taken under a first-offender guard");
    // The guard's own block, by brace matching, so "assigned after the guard
    // closes" does not pass for "assigned inside it" (Codex round 6, finding 3).
    let open_at = guard_at + GUARD.len() - 1;
    let mut depth = 0usize;
    let mut close_at = None;
    for (offset, ch) in arm_body[open_at..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close_at = Some(open_at + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let guarded = &arm_body[open_at..close_at.expect("the guard block must close")];
    for field in [
        "foreign_id_space_first_type",
        "foreign_id_space_first_object_id",
        "foreign_id_space_first_body",
        "foreign_id_space_first_fleet_id",
    ] {
        assert_eq!(
            arm_body.matches(&format!("walk.{field} = ")).count(),
            1,
            "{field} must be assigned exactly once, inside the arm that counts              the population"
        );
        assert_eq!(
            guarded.matches(&format!("walk.{field} = ")).count(),
            1,
            "{field} must be assigned INSIDE the first-offender guard; assigned              after it, the sample becomes the LAST offender of the frame"
        );
    }

    assert!(
        walk_body.contains("CompactTaskForceCandidate::ForeignIdSpace {")
            && walk_body
                .contains("walk.foreign_id_space = walk.foreign_id_space.saturating_add(1);")
            && !walk_body.contains("count_not_a_task_force(CompactTaskForceMiss"),
        "a foreign-id-space task force must be counted in its own population, never as a
         `NotATaskForce` arm"
    );
}

/// The reason states the omission arithmetic in the order the number is built.
///
/// `compact_task_force_omissions` is
/// `max(unresolved, unaccounted) + refused_before_lift + fleets_without_a_vector`.
/// A reader who cannot follow that has to trust the integer.
///
/// kills: reporting the arms without the split (the substituted stand-in's
/// whole failing was that it named a bit and no cause); and losing the
/// SOURCE_UNAVAILABLE arm into the partial-data sentence, which would say a
/// walk visited elements when it never reached its source.
#[test]
fn d29_reason_names_the_arms_behind_the_omission_count() {
    let mut walk = CompactTaskForceWalkAccounting {
        elements: 230,
        accepted: 213,
        engine_total: 230,
        distinct_lifted: 230,
        not_a_task_force: 17,
        ..CompactTaskForceWalkAccounting::default()
    };
    walk.not_a_task_force_parent_not_a_fleet_root = 17;

    let read = CompactShipRead {
        rows: Vec::new(),
        read_time_us: 0,
        changed_rows: 0,
        status_flags: COMPACT_STATUS_PARTIAL_DATA,
        observed_ship_rows: 1745,
        observed_task_force_rows: 213,
        invalid_task_force_rows: 17,
        source_unavailable: false,
        deleted_rows: 0,
        naval_discovery_us: 0,
        naval_discovery_paid: false,
        naval_walk: walk,
    };
    let reason = domain29_failure_reason(&read);
    assert!(
        reason.contains("17 task force(s) omitted"),
        "the omission count leads: {reason}"
    );
    assert!(
        reason.contains("did not validate as a task force")
            && reason.contains("17 parent not a discovered fleet root"),
        "the arm AND its split must both be in the sentence: {reason}"
    );
    assert!(
        reason.contains("(17 unaccounted)"),
        "the engine-vs-published subtraction is the completeness authority: {reason}"
    );

    // A source-unavailable frame observed NOTHING. Saying it visited 230
    // elements would be a false statement about a walk that never ran.
    let unavailable = CompactShipRead {
        source_unavailable: true,
        status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
        ..read
    };
    let reason = domain29_failure_reason(&unavailable);
    assert!(
        reason.contains("returned nothing") && !reason.contains("the walk visited"),
        "an unavailable frame must not report a walk it never made: {reason}"
    );
}

// --- d64 delayed_events_live ---------------------------------------------

/// Retired reason 7 decomposed into six codes, and every refusal maps to one.
///
/// `rc_vanilla4` carried `d64_first_rejected_timed_effect_reason = 7` on all
/// 53 degraded frames, always the same vtable and class hash. `7` covered an
/// unreadable vtable slot, an unreadable field, a value check both passes
/// refused, a value check only ONE pass refused, a Row-vs-Freed race, and the
/// identity disagreement its name describes. `6` was never inside it.
///
/// kills: routing two refusals to one code, or PERMUTING the mapping — every
/// one of the seven is asserted by value, so a source scan that counts tokens
/// on each side is not what this rests on (Codex round 2, finding 3);
/// re-emitting the retired `7`; renumbering an appended code into the range
/// that already shipped; and two codes sharing a name.
#[test]
fn d64_every_refusal_maps_to_its_own_appended_reason_and_seven_stays_retired() {
    use std::collections::BTreeSet;
    use Domain51DelayedEventFieldCheck as Check;
    use Domain51DelayedEventRefusal as Why;
    use Domain64TimedEffectRejectionReason as R;

    let fingerprint = Domain51DelayedEventFieldRefusal::new(Check::ScopeTagInvalid, 0, 0);
    // EVERY mapping, by value. Exhaustive by construction: the match below is
    // over the refusal enum, so a new variant fails to compile here rather
    // than slipping through untested.
    let expected = |refusal: Why| -> R {
        match refusal {
            Why::Freed => R::DelayedEventFreedDuringRead,
            Why::IdentityUnstable => R::DelayedEventIdentityUnstable,
            Why::ClassUndecided => R::DelayedEventClassUndecided,
            Why::FieldUnreadable => R::DelayedEventFieldUnreadable,
            Why::FieldInvalid(_) => R::DelayedEventFieldInvalid,
            Why::FieldUnstable(_) => R::DelayedEventFieldUnstable,
            Why::FreedBetweenPasses => R::DelayedEventFreedBetweenPasses,
        }
    };
    let refusals = [
        Why::Freed,
        Why::IdentityUnstable,
        Why::ClassUndecided,
        Why::FieldUnreadable,
        Why::FieldInvalid(fingerprint),
        Why::FieldUnstable(fingerprint),
        Why::FreedBetweenPasses,
    ];
    for refusal in refusals {
        assert_eq!(
            refusal.rejection_reason(),
            expected(refusal),
            "{refusal:?} must map to its own code; a permutation of this mapping sends \
             the next lane to the wrong work"
        );
    }
    let mapped = refusals
        .iter()
        .map(|refusal| refusal.rejection_reason() as u64)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        mapped.len(),
        refusals.len(),
        "two refusals sharing a code makes the capture unable to tell them apart"
    );
    assert!(
        !mapped.contains(&(R::DelayedEventTorn as u64)),
        "reason 7 is RETIRED: emitting it again would make old captures, which mean \
         `one of those six`, read as this narrower claim"
    );

    // Only the two value verdicts carry a field refusal onto the wire.
    for refusal in refusals {
        let carries = matches!(refusal, Why::FieldInvalid(_) | Why::FieldUnstable(_));
        assert_eq!(
            refusal.field_refusal().is_some(),
            carries,
            "{refusal:?} must carry a field refusal iff it is about a field's value"
        );
    }

    let all = [
        R::VtableUnavailable,
        R::RttiUnavailable,
        R::VtableChanged,
        R::UnexpectedDelayedEventVtable,
        R::UnsupportedClass,
        R::DelayedEventFreedDuringRead,
        R::DelayedEventTorn,
        R::OwnerOrScopeMismatch,
        R::DuplicateIdentity,
        R::AnchorBaseUnresolvable,
        R::DelayedEventClassUndecided,
        R::DelayedEventFieldInvalid,
        R::DelayedEventFreedBetweenPasses,
        R::DelayedEventFieldUnreadable,
        R::DelayedEventIdentityUnstable,
        R::DelayedEventFieldUnstable,
    ];
    let codes = all.iter().map(|r| *r as u64).collect::<BTreeSet<_>>();
    assert_eq!(codes.len(), all.len(), "every reason code is distinct");
    assert!(!codes.contains(&0), "zero is not a reason; it is a default");

    for appended in [
        R::DelayedEventClassUndecided,
        R::DelayedEventFieldInvalid,
        R::DelayedEventFreedBetweenPasses,
        R::DelayedEventFieldUnreadable,
        R::DelayedEventIdentityUnstable,
        R::DelayedEventFieldUnstable,
    ] {
        assert!(
            appended as u64 > R::AnchorBaseUnresolvable as u64,
            "{appended:?} must be appended, not renumbered into the old range"
        );
    }

    let names = all
        .iter()
        .map(|r| domain64_timed_effect_rejection_reason_name(*r as u64))
        .collect::<BTreeSet<_>>();
    assert_eq!(names.len(), all.len(), "every reason code has its own name");
    assert!(names.iter().all(|name| !name.starts_with("unnamed_reason_")));
    assert_eq!(
        domain64_timed_effect_rejection_reason_name(9999),
        "unnamed_reason_9999",
        "a code appended after this function must still render"
    );

    // And the rooted walk takes its code from that mapping rather than naming
    // seven constants of its own.
    let source = collector_source_lf();
    let start = source
        .find("fn domain51_read_rooted_delayed_events(")
        .expect("the rooted walk is in this file");
    let end = start
        + source[start..]
            .find("\n/// WHERE an RTTI lookup found structures")
            .expect("the rooted walk ends before the RTTI stage enum");
    let body = &source[start..end];
    assert!(
        body.contains("Domain51DelayedEventRead::Refused(refusal) =>")
            && body.contains("refusal.rejection_reason(),")
            && body.contains(".with_field_refusal(refusal.field_refusal()),"),
        "the walk must route every refusal through the one total mapping and carry its \
         field refusal onto the record"
    );
    assert!(
        !body.contains("Domain64TimedEffectRejectionReason::DelayedEvent"),
        "no delayed-event reason constant may be named at the walk; that is a second \
         copy of the mapping, and a test can only restate it"
    );
}

/// The PERF trailer names WHICH value check fired, and on what.
///
/// Without it the record says only `delayed_event_field_invalid` and the next
/// lane cannot tell a rejected zero scope tag — the gate whose refusal is
/// fail-closed pending constructor + serializer proof — from an out-of-range
/// event id (Codex round 3, finding 1); and without the third unit it cannot
/// tell two out-of-module vtables apart (round 4, finding 3).
///
/// kills: dropping ANY of the three units, or corrupting one — each is
/// asserted independently, on a fixture whose three values are pairwise
/// distinct so a swap fails; and emitting them for a rejection that is not
/// about a field's value, where a `0` would read as a check code.
#[test]
fn d64_field_refusal_reaches_the_perf_trailer_with_its_check_value_and_detail() {
    use Domain51DelayedEventFieldCheck as Check;

    // A MALFORMED-RTTI refusal, because it is the only check that uses all
    // three fields, and with three pairwise-distinct values so that a unit
    // carrying its neighbour's number fails.
    let rejection = Domain64TimedEffectRejection {
        reason: Domain64TimedEffectRejectionReason::DelayedEventFieldInvalid,
        country_tag_db_id: 21,
        element_address: 0x2608_5B1D_1160,
        observed_vtable: 0x7FF7_1234_5678,
        class_name_fnv1a64: 11_821_602_218_139_387_652,
        originator_tag_db_id: Some(21),
        field_refusal: Some(Domain51DelayedEventFieldRefusal::new(
            Check::CeventRttiMalformed,
            0x7FF7_ABCD_0000,
            RttiMalformedStage::LocatorNotInModule as u64,
        )),
    };
    let units = domain64_timed_effect_rejection_diagnostic_units(rejection);
    let value = |name: &str| {
        units
            .iter()
            .find(|unit| unit.name == name)
            .map(|unit| unit.value)
    };
    assert_eq!(
        value("d64_rejected_timed_effect_field_check"),
        Some(Check::CeventRttiMalformed as u64),
        "the check must reach the wire as its own stable code"
    );
    assert_eq!(
        value("d64_rejected_timed_effect_field_observed"),
        Some(0x7FF7_ABCD_0000),
        "the observed fact with it — for this check, the CEvent vtable"
    );
    assert_eq!(
        value("d64_rejected_timed_effect_field_detail"),
        Some(RttiMalformedStage::LocatorNotInModule as u64),
        "and the SECOND fact, which is what tells two out-of-module vtables apart"
    );

    // A check with no second fact publishes a MEASURED zero for it, rather
    // than omitting the unit — "this check has no second fact" and "this unit
    // is missing" are different statements about the record.
    let units =
        domain64_timed_effect_rejection_diagnostic_units(Domain64TimedEffectRejection {
            field_refusal: Some(Domain51DelayedEventFieldRefusal::new(
                Check::ScopeTagInvalid,
                0,
                0,
            )),
            ..rejection
        });
    let value = |name: &str| {
        units
            .iter()
            .find(|unit| unit.name == name)
            .map(|unit| unit.value)
    };
    assert_eq!(
        value("d64_rejected_timed_effect_field_check"),
        Some(Check::ScopeTagInvalid as u64)
    );
    assert_eq!(
        value("d64_rejected_timed_effect_field_observed"),
        Some(0),
        "a MEASURED zero is the whole finding for the zero-tag gate"
    );
    assert_eq!(value("d64_rejected_timed_effect_field_detail"), Some(0));

    // A rejection that is not about a field's value publishes neither, rather
    // than a zero that reads like a check code.
    let units = domain64_timed_effect_rejection_diagnostic_units(Domain64TimedEffectRejection {
        reason: Domain64TimedEffectRejectionReason::DuplicateIdentity,
        field_refusal: None,
        ..rejection
    });
    assert!(
        !units
            .iter()
            .any(|unit| unit.name.starts_with("d64_rejected_timed_effect_field_")),
        "a rejection with nothing to say about a field must say nothing"
    );
}

/// THE PAIR VERDICT, over the complete ordered-pair matrix.
///
/// The classifier is a pure total function of two kinds, so this drives every
/// pair rather than reading the match (Codex round 2, finding 3: an arm-order
/// pin does not see a swapped `Why::*` result).
///
/// kills: promoting a better-informed pass over a less-informed one in any of
/// the three memory/reallocation ranks; making the verdict order-sensitive;
/// answering `FieldInvalid` for a pair the second pass DISPROVED (round 2's
/// BLOCKING); and comparing only the CHECK rather than the whole fingerprint,
/// which let a rejected scope tag and a rejected originator tag — or the same
/// check on two different values — count as agreement (round 3's BLOCKING).
#[test]
fn d64_pair_verdict_is_total_symmetric_and_worst_first() {
    use Domain51DelayedEventFieldCheck as Check;
    use Domain51DelayedEventRefusal as Why;
    use Domain51PassKind as Kind;

    let scope_zero = Domain51DelayedEventFieldRefusal::new(Check::ScopeTagInvalid, 0, 0);
    let originator_zero =
        Domain51DelayedEventFieldRefusal::new(Check::OriginatorTagInvalid, 0, 0);
    let scope_other = Domain51DelayedEventFieldRefusal::new(Check::ScopeTagInvalid, 9_999, 0);
    // THE ROUND-4 PAIR: the same check and the same STAGE, on two different
    // out-of-module CEvent vtables. Before the `detail` split these two were
    // `{CeventRttiMalformed, VtableNotInModule}` and nothing else, so they
    // compared equal and published the structural reason for a vtable that had
    // demonstrably changed between the passes.
    let rtti_lo = Domain51DelayedEventFieldRefusal::new(
        Check::CeventRttiMalformed,
        0x1_0000,
        RttiMalformedStage::VtableNotInModule as u64,
    );
    let rtti_hi = Domain51DelayedEventFieldRefusal::new(
        Check::CeventRttiMalformed,
        0x2_0000,
        RttiMalformedStage::VtableNotInModule as u64,
    );
    let rtti_other_stage = Domain51DelayedEventFieldRefusal::new(
        Check::CeventRttiMalformed,
        0x1_0000,
        RttiMalformedStage::LocatorNotInModule as u64,
    );
    let a = Kind::FieldInvalid(scope_zero);
    let b = Kind::FieldInvalid(originator_zero);
    let c = Kind::FieldInvalid(scope_other);
    let lo = Kind::FieldInvalid(rtti_lo);
    let hi = Kind::FieldInvalid(rtti_hi);
    let stage = Kind::FieldInvalid(rtti_other_stage);
    let kinds = [
        Kind::Freed,
        Kind::ClassUndecided,
        Kind::FieldUnreadable,
        a,
        b,
        c,
        lo,
        hi,
        stage,
        Kind::Row,
    ];

    // SYMMETRY over every ordered pair: which pass saw what cannot change the
    // verdict, because the two passes are the same read twice.
    for first in kinds {
        for second in kinds {
            let forward = domain51_delayed_event_pair_refusal(first, second);
            let backward = domain51_delayed_event_pair_refusal(second, first);
            // The CODE is symmetric. The carried fingerprint is the first
            // pass's by construction, so compare the reasons, and the whole
            // verdict wherever the two passes carry the same one.
            assert_eq!(
                forward.rejection_reason(),
                backward.rejection_reason(),
                "the verdict for ({first:?}, {second:?}) must not depend on the order"
            );
        }
    }

    // THE RANKS.
    let cases = [
        ((Kind::Freed, Kind::Freed), Why::Freed),
        ((Kind::Row, Kind::Row), Why::IdentityUnstable),
        (
            (Kind::ClassUndecided, Kind::FieldUnreadable),
            Why::ClassUndecided,
        ),
        ((Kind::ClassUndecided, Kind::Freed), Why::ClassUndecided),
        ((Kind::ClassUndecided, a), Why::ClassUndecided),
        ((Kind::ClassUndecided, Kind::Row), Why::ClassUndecided),
        (
            (Kind::ClassUndecided, Kind::ClassUndecided),
            Why::ClassUndecided,
        ),
        ((Kind::FieldUnreadable, Kind::Freed), Why::FieldUnreadable),
        ((Kind::FieldUnreadable, a), Why::FieldUnreadable),
        ((Kind::FieldUnreadable, Kind::Row), Why::FieldUnreadable),
        (
            (Kind::FieldUnreadable, Kind::FieldUnreadable),
            Why::FieldUnreadable,
        ),
        ((Kind::Freed, a), Why::FreedBetweenPasses),
        ((Kind::Freed, Kind::Row), Why::FreedBetweenPasses),
        // ROUND 2's finding: only agreement supports the structural verdict.
        ((a, a), Why::FieldInvalid(scope_zero)),
        ((b, b), Why::FieldInvalid(originator_zero)),
        ((a, Kind::Row), Why::FieldUnstable(scope_zero)),
        // ROUND 3's finding: agreement is on the WHOLE fingerprint. A rejected
        // scope tag and a rejected originator tag are not the same refusal,
        // and neither are the same check on two different values.
        ((a, b), Why::FieldUnstable(scope_zero)),
        ((a, c), Why::FieldUnstable(scope_zero)),
        // ROUND 4's finding: the same check AND the same stage, on two
        // different out-of-module vtables, is not a reproduced refusal.
        ((lo, lo), Why::FieldInvalid(rtti_lo)),
        ((lo, hi), Why::FieldUnstable(rtti_lo)),
        ((lo, stage), Why::FieldUnstable(rtti_lo)),
    ];
    for ((first, second), want) in cases {
        assert_eq!(
            domain51_delayed_event_pair_refusal(first, second),
            want,
            "({first:?}, {second:?}) must be {want:?}"
        );
    }

    // Stated as the property, not just the table: NO pair whose two passes did
    // not produce the SAME fingerprint may answer `FieldInvalid` — that
    // verdict claims re-reading reproduced the refusal, and those pairs are
    // evidence it did not.
    for first in kinds {
        for second in kinds {
            let reproduced = matches!(
                (first, second),
                (Kind::FieldInvalid(x), Kind::FieldInvalid(y)) if x == y
            );
            let verdict = domain51_delayed_event_pair_refusal(first, second);
            let claims_reproduced = matches!(verdict, Why::FieldInvalid(_));
            assert_eq!(
                claims_reproduced, reproduced,
                "({first:?}, {second:?}) -> {verdict:?}: the read-defect code is exactly \
                 the pairs whose two passes produced the same fingerprint"
            );
        }
    }

    // And the production match delegates rather than deciding for itself.
    let source = collector_source_lf();
    let start = source
        .find("fn domain51_read_delayed_event_row(")
        .expect("the double read is in this file");
    let end = start
        + source[start..]
            .find("\n/// Identity equality that EXCLUDES")
            .expect("the double read ends before the identity helper");
    let body = &source[start..end];
    assert!(
        body.contains(
            "(Pass::Row(a), Pass::Row(b)) if domain51_delayed_event_identity_eq(&a, &b) =>"
        ),
        "the ONE accepting arm must stay exactly what it was"
    );
    assert!(
        body.contains("domain51_delayed_event_pair_refusal(first.kind(), second.kind()),"),
        "every other pair must go through the classifier this test drives"
    );
    assert_eq!(
        body.matches("Domain51DelayedEventRefusal::").count() + body.matches("Why::").count(),
        0,
        "the double read must name no refusal of its own; that is a second copy of \
         the decision"
    );
}

/// THE FIELD READ, driven over the synthetic module image.
///
/// The round-2 fix was pinned by source scan, which cannot see whether the
/// classification is RIGHT (Codex round 3, finding 3). These stage each
/// outcome over real memory and read the answer.
///
/// kills: routing a `read_*` refusal to `Invalid`; routing a value check to
/// `Unreadable`; merging the scope and originator gates, which would make two
/// passes that rejected different fields compare equal; and dropping the
/// observed value, which would make the same check on two different values
/// compare equal.
#[test]
fn d64_field_read_classifies_each_outcome_over_a_real_module_image() {
    use compact_contract::build_profile as bp;
    use Domain51DelayedEventFieldCheck as Check;
    use Domain51DelayedEventFields as Out;

    let image = d64_module_image();
    let event = d64_event_object(&image, 4_200_007);
    let object = d64_delayed_event_object(&image, D64_RVA_VTABLE_DELAYED, event.addr(), 7);
    page_cache_new_frame();

    // THE HAPPY PATH, so every refusal below is a change from a working read
    // rather than a fixture that never worked.
    assert!(
        matches!(
            domain51_read_delayed_event_fields(image.base, object.addr()),
            Out::Row(_)
        ),
        "the fixture must read cleanly, or nothing below proves anything"
    );

    // A CEVENT POINTER THAT READ AND IS NOT A HEAP POINTER.
    let refusal = |object: &AlignedBytes<0x100>| {
        page_cache_new_frame();
        match domain51_read_delayed_event_fields(image.base, object.addr()) {
            Out::Invalid(refusal) => refusal,
            Out::Row(_) => panic!("expected a refusal, got a row"),
            Out::Unreadable => panic!("expected a value refusal, got Unreadable"),
        }
    };

    let mut staged = d64_delayed_event_object(&image, D64_RVA_VTABLE_DELAYED, event.addr(), 7);
    staged.write_u64(bp::DOMAIN51_CDELAYED_EVENT_CEVENT_OFF, 0x1234);
    let got = refusal(&staged);
    assert_eq!(got.check, Check::CeventPointerNotHeap);
    assert_eq!(got.observed, 0x1234, "the word that failed the check");

    // AN EVENT ID OUTSIDE THE RANGE.
    let low_event = d64_event_object(&image, 0);
    let staged = d64_delayed_event_object(&image, D64_RVA_VTABLE_DELAYED, low_event.addr(), 7);
    let got = refusal(&staged);
    assert_eq!(got.check, Check::CombinedEventIdOutOfRange);
    assert_eq!(got.observed, 0);

    // THE TWO COUNTRY-TAG GATES, SEPARATELY. This is round 3's BLOCKING: one
    // gate covering both fields let these two compare equal.
    let mut staged = d64_delayed_event_object(&image, D64_RVA_VTABLE_DELAYED, event.addr(), 7);
    staged.write_u32(bp::DOMAIN51_CDELAYED_EVENT_SCOPE_TAG_OFF, 0);
    let scope = refusal(&staged);
    assert_eq!(scope.check, Check::ScopeTagInvalid);
    assert_eq!(scope.observed, 0);

    let mut staged = d64_delayed_event_object(&image, D64_RVA_VTABLE_DELAYED, event.addr(), 7);
    staged.write_u32(bp::DOMAIN51_CDELAYED_EVENT_ORIGINATOR_TAG_OFF, 0);
    let originator = refusal(&staged);
    assert_eq!(
        originator.check,
        Check::OriginatorTagInvalid,
        "the ORIGINATOR gate must name itself; a shared gate cannot"
    );
    assert_ne!(
        scope, originator,
        "a rejected scope and a rejected originator are different refusals, and two \
         passes that produced them have agreed on nothing"
    );

    // THE SAME CHECK ON A DIFFERENT VALUE is also not agreement.
    let mut staged = d64_delayed_event_object(&image, D64_RVA_VTABLE_DELAYED, event.addr(), 7);
    let out_of_range = domain51::scripted_state_domain51_gate::compact::MAX_COUNTRY_TAG_DB_ID + 1;
    staged.write_u32(bp::DOMAIN51_CDELAYED_EVENT_SCOPE_TAG_OFF, out_of_range);
    let scope_high = refusal(&staged);
    assert_eq!(scope_high.check, Check::ScopeTagInvalid);
    assert_eq!(scope_high.observed, u64::from(out_of_range));
    assert_ne!(
        scope, scope_high,
        "the same check on two different values is not a reproduced refusal"
    );

    // A CEVENT POINTER THAT PASSES THE HEAP GATE AND WILL NOT READ.
    //
    // `Out::Unreadable` is otherwise unreached by this test, so a change that
    // routed a failed read to `Invalid` -- the read-defect side -- would leave
    // it green (Codex round 4, finding 4). `d64_unreadable_heap_pointer`
    // reserves a page inside the heap window without committing it: the
    // address satisfies `is_map_control_heap_pointer` and refuses every read,
    // which is the only shape that separates the two answers.
    let unreadable = d64_unreadable_heap_pointer();
    let mut staged = d64_delayed_event_object(&image, D64_RVA_VTABLE_DELAYED, event.addr(), 7);
    staged.write_u64(
        bp::DOMAIN51_CDELAYED_EVENT_CEVENT_OFF,
        unreadable.addr() as u64,
    );
    page_cache_new_frame();
    assert!(
        matches!(
            domain51_read_delayed_event_fields(image.base, staged.addr()),
            Out::Unreadable
        ),
        "a CEvent pointer that reads as a heap address and whose own word 0 refuses is \
         a memory-availability finding, not a value that failed a check"
    );

    // A CEVENT WHOSE CLASS IS SOMETHING ELSE.
    let mut other = AlignedBytes::<0x40>::new();
    other.write_u64(0, image.vtable(D64_RVA_VTABLE_UNRELATED) as u64);
    other.write_u32(bp::DOMAIN51_CEVENT_COMBINED_ID_OFF, 4_200_007);
    let staged = d64_delayed_event_object(&image, D64_RVA_VTABLE_DELAYED, other.addr(), 7);
    let got = refusal(&staged);
    assert_eq!(got.check, Check::CeventClassMismatch);
    assert_eq!(
        got.observed,
        image.vtable(D64_RVA_VTABLE_UNRELATED) as u64,
        "the CEvent vtable identifies the class the gate refused"
    );
}

/// A DECORATED NAME LONGER THAN THE DIAGNOSTIC WINDOW IS NOT MALFORMED RTTI.
///
/// `RTTI_NAME_WINDOW`'s own comment says it bounds the diagnostic name only
/// and that no classification may rest on it, and
/// `domain64_admits_a_derived_class_whose_name_exceeds_the_name_window` pins a
/// legal class whose name is far longer. A round-2 version of the CEvent gate
/// read that window and called an absent terminator malformed, which would
/// have published a read defect for valid RTTI (Codex round 3, finding 2).
///
/// kills: routing the CEvent gate back through `naval_rtti_class_name` or any
/// other window-bounded read.
#[test]
fn d64_cevent_gate_uses_exact_equality_and_inherits_no_name_window() {
    use Domain51DelayedEventFieldCheck as Check;
    use Domain51DelayedEventFields as Out;

    // 1. A LONG name that is not CEvent: a class mismatch, not malformed RTTI.
    let image = d64_module_image();
    let long_name = format!(".?AV?$CEventT@{}@@", "V".repeat(400));
    assert!(long_name.len() > RTTI_NAME_WINDOW);
    image.write_type_descriptor(D64_RVA_TD_DERIVED, &long_name);
    image.write_locator(D64_RVA_COL_EVENT, D64_RVA_TD_DERIVED, D64_RVA_CHD_DELAYED);
    let event = d64_event_object(&image, 4_200_007);
    let object = d64_delayed_event_object(&image, D64_RVA_VTABLE_DELAYED, event.addr(), 7);
    page_cache_new_frame();
    assert_eq!(
        naval_rtti_class_name(image.base, image.vtable(D64_RVA_VTABLE_EVENT)),
        None,
        "the fixed window really cannot read this name, which is the trap"
    );
    match domain51_read_delayed_event_fields(image.base, object.addr()) {
        Out::Invalid(refusal) => assert_eq!(
            refusal.check,
            Check::CeventClassMismatch,
            "a name the window cannot read is still a name, and this one is not CEvent"
        ),
        other => panic!("expected a class mismatch, got {:?}", DebugOut(&other)),
    }

    // 2. A VALID, SHORT name whose 96-byte window would run past the image but
    // whose own 13 bytes do not.
    let image = d64_module_image();
    let want = ".?AVCEvent@@".len() + 1;
    let boundary_td = D64_FIX_SIZE_OF_IMAGE - want - RTTI_TYPE_DESCRIPTOR_NAME_OFF;
    assert!(
        boundary_td + RTTI_TYPE_DESCRIPTOR_NAME_OFF + RTTI_NAME_WINDOW > D64_FIX_SIZE_OF_IMAGE,
        "the fixture must place the name where a 96-byte window does NOT fit"
    );
    assert!(
        boundary_td >= D64_FIX_LAST_PAGE_RVA,
        "and inside the committed last page"
    );
    image.write_type_descriptor(boundary_td, ".?AVCEvent@@");
    image.write_locator(D64_RVA_COL_EVENT, boundary_td, D64_RVA_CHD_DELAYED);
    let event = d64_event_object(&image, 4_200_007);
    let object = d64_delayed_event_object(&image, D64_RVA_VTABLE_DELAYED, event.addr(), 7);
    page_cache_new_frame();
    assert_eq!(
        naval_rtti_class_name(image.base, image.vtable(D64_RVA_VTABLE_EVENT)),
        None,
        "the window path cannot confine this name, which is the trap"
    );
    assert!(
        matches!(
            domain51_read_delayed_event_fields(image.base, object.addr()),
            Out::Row(_)
        ),
        "exact equality confines by the name's OWN length, so this valid CEvent is \
         admitted"
    );

    // 3. A LOCATOR OUTSIDE THE IMAGE is malformed RTTI, and says where.
    let image = d64_module_image();
    image.write_u64(
        D64_RVA_VTABLE_EVENT - 8,
        (image.base + D64_FIX_FAR_PAGE_RVA) as u64,
    );
    let event = d64_event_object(&image, 4_200_007);
    let object = d64_delayed_event_object(&image, D64_RVA_VTABLE_DELAYED, event.addr(), 7);
    page_cache_new_frame();
    match domain51_read_delayed_event_fields(image.base, object.addr()) {
        Out::Invalid(refusal) => {
            assert_eq!(refusal.check, Check::CeventRttiMalformed);
            // BOTH halves. The vtable is the identity the gate ran on, and it
            // is what changes when a stale word is re-read — which is the
            // whole reason `detail` exists (Codex round 4, finding 3).
            assert_eq!(
                refusal.observed,
                image.vtable(D64_RVA_VTABLE_EVENT) as u64,
                "the CEvent vtable is the observed fact, so two passes that read \
                 DIFFERENT out-of-module vtables do not compare equal"
            );
            assert_eq!(
                refusal.detail,
                RttiMalformedStage::LocatorNotInModule as u64,
                "and the stage says WHERE the RTTI was unusable"
            );
        }
        other => panic!("expected malformed RTTI, got {:?}", DebugOut(&other)),
    }
}

/// A heap address that is RESERVED and not COMMITTED: it satisfies
/// `is_map_control_heap_pointer` and refuses every read.
///
/// The two are otherwise hard to separate in a test, and separating them is
/// the whole of the round-1/round-2 finding: an address the recorder could not
/// read must never be reported as a value that failed a check.
struct D64UnreadableHeapPage(*mut c_void);

impl Drop for D64UnreadableHeapPage {
    fn drop(&mut self) {
        unsafe {
            VirtualFree(self.0, 0, T_MEM_RELEASE);
        }
    }
}

impl D64UnreadableHeapPage {
    fn addr(&self) -> usize {
        self.0 as usize
    }
}

fn d64_unreadable_heap_pointer() -> D64UnreadableHeapPage {
    // Anchor on a REAL heap object, so the search starts inside the window the
    // collector recognises instead of wherever a null hint happens to land.
    let anchor = AlignedBytes::<0x40>::new();
    assert!(
        is_map_control_heap_pointer(anchor.addr()),
        "the fixture heap must be inside the window this test is about"
    );
    let mut want = (anchor.addr() & !0xFFFF) + 0x1000_0000;
    for _ in 0..4096usize {
        let got = unsafe { VirtualAlloc(want as *mut c_void, 0x1000, T_MEM_RESERVE, T_PAGE_READWRITE) };
        if !got.is_null() {
            page_cache_new_frame();
            if is_map_control_heap_pointer(got as usize) && read_u64(got as usize).is_none() {
                return D64UnreadableHeapPage(got);
            }
            unsafe {
                VirtualFree(got, 0, T_MEM_RELEASE);
            }
        }
        want += 0x1000_0000;
    }
    panic!("could not reserve an uncommitted page inside the heap window");
}

/// A tiny `Debug` shim so the panics above can print a
/// `Domain51DelayedEventFields`, which deliberately does not derive `Debug`
/// (it carries a row).
struct DebugOut<'a>(&'a Domain51DelayedEventFields);

impl std::fmt::Debug for DebugOut<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Domain51DelayedEventFields::Row(_) => f.write_str("Row(..)"),
            Domain51DelayedEventFields::Unreadable => f.write_str("Unreadable"),
            Domain51DelayedEventFields::Invalid(refusal) => write!(f, "Invalid({refusal:?})"),
        }
    }
}

/// The typed RTTI helpers, driven — and the `Option` forms proven to be pure
/// delegations of them.
///
/// kills: classifying a step whose reads all completed as `Unreadable` (which
/// is what put a malformed locator under a page fault's name), classifying a
/// step that could not read as `Malformed`, and re-introducing a second
/// implementation behind either `Option` wrapper.
#[test]
fn d64_rtti_failure_separates_unreadable_from_malformed() {
    let image = d64_module_image();
    page_cache_new_frame();
    let span = rtti_module_image_span(image.base).expect("the fixture's PE headers parse");

    // The happy path.
    assert_eq!(
        naval_rtti_class_name_is_typed(
            image.base,
            image.vtable(D64_RVA_VTABLE_EVENT),
            ".?AVCEvent@@"
        ),
        Ok(true)
    );
    assert_eq!(
        naval_rtti_class_name_is_typed(
            image.base,
            image.vtable(D64_RVA_VTABLE_EVENT),
            ".?AVCModifier@@"
        ),
        Ok(false),
        "a class that is not the expected one is an ANSWER, not a failure"
    );

    // A vtable below the module base cannot even be turned into an RVA.
    assert_eq!(
        naval_rtti_class_name_is_typed(image.base, image.base - 0x1000, ".?AVCEvent@@"),
        Err(RttiLookupFailure::Malformed(
            RttiMalformedStage::VtableNotInModule
        ))
    );
    // A vtable inside the reserve but PAST the declared image.
    assert_eq!(
        naval_rtti_class_name_is_typed(
            image.base,
            image.base + D64_FIX_FAR_PAGE_RVA,
            ".?AVCEvent@@"
        ),
        Err(RttiLookupFailure::Malformed(
            RttiMalformedStage::VtableNotInModule
        ))
    );
    // A locator OUTSIDE the image, reached from a vtable inside it.
    image.write_u64(
        D64_RVA_VTABLE_EVENT - 8,
        (image.base + D64_FIX_FAR_PAGE_RVA) as u64,
    );
    page_cache_new_frame();
    assert_eq!(
        naval_rtti_class_name_is_typed(
            image.base,
            image.vtable(D64_RVA_VTABLE_EVENT),
            ".?AVCEvent@@"
        ),
        Err(RttiLookupFailure::Malformed(
            RttiMalformedStage::LocatorNotInModule
        ))
    );
    // A locator SLOT that would read from an address the module does not
    // cover: the locator lives at `vtable - 8`, so a vtable at rva 0 has none.
    assert_eq!(
        rtti_complete_object_locator_typed(image.base, span, image.base),
        Err(RttiLookupFailure::Malformed(
            RttiMalformedStage::LocatorSlotNotInModule
        ))
    );
    // A name whose own bytes run past the image.
    assert_eq!(
        rtti_type_descriptor_name_is_typed(
            image.base,
            span,
            D64_FIX_SIZE_OF_IMAGE - RTTI_TYPE_DESCRIPTOR_NAME_OFF,
            ".?AVCEvent@@"
        ),
        Err(RttiLookupFailure::Malformed(
            RttiMalformedStage::NameNotInModule
        ))
    );
    // A type-descriptor RVA whose name offset overflows.
    assert_eq!(
        rtti_type_descriptor_name_is_typed(image.base, span, usize::MAX, ".?AVCEvent@@"),
        Err(RttiLookupFailure::Malformed(
            RttiMalformedStage::TypeDescriptorRvaOverflow
        ))
    );

    // THE UNREADABLE HALF, which no assertion above reaches: without these,
    // reclassifying a failed READ as `Malformed` would leave this test green
    // (Codex round 4, finding 4).
    //
    // The fixture RESERVES the whole image and COMMITS four pages, so an RVA
    // inside the declared image but outside those pages passes every
    // confinement check and refuses every read. That is exactly the shape the
    // tie-break rule is about.
    let unreadable_rva = D64_RTTI_PAGE_RVA + 0x8000;
    assert!(
        unreadable_rva + RTTI_NAME_WINDOW < D64_FIX_SIZE_OF_IMAGE,
        "the uncommitted fixture RVA must still be inside the declared image"
    );
    assert_eq!(
        read_u64(image.base + unreadable_rva),
        None,
        "the fixture RVA must really be unreadable, or these prove nothing"
    );
    // The LOCATOR SLOT read: a vtable at an uncommitted in-image RVA.
    assert_eq!(
        rtti_complete_object_locator_typed(image.base, span, image.base + unreadable_rva),
        Err(RttiLookupFailure::Unreadable)
    );
    assert_eq!(
        naval_rtti_class_name_is_typed(
            image.base,
            image.base + unreadable_rva,
            ".?AVCEvent@@"
        ),
        Err(RttiLookupFailure::Unreadable)
    );
    // The NAME BYTES read.
    assert_eq!(
        rtti_type_descriptor_name_is_typed(image.base, span, unreadable_rva, ".?AVCEvent@@"),
        Err(RttiLookupFailure::Unreadable)
    );

    // THE DELEGATIONS: one implementation each, so the classified and the
    // unclassified answers cannot drift apart.
    let source = collector_source_lf();
    for (wrapper, typed) in [
        (
            "fn rtti_complete_object_locator(",
            "rtti_complete_object_locator_typed(module_base, image_span, vtable).ok()",
        ),
        (
            "fn rtti_type_descriptor_name_is(",
            "rtti_type_descriptor_name_is_typed(module_base, image_span, td_rva, expected).ok()",
        ),
    ] {
        let at = source
            .find(wrapper)
            .unwrap_or_else(|| panic!("{wrapper} must still exist for its other callers"));
        let body = &source[at..(at + 500).min(source.len())];
        assert!(
            body.contains(typed),
            "{wrapper} must delegate to its typed form; two implementations would drift"
        );
    }
    // And nothing on the class-name path may be window-bounded again.
    let at = source
        .find("fn naval_rtti_class_name_is_typed(")
        .expect("the typed exact-equality lookup is in this file");
    let end = at
        + source[at..]
            .find("\n}\n")
            .expect("it ends at a column-zero brace");
    assert!(
        !source[at..end].contains("RTTI_NAME_WINDOW"),
        "the gate must inherit no diagnostic name window"
    );
}

/// The five refusal branches are five sentences, not one bit.
///
/// kills: dropping the branch from the read (the reason would fall back to
/// "without naming a branch"); and letting the lifecycle-drain branch
/// overwrite an earlier, more specific refusal.
#[test]
fn d64_reason_names_its_branch_and_the_first_refusal_wins() {
    let read = |refusal: Option<Domain64Refusal>| CompactDelayedEventsRead {
        rows: Vec::new(),
        lifecycle: None,
        lifecycle_reservation_id: None,
        observed_rows: 0,
        read_time_us: 0,
        status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
        diagnostic_units: Vec::new(),
        refusal,
    };

    let rejection = Domain64TimedEffectRejection {
        reason: Domain64TimedEffectRejectionReason::DelayedEventFieldInvalid,
        country_tag_db_id: 21,
        element_address: 0x2608_5B1D_1160,
        observed_vtable: 0x7FF7_1234_5678,
        class_name_fnv1a64: 11_821_602_218_139_387_652,
        originator_tag_db_id: Some(21),
        field_refusal: Some(Domain51DelayedEventFieldRefusal::new(
            Domain51DelayedEventFieldCheck::ScopeTagInvalid,
            0,
            0,
        )),
    };
    let reason = domain64_failure_reason(&read(Some(Domain64Refusal::RejectedTimedEffect(
        rejection,
    ))));
    assert!(
        reason.contains("delayed_event_field_invalid") && reason.contains("(reason 12)"),
        "the rejection's own reason must be named AND numbered: {reason}"
    );
    assert!(
        reason.contains("tag_db_id 21"),
        "the country the walk was inside must survive into the sentence: {reason}"
    );

    // Every branch says something different.
    let branches = [
        Domain64Refusal::NoScalarSource,
        Domain64Refusal::CountryRosterUnreadable,
        Domain64Refusal::ScanFailedClosed,
        Domain64Refusal::LifecycleDrainUnavailable,
    ];
    let mut seen = std::collections::BTreeSet::new();
    for branch in branches {
        let text = domain64_failure_reason(&read(Some(branch)));
        assert!(
            !text.contains("without naming a branch"),
            "{branch:?} must produce a sentence of its own"
        );
        assert!(seen.insert(text), "{branch:?} must not repeat another branch");
    }

    // The FIRST refusal wins: `get_or_insert`, never an assignment.
    let source = collector_source_lf();
    let start = source
        .find("fn read_compact_delayed_events(")
        .expect("the d64 reader is in this file");
    let end = start
        + source[start..]
            .find("\n/// B1 exact owner+nested iterator.")
            .expect("the d64 reader ends before the market-trade walk");
    let body = &source[start..end];
    assert!(
        body.contains(".get_or_insert(Domain64Refusal::LifecycleDrainUnavailable)"),
        "the lifecycle branch must not displace an earlier, more specific refusal"
    );
    assert!(
        !body.contains("refusal = Some(Domain64Refusal::LifecycleDrainUnavailable)"),
        "a plain assignment here would relabel a rejected timed effect as a drain failure"
    );
}

// --- d93 country_army_upgrade_requests_live ------------------------------

/// The four causes that reached `rc_vanilla4` as one sentence.
///
/// All four of d93's degraded frames had DIFFERENT causes — request vector
/// changed, country/status graph changed, delivery vector changed, coherent
/// restarts exhausted — and all four published the same substituted sentence.
/// Every one is the engine mutating the AUR1 graph under the walk, which is
/// why the fix is the outcome and not the read.
///
/// kills: dropping the one-hot unit name from the sentence, which is what lets
/// a reader join the reason to the PERF trailer without a hash table; and
/// losing the country context, which is the only locator d93 has.
#[test]
fn d93_reason_carries_the_same_unit_name_the_trailer_publishes() {
    use country_army_upgrade_requests_live_lane::CarrierError;

    for (text, unit) in [
        (
            "AUR1 request vector changed during snapshot",
            "d93_fail_reason_request_vector_changed",
        ),
        (
            "AUR1 country/status graph changed during snapshot",
            "d93_fail_reason_country_graph_changed",
        ),
        (
            "AUR1 delivery vector changed during snapshot",
            "d93_fail_reason_delivery_vector_changed",
        ),
        (
            "AUR1 object graph kept changing across every coherent-read restart",
            "d93_fail_reason_coherent_restarts_exhausted",
        ),
    ] {
        let error = CarrierError::CountryRead {
            tag_db_id: 30,
            object_ptr: 0x25FF_1234_5678,
            source: Box::new(CarrierError::Invalid(text.to_string())),
        };
        let reason = d93_failure_reason(&error);
        assert!(
            reason.contains(unit),
            "the reason must carry the trailer's own unit name {unit}: {reason}"
        );
        assert!(
            reason.contains(text),
            "and the carrier's own words: {reason}"
        );
        assert!(
            reason.contains("country tag_db_id 30") && reason.contains("0x000025ff12345678"),
            "the country context must survive the unwrap: {reason}"
        );
        // The units builder and the sentence must agree on the unit.
        let units = d93_failure_diagnostic_units(&error);
        assert!(
            units.iter().any(|u| u.name == unit),
            "the trailer publishes {unit} for the same error"
        );
    }

    // A memory failure keeps its exact address and width rather than being
    // flattened into "invalid".
    let reason = d93_failure_reason(&CarrierError::Memory {
        address: 0x25FF_0000_1000,
        width: 8,
    });
    assert!(
        reason.contains("8-byte read at 0x000025ff00001000"),
        "a memory refusal names the read that failed: {reason}"
    );
}

// --- d42 air_combat_losses_live ------------------------------------------

/// THE VALUE, which is half of a disjunctive predicate's finding.
///
/// `1808` (`REGION_RECORD_ATTRIBUTION_TAG_NOT_A_TAG`) refused 74 frames of
/// `rc_vanilla4`. `valid_compact_air_wing_owner_tag_db_id` refuses ZERO and
/// refuses out-of-vector, and the capture published the address, the ordinal,
/// the container index and both extents without ever publishing the number
/// that separates them.
///
/// kills: emitting the value as `Context` (a budget squeeze would drop it and
/// reproduce the unanswerable frame); emitting a missing value as a measured
/// `0` (which is exactly the answer that would matter most, so "absent" and
/// "zero" must not collide); and reverting any value-validating predicate to
/// the plain `refuse`, which is what left 1808 mute.
#[test]
fn d42_value_validating_predicates_publish_the_number_they_refused() {
    let mut walk = Domain42WalkDiag::default();
    walk.enter(Domain42WalkStage::RegionRecord, 0x2608_1234_5678, 14);
    walk.refuse_with_value(
        domain42_predicate::REGION_RECORD_ATTRIBUTION_TAG_NOT_A_TAG,
        0,
    );
    let units = domain42_declared_diagnostic_units(&Domain42Diag {
        region_walk: Some(walk),
        ..Domain42Diag::default()
    });
    let by_name = |name: &str| {
        units
            .iter()
            .find(|unit| unit.name == name)
            .unwrap_or_else(|| panic!("{name} must be emitted"))
    };
    let value = by_name("d42_region_walk_predicate_value_sample");
    assert_eq!(value.value, 0);
    assert_eq!(
        value.presence,
        compact_contract::diagnostic_channel::UnitPresence::Observed,
        "a MEASURED zero is the whole answer for 1808 and must not read as absent"
    );
    assert_eq!(
        value.role,
        compact_contract::diagnostic_channel::UnitRole::Deciding,
        "for a disjunctive predicate the value is half the finding, so it rides the \
         per-offender floor with the predicate"
    );

    // A memory-availability refusal has no value, and that is not a zero.
    let mut unread = Domain42WalkDiag::default();
    unread.enter(Domain42WalkStage::RegionRecord, 0x2608_1234_5678, 14);
    let units = domain42_declared_diagnostic_units(&Domain42Diag {
        region_walk: Some(unread),
        ..Domain42Diag::default()
    });
    let value = units
        .iter()
        .find(|unit| unit.name == "d42_region_walk_predicate_value_sample")
        .expect("the unit is emitted either way, with its presence carrying the difference");
    assert_eq!(
        value.presence,
        compact_contract::diagnostic_channel::UnitPresence::Absent,
        "an unnamed-predicate refusal has no value to report"
    );

    // Entering a stage clears a value recorded earlier, so no predicate can
    // wear a number that belongs to another one.
    let mut reused = Domain42WalkDiag::default();
    reused.refuse_with_value(domain42_predicate::REGION_RECORD_WING_ID_ZERO, 7);
    reused.enter(Domain42WalkStage::RegionDamageEntry, 0x10, 0);
    assert!(
        !reused.refused_value_known && reused.refused_value == 0,
        "entering a stage resets the recorded value with the predicate"
    );

    // EVERY value-validating predicate records its value in production.
    let source = collector_source_lf();
    let start = source
        .find("fn sample_compact_air_region_combat_native(")
        .expect("the region walk is in this file");
    let end = start
        + source[start..]
            .find("\nfn valid_compact_air_wing_owner_tag_db_id(")
            .expect("the region walk ends before the tag validator");
    let body = &source[start..end];
    for predicate in [
        "REGION_OWNER_TAG_NOT_A_TAG",
        "REGION_OBJECT_OWNER_NOT_A_TAG",
        "REGION_RECORD_WING_ID_ZERO",
        "REGION_RECORD_WING_ID_ABOVE_MAX",
        "REGION_RECORD_PLANE_COUNT_ABOVE_MAX",
        "REGION_RECORD_SELECTION_FLAG_NOT_BOOL",
        "REGION_RECORD_CLASSIFICATION_FLAG_NOT_BOOL",
        "REGION_RECORD_COPIED_FLAG_NOT_BOOL",
        "REGION_RECORD_ATTRIBUTION_TAG_NOT_A_TAG",
        "REGION_DAMAGE_ENTRY_CAUSE_ABOVE_MAX",
        "REGION_DAMAGE_ENTRY_AMOUNT_NEGATIVE",
        "REGION_DAMAGE_ENTRY_AMOUNT_ABOVE_MAX",
    ] {
        let needle = format!("domain42_predicate::{predicate}");
        let at = body
            .find(&needle)
            .unwrap_or_else(|| panic!("{predicate} must be refused inside the region walk"));
        // The call that carries it: look backwards to the nearest `diag.`.
        let before = &body[..at];
        let call = before
            .rfind("diag.refuse")
            .expect("the predicate is named by a refuse call");
        assert!(
            before[call..].starts_with("diag.refuse_with_value("),
            "{predicate} refuses a well-read number and must publish it"
        );
    }
}

/// The producer's sentence, not the walk's stand-in.
///
/// kills: dropping the predicate NAME (leaving a bare number no reader can
/// resolve without the source); dropping the value; and losing the
/// re-walk marker, which is what separates "refused once" from "refused a
/// NAMED predicate again after the transient retry".
#[test]
fn d42_reason_names_the_predicate_the_value_and_the_place() {
    let mut walk = Domain42WalkDiag::default();
    walk.enter_container(1, 440);
    walk.enter_level(14, 16);
    walk.enter(Domain42WalkStage::RegionRecord, 0x2608_1234_5678, 14);
    walk.refuse_with_value(
        domain42_predicate::REGION_RECORD_ATTRIBUTION_TAG_NOT_A_TAG,
        0,
    );
    let reason = domain42_failure_reason(&Domain42Diag {
        region_walk: Some(walk),
        region_walk_retried: true,
        ..Domain42Diag::default()
    });
    assert!(
        reason.contains("region_record_attribution_tag_is_a_handle")
            && reason.contains("predicate 1808"),
        "the predicate must be named AND numbered: {reason}"
    );
    assert!(
        reason.contains("refused value 0 (0x0)"),
        "the number that separates the sentinel from junk must be in the sentence: {reason}"
    );
    assert!(
        reason.contains("container 1 of 440") && reason.contains("entry 14 of 16"),
        "both ordinal/extent pairs locate the record: {reason}"
    );
    assert!(
        reason.contains("after one transient re-walk"),
        "a NAMED predicate that survived the retry is a different claim: {reason}"
    );

    // An UNNAMED predicate — a bounded read that returned nothing through `?` —
    // is the only refusal that may be described as unreadable memory. That is
    // what `is_unnamed` means.
    let mut unread = Domain42WalkDiag::default();
    unread.enter(Domain42WalkStage::RegionSideVector, 0x40, 1);
    assert!(
        domain42_predicate::is_unnamed(unread.predicate),
        "entering a stage leaves the unnamed code, or this fixture proves nothing"
    );
    let reason = domain42_failure_reason(&Domain42Diag {
        region_walk: Some(unread),
        ..Domain42Diag::default()
    });
    assert!(
        reason.contains("no value (a bounded read found the memory unreadable)"),
        "an unnamed-predicate refusal must not imply a measured value: {reason}"
    );

    // A NAMED predicate with no recorded value is a STRUCTURAL check that has
    // no scalar to report — a non-heap pointer, a vtable echo, a count against
    // its own capacity. Calling those unreadable memory is a false statement
    // about a read that completed (Codex round 1, finding 3), and it would
    // send the next lane looking for a page fault that never happened.
    for named in [
        domain42_predicate::REGION_OBJECT_NOT_HEAP,
        domain42_predicate::REGION_OBJECT_VTABLE_ECHO,
        domain42_predicate::REGION_SIDE_COUNT_ABOVE_CAPACITY,
        domain42_predicate::REGION_TABLE_BEGIN_NOT_HEAP,
        domain42_predicate::REGION_ROW_BUDGET_EXCEEDED,
    ] {
        assert!(
            !domain42_predicate::is_unnamed(named),
            "{named} is a named predicate, or this loop is testing the wrong thing"
        );
        let mut walk = Domain42WalkDiag::default();
        walk.enter(Domain42WalkStage::RegionObject, 0x80, 2);
        walk.refuse(named);
        assert!(
            !walk.refused_value_known,
            "these predicates carry no scalar, which is the case under test"
        );
        let reason = domain42_failure_reason(&Domain42Diag {
            region_walk: Some(walk),
            ..Domain42Diag::default()
        });
        assert!(
            reason.contains("no scalar value captured for this predicate"),
            "predicate {named} completed its read and must not be reported as \
             unreadable memory: {reason}"
        );
        assert!(
            !reason.contains("found the memory unreadable"),
            "predicate {named} read fine; only an unnamed predicate may say that: {reason}"
        );
    }

    // Nothing observed at all still produces a sentence, not an empty one.
    let reason = domain42_failure_reason(&Domain42Diag::default());
    assert!(
        reason.contains("without naming a branch"),
        "a failing word with no diagnosis says so plainly: {reason}"
    );
}

/// The declared roles survive into the outcome.
///
/// `deciding_units_from_perf` flattens everything to `Deciding` because a
/// legacy PERF trailer carries no roles. D42's do, and flattening them would
/// make every unit compete for the per-offender floor — the exact failure the
/// declared channel exists to prevent.
///
/// kills: routing d42's units through `deciding_units_from_perf`.
#[test]
fn d42_outcome_units_keep_the_producers_own_roles_and_presences() {
    use compact_contract::diagnostic_channel::{UnitPresence, UnitRole};

    let declared = vec![
        DeclaredDiagnosticUnit::deciding("a", 1),
        DeclaredDiagnosticUnit::counter("b", 2),
        DeclaredDiagnosticUnit::context("c", 3),
        DeclaredDiagnosticUnit::new("d", 0, UnitRole::Locator, UnitPresence::Absent),
    ];
    let carried = outcome_units_from_declared(&declared);
    assert_eq!(carried.len(), declared.len());
    for (before, after) in declared.iter().zip(carried.iter()) {
        assert_eq!(after.name, before.name);
        assert_eq!(after.value, before.value);
        assert_eq!(after.role, before.role, "the role must not be rewritten");
        assert_eq!(
            after.presence, before.presence,
            "an absent value must not become a measured one"
        );
    }
    assert!(
        carried.iter().any(|unit| unit.role != UnitRole::Deciding),
        "a converter that flattened every role would pass the loop above vacuously \
         only if every input were Deciding; this fixture is not"
    );
}

/// DECISIVE for D42's two records agreeing: the helper hands back the word it
/// was given, WHATEVER the frame's own word carries.
///
/// D42 is the only domain that is both worker-converted and routed through the
/// ledger, so it is the only site where the derived word has nowhere to go but
/// a local. Its PERF record publishes that local and its DATA record publishes
/// `read.status_flags`, because the inline arm exists to reproduce the encode
/// the WORKER performed and the worker had no local. The two records therefore
/// agree only because the two spellings carry the same bits.
///
/// `degraded_frame_tests::record_reader_status_outcome_is_total_over_the_bits
/// _it_claims` already drives the three return paths, but every one of its
/// cases passes the SAME word as the frame base and as the reader word. That
/// is the one shape which cannot detect a frame bit leaking into the result:
/// `derive_status` seeds its accumulator from a base, and if that base were
/// the FRAME's word instead of the read's, every equal-word case would still
/// pass while D42's PERF record silently gained a bit its DATA record does
/// not have.
///
/// So each case here gives the frame a bit the reader's word does NOT carry.
///
/// kills: seeding `record_outcome`'s base from `frame_base_status` rather than
/// from `read_status`; any early return that answers with a composed word
/// instead of the one it was handed; and a future OR applied to the result
/// before it is published.
#[test]
fn the_ledger_helper_returns_the_word_it_was_handed() {
    use compact_contract::domain_outcome::DomainOutcomeLedger;

    let domain_id = COMPACT_DOMAIN_AIR_COMBAT_LOSSES_LIVE;
    // Carried by the FRAME and by no reader: if the result were composed from
    // the frame's word, this bit is what would appear in it.
    let frame_only = COMPACT_STATUS_NO_CLOCK;

    // Path 1: the reader claims bits of its own. Driven over every bit the
    // helper recognises, so a single mishandled bit cannot hide behind the
    // others.
    let mut bit = 1u32;
    let mut covered = 0u32;
    while bit != 0 {
        if READER_OWNED_FAILURE_BITS & bit != 0 && bit != frame_only {
            let mut outcomes = DomainOutcomeLedger::new();
            let published = record_reader_status_outcome(
                &mut outcomes,
                domain_id,
                frame_only,
                bit,
                String::new,
                Vec::new(),
            );
            assert_eq!(
                published, bit,
                "bit {bit:#010x}: the published word must be the reader's word, but the \
                 frame's {frame_only:#010x} reached it -- D42's PERF record would then \
                 carry a bit its DATA record does not",
            );
            covered |= bit;
        }
        bit <<= 1;
    }
    assert_eq!(
        covered,
        READER_OWNED_FAILURE_BITS & !frame_only,
        "the loop must have driven every reader-owned bit but the frame's own",
    );

    // Path 2: the reader claims nothing, so the helper returns early. The
    // frame is still failing, which is precisely when a composed answer would
    // look plausible.
    let mut outcomes = DomainOutcomeLedger::new();
    let published = record_reader_status_outcome(
        &mut outcomes,
        domain_id,
        frame_only,
        0,
        String::new,
        Vec::new(),
    );
    assert_eq!(
        published, 0,
        "a reader that claimed nothing publishes nothing, whatever the frame carries",
    );

    // Path 3: the worker panicked, so the helper returns before deriving at
    // all -- the one path that never reaches `derive_status`.
    let mut outcomes = DomainOutcomeLedger::new();
    record_worker_panic_outcome(&mut outcomes, domain_id, &"the walk tripped an assert");
    let panic_status = COMPACT_STATUS_WORKER_PANIC | COMPACT_STATUS_PARTIAL_DATA;
    let published = record_reader_status_outcome(
        &mut outcomes,
        domain_id,
        frame_only,
        panic_status,
        || "a restatement of the bits".to_string(),
        Vec::new(),
    );
    assert_eq!(
        published, panic_status,
        "the panic path must also hand back the reader's word untouched",
    );

    // AND THE PRODUCTION CONSEQUENCE, stated where it is easy to check: the
    // two D42 records are built from the two different spellings, so if the
    // equality above ever stops holding they disagree on the wire.
    let source = collector_source_lf();
    let block_at = source
        .find("    if let Some(published) = air_combat_losses_live_read {")
        .expect("D42 still publishes from a `published` binding");
    let block_end = source[block_at..]
        .find("\n    }\n")
        .map(|at| block_at + at)
        .unwrap_or(source.len());
    let block = &source[block_at..block_end];
    assert!(
        block.contains("status_flags: d42_published_status,"),
        "D42's PERF record must publish the ledger's own derived word",
    );
    let inline_at = block
        .find("None => append_compact_air_strength_deltas_data_record(")
        .expect("D42 keeps an inline fallback encode");
    assert!(
        block[inline_at..].contains("read.status_flags,"),
        "D42's inline DATA encode must hand the builder the same field the WORKER \
         encoded, or the two encodes stop being comparable",
    );
    assert!(
        !block[inline_at..].contains("d42_published_status"),
        "the inline DATA encode must not reach for the local: the worker had no local, \
         and `the_seven_converted_domains_are_wired_through_the_publishing_helper` \
         compares the two argument lists field by field",
    );
}
