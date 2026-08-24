//! Lane TB's proof that swapping d41's per-country duplicate-`local_id`
//! detector off the heap changed no recorded byte.
//!
//! Five tests. The first is the differential one — every answer
//! [`ProjectLocalIdSeen`] gives, against the `BTreeSet<u32>` it replaced, over
//! a sequence built to cover repeats, the pool cap and the one deliberate
//! divergence past it. The second and third drive the two REAL decode arms
//! over staged pools, so the BEHAVIOUR is proven where it is used and not only
//! where it is defined — but those three all pass if production reverts to
//! `BTreeSet`, because reverting preserves every byte and only loses the
//! saving (Codex review `01a033dd-4679-7110-9193-4babb2ba925b`, SHOULD-FIX 3),
//! so the fifth pins that both arms still USE the detector. The fourth is a
//! source pin over the lane's other change — the specialization-scan state
//! code — because that function needs a live heap to reach any of its
//! interesting exits.
//!
//! WHY THAT IS THE WHOLE BYTE-IDENTITY ARGUMENT. `seen_ids` is written and
//! read in exactly one expression in each arm —
//! `if !seen_ids.insert(local_id) { return CountryLifecycle::Aborted; }` — it
//! is never iterated, never compared and never leaves the decode, and no
//! payload builder can reach it. So the only thing it can move is WHETHER a
//! country aborts. Test 1 pins the predicate; tests 2 and 3 pin both outcomes
//! (`Aborted`, and a full pool of rows) on both arms.

use super::*;

// The two source-pin helpers, borrowed from lane RZ's module rather than
// copied into a third place. See test 4.
use super::rz_projects_worker::{attach_log_source, function_body};

// ------------------------------------------------------------------
// 1. The predicate
// ------------------------------------------------------------------

/// The detector must answer what `BTreeSet::<u32>::insert` answered, id for id.
///
/// Driven side by side rather than asserted from the shape of the code,
/// because "set membership over `u32`" is only obviously the same predicate
/// until someone reaches for `ids[..]` instead of `ids[..len]` (which makes
/// the never-written zero slots members) or for a last-value compare (which
/// misses a non-adjacent repeat). The sequence below contains both shapes: a
/// non-adjacent repeat, a repeat of the FIRST id after many others, and — the
/// case a fixed array has and a tree does not — a fill to exactly
/// `MAX_PROJECTS_PER_COUNTRY`.
///
/// KILLS: a `len < MAX - 1` bound, which refuses the legal 64th id; `!=`
/// against the previous id, which lets the non-adjacent repeat through; and
/// any change that stops recording an id it reported as new, which the
/// re-probe at the end catches.
///
/// DOES NOT KILL `ids[..]` in place of `ids[..self.len]`, and an earlier
/// version of this comment claimed it did and then contradicted itself two
/// lines later (Codex review `01a033f8-f467-76f0-b6f6-40b6325d0ab4`, NIT 6).
/// Production rejects `local_id == 0` one statement before the insert and this
/// test never inserts zero, so the never-written zero slots can never collide
/// with a real id and the wider slice answers identically.
/// Insert `id` into both, assert they agree, and — when the answer was "new" —
/// immediately re-probe it.
///
/// THE RE-PROBE HAS TO HAPPEN NOW, not at the end. Once the detector is full,
/// a missing id and a present id BOTH answer `false`: the missing one falls
/// through `contains` and then fails the capacity check. So a detector that
/// reports an id as new, bumps `len` and never stores it is invisible to any
/// re-probe taken after the fill — which is how the previous version of this
/// test passed exactly that mutation on its final fill id (Codex review
/// `01a0341f-b115-7322-adc1-1961da7f8b61`, SHOULD-FIX 1). While capacity
/// remains, the two answers differ, and this is the only window in which they
/// do.
///
/// In a correct detector the re-probe returns `false` and mutates nothing, so
/// it cannot consume capacity or perturb the fill.
///
/// ITS REACH, EXACTLY: it covers every id EXCEPT the 64th, because after that
/// insertion the detector is already full and the capacity check answers
/// `false` for a stored and an unstored id alike. An implementation that
/// omitted only the final slot write would still pass. That is not
/// production-observable — there is no 65th id to collide with it — so it is
/// stated rather than chased (Codex review
/// `01a0342a-9961-73e0-afaf-def32f07c96e`, NIT 3).
fn insert_and_confirm_stored(
    seen: &mut ProjectLocalIdSeen,
    reference: &mut BTreeSet<u32>,
    id: u32,
) {
    let detector_answer = seen.insert(id);
    let reference_answer = reference.insert(id);
    assert_eq!(
        detector_answer, reference_answer,
        "detector and BTreeSet must agree on id {id}"
    );
    if detector_answer {
        assert!(
            !seen.insert(id),
            "the detector reported id {id} as new, so it must now HOLD it"
        );
    }
}

#[test]
fn the_local_id_seen_detector_answers_exactly_what_a_btreeset_did() {
    let mut seen = ProjectLocalIdSeen::new();
    let mut reference: BTreeSet<u32> = BTreeSet::new();

    // A non-adjacent repeat (7), an adjacent repeat (12), and a repeat of the
    // first id after several others (3).
    for id in [3u32, 7, 12, 12, 41, 7, 900, 3, 41] {
        insert_and_confirm_stored(&mut seen, &mut reference, id);
    }

    // Fill to exactly the pool cap. `reference` already holds 5 ids, so this
    // adds the remaining 59 and lands on `MAX_PROJECTS_PER_COUNTRY` for both.
    let mut next = 1_000u32;
    while reference.len() < MAX_PROJECTS_PER_COUNTRY {
        insert_and_confirm_stored(&mut seen, &mut reference, next);
        next += 1;
    }
    assert_eq!(seen.len, MAX_PROJECTS_PER_COUNTRY);
    assert_eq!(reference.len(), MAX_PROJECTS_PER_COUNTRY);

    // EVERY id the reference holds must still read as present at a FULL
    // detector — not a selection of them.
    //
    // This is the weaker of the two storage checks and it is kept for what it
    // does cover: a detector that FORGETS an id it stored, or whose lookup
    // stops working once `len` reaches the cap. It cannot catch an id that was
    // never stored, because at a full detector that answers `false` through
    // the capacity check — `insert_and_confirm_stored` is what covers that,
    // at the only moment the two are distinguishable.
    let recorded: Vec<u32> = reference.iter().copied().collect();
    assert_eq!(recorded.len(), MAX_PROJECTS_PER_COUNTRY);
    for id in recorded {
        assert!(
            !seen.insert(id),
            "id {id} was recorded and must stay present at a full detector"
        );
        assert!(!reference.insert(id));
    }

    // THE ONE DELIBERATE DIVERGENCE, pinned so it is a decision and not a
    // surprise. A `BTreeSet` grows; the detector cannot, and answers
    // "already seen" so the country aborts whole rather than silently
    // forgetting an id and letting a LATER duplicate of it through. The
    // production path cannot reach this: `read_country_project_lifecycle`
    // refuses `count > PROJECT_LIFECYCLE_MAX_PER_COUNTRY` before either decode
    // arm is entered. See `ProjectLocalIdSeen`.
    let over_cap = 9_999_999u32;
    assert!(
        !seen.insert(over_cap),
        "a full detector must fail closed on a 65th distinct id"
    );
    assert!(
        reference.insert(over_cap),
        "the BTreeSet it replaced would have grown instead — this is the \
         documented divergence, and it is unreachable from production"
    );
}

// ------------------------------------------------------------------
// 2. and 3. The two decode arms
// ------------------------------------------------------------------

/// Stage `count` contiguous pool records with the given local ids, plus one
/// state machine per record, and return the pieces the two arms need.
///
/// The state machines are returned so the caller keeps them alive: the decode
/// reads through the raw addresses written into the pool records, so dropping
/// the backing allocations before the decode would read freed memory.
fn stage_pool(
    local_ids: &[u32],
    project_vtable: usize,
    state_machine_vtable: usize,
    prototype_vtable: usize,
    simple_vtable: usize,
) -> (
    Box<AlignedBytes<PROJECT_POOL_ARRAY_BULK_LEN>>,
    Vec<Box<AlignedBytes<PROJECT_STATE_MACHINE_SNAPSHOT_LEN>>>,
) {
    let mut states = Vec::new();
    for _ in 0..local_ids.len() {
        let mut state = AlignedBytes::<PROJECT_STATE_MACHINE_SNAPSHOT_LEN>::new();
        write_project_state_machine(
            &mut state,
            state_machine_vtable,
            prototype_vtable,
            simple_vtable,
        );
        states.push(state);
    }
    let mut pool = AlignedBytes::<PROJECT_POOL_ARRAY_BULK_LEN>::new();
    assert!(is_map_control_heap_pointer(pool.addr()));
    for (index, local_id) in local_ids.iter().enumerate() {
        write_project_pool_record(
            &mut pool,
            index,
            project_vtable,
            *local_id,
            5_000 + index as u32,
            index % 2 == 0,
            index % 3 == 0,
        );
        pool.write_u64(
            index * PROJECT_COUNTRY_STRIDE + PROJECT_PROJECT_OFF_STATE_MACHINE_PTR,
            states[index].addr() as u64,
        );
    }
    (pool, states)
}

fn decode_both_arms(
    pool_addr: usize,
    count: u32,
    project_vtable: usize,
    state_machine_vtable: usize,
    prototype_vtable: usize,
    simple_vtable: usize,
) -> (CountryLifecycle, CountryLifecycle) {
    let mut buffer = [0u8; PROJECT_POOL_ARRAY_BULK_LEN];
    let mut bulk_walk = Domain41LifecycleWalkStats::default();
    let bulk = read_country_project_lifecycle_bulk(
        pool_addr,
        count,
        project_vtable,
        state_machine_vtable,
        prototype_vtable,
        simple_vtable,
        None,
        &mut buffer,
        &mut bulk_walk,
    )
    .expect("contiguous bulk read accepted");
    let mut scalar_walk = Domain41LifecycleWalkStats::default();
    let scalar = read_country_project_lifecycle_scalar(
        pool_addr,
        count,
        project_vtable,
        state_machine_vtable,
        prototype_vtable,
        simple_vtable,
        None,
        &mut scalar_walk,
    );
    (bulk, scalar)
}

fn lifecycle_kind(read: &CountryLifecycle) -> &'static str {
    match read {
        CountryLifecycle::None => "none",
        CountryLifecycle::Aborted => "aborted",
        CountryLifecycle::Rows { .. } => "rows",
    }
}

/// A torn pool — two records carrying the same `local_id`, not adjacent — must
/// still abort the country WHOLE on both arms.
///
/// This is the invariant the container exists for, and it is the one a
/// cheaper detector is most likely to lose. The duplicate sits at index 3
/// against index 0, so a detector that only compares against the previous id
/// would emit four rows here instead of refusing.
///
/// KILLS: a detector that forgets ids, compares only the last one, or reports
/// a repeat as new. All three turn `aborted` into `rows`, and the fail-closed
/// WAVE5 contract — no partial lifecycle rows for a country whose pool did not
/// pass — is gone with no other symptom.
#[test]
fn a_duplicate_local_id_aborts_the_country_on_both_arms() {
    let _d41_diagnostics = lock_domain41_diagnostics();
    page_cache_new_frame();
    let project_vtable = HEAP_LO + 0x3000;
    let state_machine_vtable = HEAP_LO + 0x4000;
    let prototype_vtable = HEAP_LO + 0x5000;
    let simple_vtable = HEAP_LO + 0x6000;

    let ids = [11u32, 12, 13, 11];
    let (pool, _states) = stage_pool(
        &ids,
        project_vtable,
        state_machine_vtable,
        prototype_vtable,
        simple_vtable,
    );
    let (bulk, scalar) = decode_both_arms(
        pool.addr(),
        ids.len() as u32,
        project_vtable,
        state_machine_vtable,
        prototype_vtable,
        simple_vtable,
    );

    assert_eq!(
        lifecycle_kind(&bulk),
        "aborted",
        "a repeated local id must abort the country on the bulk arm"
    );
    assert_eq!(
        lifecycle_kind(&scalar),
        "aborted",
        "and on the scalar arm, which must not diverge from it"
    );
}

/// A pool filled to exactly `MAX_PROJECTS_PER_COUNTRY` distinct ids must
/// decode in full, on both arms, and to the same rows.
///
/// The upper end of the fixed detector is where an off-by-one lives, and 64 is
/// the count `read_country_project_lifecycle` still accepts
/// (`PROJECT_LIFECYCLE_MAX_PER_COUNTRY`), so this is the largest pool
/// production can present. Compared as rows so the two arms are pinned to each
/// other as well as to the count.
///
/// KILLS: a `self.len < MAX - 1` bound or any capacity slip that makes the
/// 64th distinct id read as a duplicate — the whole country then aborts on the
/// widest pool in the game and on nothing smaller, which is exactly the shape
/// that would ship.
#[test]
fn a_full_pool_of_distinct_local_ids_decodes_on_both_arms() {
    let _d41_diagnostics = lock_domain41_diagnostics();
    page_cache_new_frame();
    let project_vtable = HEAP_LO + 0x3000;
    let state_machine_vtable = HEAP_LO + 0x4000;
    let prototype_vtable = HEAP_LO + 0x5000;
    let simple_vtable = HEAP_LO + 0x6000;

    let ids: Vec<u32> = (1..=MAX_PROJECTS_PER_COUNTRY as u32).collect();
    assert_eq!(ids.len() as u32, PROJECT_LIFECYCLE_MAX_PER_COUNTRY);
    let (pool, _states) = stage_pool(
        &ids,
        project_vtable,
        state_machine_vtable,
        prototype_vtable,
        simple_vtable,
    );
    let (bulk, scalar) = decode_both_arms(
        pool.addr(),
        ids.len() as u32,
        project_vtable,
        state_machine_vtable,
        prototype_vtable,
        simple_vtable,
    );

    let bulk_rows = lifecycle_rows(bulk, 47);
    let scalar_rows = lifecycle_rows(scalar, 47);
    assert_eq!(
        bulk_rows.len(),
        MAX_PROJECTS_PER_COUNTRY,
        "a full pool of distinct ids must decode whole, not abort on its last id"
    );
    assert_eq!(
        bulk_rows, scalar_rows,
        "the two arms must return the same rows for the widest legal pool"
    );
    assert_eq!(
        bulk_rows
            .iter()
            .map(|row| row.local_id)
            .collect::<Vec<u32>>(),
        ids,
        "and in pool order, every id kept"
    );
}

// ------------------------------------------------------------------
// 4. Every exit of the refresh function announces the RIGHT one
// ------------------------------------------------------------------

/// The braced block that opens at the first `{` at or after `anchor`, and the
/// offset just past its closing brace.
///
/// Index ordering is not membership: "after the `if` and before the `else`"
/// is satisfied by a statement sitting in the WRONG arm, and three of test 4's
/// assertions were exactly that shape (Codex review
/// `01a0340e-63cb-7142-87b6-08edafe374d5`, SHOULD-FIX 5). Brace matching says
/// which block a statement is actually in.
fn block_after<'a>(text: &'a str, anchor: &str) -> (&'a str, usize) {
    let at = text
        .find(anchor)
        .unwrap_or_else(|| panic!("anchor `{anchor}` must exist"));
    let after = &text[at..];
    let open = after.find('{').expect("the block opens");
    let mut depth = 0usize;
    for (index, byte) in after.as_bytes().iter().copied().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return (&after[open + 1..index], at + index + 1);
                }
            }
            _ => {}
        }
    }
    panic!("the block after `{anchor}` never closes");
}

/// The first `dictionary.frame_state = ...` assignment after `anchor`.
///
/// Anchors are guard lines, so "the first stamp after the guard" is the stamp
/// inside that guard's block. Returns the bare constant name.
fn state_stamped_after<'a>(body: &'a str, anchor: &str) -> &'a str {
    const STAMP: &str = "dictionary.frame_state = ";
    let at = body
        .find(anchor)
        .unwrap_or_else(|| panic!("anchor `{anchor}` must exist in the refresh function"));
    let after = &body[at + anchor.len()..];
    let stamp = after
        .find(STAMP)
        .unwrap_or_else(|| panic!("no frame_state stamp follows `{anchor}`"));
    let value = &after[stamp + STAMP.len()..];
    let end = value
        .find(';')
        .unwrap_or_else(|| panic!("the stamp after `{anchor}` never ends"));
    value[..end].trim()
}

/// `refresh_project_specialization_dictionary` must label every one of its
/// exits with the CORRECT `D41_SPECIALIZATION_SCAN_*` state, and
/// `read_compact_projects` must carry all five per-frame instruments across
/// to the phase struct.
///
/// A SOURCE PIN, because the function needs a live `CollectorState` and a live
/// heap to drive to any of its interesting exits, and no offline fixture has
/// either. What it protects is the READING of the units: their whole purpose is
/// to say which exit was taken, and a state that names the wrong exit is worse
/// than no state at all — it would send the follow-up lane to the wrong
/// function.
///
/// An earlier version of this test checked only that each `return;` was
/// preceded by SOME state and that each tail token appeared once. It would
/// have passed with all three early returns stamping the same wrong code, with
/// `COMPLETED` and `GAP_DISCARDED` swapped between the two `HEAP_HI` branches,
/// and with the tracker-to-phase readback replaced by a literal zero (Codex
/// review 01a033cf-3756-7103-84e4-bd5b5d28e093, SHOULD-FIX 3). All four of
/// those are killed below.
///
/// ALSO KILLS: a `return;` added without a state; deleting the
/// `..._UNLABELLED_EXIT` initialiser, which would leave a future unstamped exit
/// inheriting the last labelled decision; moving the initialiser below the
/// `wanted.is_empty()` guard, which makes that guard report the previous
/// frame's state; and dropping the explicit `..._NOT_RUN` stamp that makes the
/// source-unavailable arm's published zero a statement.
#[test]
fn every_specialization_refresh_exit_carries_the_right_state_code() {
    let source = attach_log_source();
    let body = function_body(&source, "fn refresh_project_specialization_dictionary(");

    // (a) Every per-frame instrument must be cleared BEFORE the first guard,
    // so a frame can never publish a previous frame's answer.
    //
    // "before the first guard" and not "the first N statements": the
    // requirement is about ordering against the guards, and pinning a literal
    // statement count would fail on a harmless local or assertion added ahead
    // of them (Codex review `01a033dd-4679-7110-9193-4babb2ba925b`, NIT 5).
    let first_guard = body
        .find("if wanted.is_empty() {")
        .expect("the empty-wanted guard must exist");
    for reset in [
        "dictionary.frame_gap_restarts = 0;",
        "dictionary.frame_completions = 0;",
        "dictionary.frame_handle_restarts = 0;",
        "dictionary.frame_sweep_us = 0;",
        "dictionary.frame_state = D41_SPECIALIZATION_SCAN_UNLABELLED_EXIT;",
    ] {
        let at = body
            .find(reset)
            .unwrap_or_else(|| panic!("`{reset}` must exist in the refresh function"));
        assert!(
            at < first_guard,
            "`{reset}` must run before the first guard, or that guard reports \
             the previous frame"
        );
    }

    // (b) Each guard must stamp ITS OWN code, not merely some code. The anchors
    // are the guard lines; the assertion is on the first stamp inside each.
    for (anchor, expected) in [
        (
            "if wanted.is_empty() {",
            "D41_SPECIALIZATION_SCAN_WANTED_EMPTY",
        ),
        (
            "if dictionary.scanned && !has_unresolved {",
            "D41_SPECIALIZATION_SCAN_ALREADY_COVERED",
        ),
        (
            "let Some(token_record_vtable) = dictionary.token_record_vtable else {",
            "D41_SPECIALIZATION_SCAN_VTABLE_UNRESOLVED",
        ),
        // The two `HEAP_HI` arms, pinned so they cannot be swapped: a
        // discarded pass must not be published as a completed census.
        (
            "if dictionary.scan_failed {",
            "D41_SPECIALIZATION_SCAN_GAP_DISCARDED",
        ),
        (
            "dictionary.scan_wanted.clear();",
            "D41_SPECIALIZATION_SCAN_COMPLETED",
        ),
    ] {
        assert_eq!(
            state_stamped_after(&body, anchor),
            expected,
            "`{anchor}` must stamp {expected}"
        );
    }

    // (b2) BOTH new-handle restart branches must count. They reset
    // `scan_cursor` to `HEAP_LO` and discard coverage while incrementing
    // nothing else, so an uncounted one is a silent restart.
    assert_eq!(
        body.matches("dictionary.frame_handle_restarts = 1;")
            .count(),
        2,
        "both `scan_cursor = HEAP_LO` restart branches must count the restart"
    );
    for anchor in [
        "if dictionary.scanned {",
        "} else if dictionary.scan_cursor > HEAP_LO && !new_scan_handles.is_empty() {",
    ] {
        let at = body
            .find(anchor)
            .unwrap_or_else(|| panic!("restart branch `{anchor}` must exist"));
        let after = &body[at + anchor.len()..];
        let stamp = after
            .find("dictionary.frame_handle_restarts = 1;")
            .unwrap_or_else(|| panic!("`{anchor}` must count its restart"));
        let next_branch = after.find("\n    }").unwrap_or(after.len());
        assert!(
            stamp < next_branch,
            "`{anchor}` must count its restart INSIDE its own block"
        );
    }

    // (b3) The three terminal outcomes, checked by BRACE MATCHING and not by
    // index ordering. Each arm must carry its own state and its own event
    // stamp, and must not carry the other arm's.
    //
    // Ordering alone passed a `frame_gap_restarts = 1;` moved into the CLEAN
    // arm just above `dictionary.scanned = true;`, and a
    // `frame_completions = 1;` moved past the whole `if address >= HEAP_HI`
    // statement, which would report a completion on every in-progress call
    // (Codex review `01a0340e-63cb-7142-87b6-08edafe374d5`, SHOULD-FIX 5).
    let (heap_hi_arm, heap_hi_end) = block_after(&body, "if address >= HEAP_HI {");
    let (gap_arm, gap_end) = block_after(heap_hi_arm, "if dictionary.scan_failed {");
    let (clean_arm, _) = block_after(&heap_hi_arm[gap_end..], "else {");
    let (in_progress_arm, _) = block_after(&body[heap_hi_end..], "else {");

    for (arm, name, state, event, foreign_event) in [
        (
            gap_arm,
            "discarded-pass",
            "D41_SPECIALIZATION_SCAN_GAP_DISCARDED",
            Some("dictionary.frame_gap_restarts = 1;"),
            Some("dictionary.frame_completions = 1;"),
        ),
        (
            clean_arm,
            "completed-census",
            "D41_SPECIALIZATION_SCAN_COMPLETED",
            Some("dictionary.frame_completions = 1;"),
            Some("dictionary.frame_gap_restarts = 1;"),
        ),
        (
            in_progress_arm,
            "mid-heap",
            "D41_SPECIALIZATION_SCAN_IN_PROGRESS",
            None,
            None,
        ),
    ] {
        assert_eq!(
            arm.matches(&format!("dictionary.frame_state = {state};"))
                .count(),
            1,
            "the {name} arm must stamp {state} exactly once"
        );
        match event {
            Some(stamp) => assert_eq!(
                arm.matches(stamp).count(),
                1,
                "the {name} arm must carry `{stamp}` exactly once"
            ),
            None => {
                for stamp in [
                    "dictionary.frame_gap_restarts = 1;",
                    "dictionary.frame_completions = 1;",
                ] {
                    assert!(
                        !arm.contains(stamp),
                        "the {name} arm must not carry `{stamp}`"
                    );
                }
            }
        }
        if let Some(stamp) = foreign_event {
            assert!(
                !arm.contains(stamp),
                "the {name} arm must not carry the other arm's `{stamp}`"
            );
        }
    }

    // And exactly once in the whole function, so neither stamp can also sit
    // somewhere the arm scan above does not look.
    for stamp in [
        "dictionary.frame_gap_restarts = 1;",
        "dictionary.frame_completions = 1;",
    ] {
        assert_eq!(
            body.matches(stamp).count(),
            1,
            "`{stamp}` must appear exactly once in the whole function"
        );
    }

    // (b4) The sweep span is taken off the budget's own `Instant`, ONCE, after
    // the outer loop CLOSES and before the cursor is written.
    //
    // Both bounds are load-bearing. "Before the cursor write" alone would pass
    // a stamp moved ABOVE the `while`, which publishes a near-zero sweep on
    // every frame and would be read as "the census is not the cost". The lower
    // bound is the loop's closing brace, found by brace matching — an earlier
    // version used the START of the loop body's last statement, which passed a
    // stamp placed INSIDE either arm of that statement (Codex review
    // `01a0340e-63cb-7142-87b6-08edafe374d5`, SHOULD-FIX 5).
    assert_eq!(
        body.matches("dictionary.frame_sweep_us = duration_micros_u64(started.elapsed());")
            .count(),
        1,
        "the sweep span must be stamped exactly once, off the budget's own clock"
    );
    let sweep_stamp = body
        .find("dictionary.frame_sweep_us = duration_micros_u64(started.elapsed());")
        .expect("the sweep span is stamped");
    let (_, sweep_loop_end) = block_after(&body, "while address < HEAP_HI");
    let cursor_write = body
        .find("dictionary.scan_cursor = address;")
        .expect("the cursor is written");
    assert!(
        sweep_loop_end < sweep_stamp && sweep_stamp < cursor_write,
        "the sweep span must be stamped after the outer loop closes and before \
         the cursor write"
    );

    // (c) Each of the reachable codes is stamped exactly once, so none of the
    // five above can be satisfied by a duplicate somewhere else, and the
    // mid-heap `IN_PROGRESS` arm exists.
    for state in [
        "D41_SPECIALIZATION_SCAN_UNLABELLED_EXIT",
        "D41_SPECIALIZATION_SCAN_WANTED_EMPTY",
        "D41_SPECIALIZATION_SCAN_ALREADY_COVERED",
        "D41_SPECIALIZATION_SCAN_VTABLE_UNRESOLVED",
        "D41_SPECIALIZATION_SCAN_IN_PROGRESS",
        "D41_SPECIALIZATION_SCAN_COMPLETED",
        "D41_SPECIALIZATION_SCAN_GAP_DISCARDED",
    ] {
        assert_eq!(
            body.matches(&format!("dictionary.frame_state = {state};"))
                .count(),
            1,
            "{state} must be stamped exactly once"
        );
    }

    // (d) `IN_PROGRESS` is the LAST stamp in the function, i.e. the arm that
    // runs when the sweep stopped short of `HEAP_HI`. Anything else last means
    // the mid-heap case is being reported as a terminal one.
    let last = body
        .rfind("dictionary.frame_state = ")
        .expect("the function stamps a state");
    assert!(
        body[last..].starts_with("dictionary.frame_state = D41_SPECIALIZATION_SCAN_IN_PROGRESS;"),
        "the mid-heap arm must be the function's last state stamp"
    );

    // (e) Every bare `return;` is immediately preceded by a stamp.
    let lines: Vec<&str> = body.lines().map(str::trim).collect();
    let mut labelled_returns = 0usize;
    for (index, line) in lines.iter().enumerate() {
        if *line != "return;" {
            continue;
        }
        let previous = lines[..index]
            .iter()
            .rev()
            .find(|candidate| !candidate.is_empty() && !candidate.starts_with("//"))
            .copied()
            .unwrap_or("");
        assert!(
            previous.starts_with("dictionary.frame_state = D41_SPECIALIZATION_SCAN_"),
            "unlabelled return in refresh_project_specialization_dictionary, \
             preceded by `{previous}`"
        );
        labelled_returns += 1;
    }
    assert_eq!(
        labelled_returns, 3,
        "the function has exactly three early returns -- empty wanted, already \
         covered, and an unresolved token record vtable"
    );

    // (f) THE BRIDGE, in order. Everything above is worthless if the reader
    // publishes a constant instead of what the tracker holds — and presence
    // alone is not enough, because a readback moved ABOVE the classify call
    // publishes the previous frame and a literal added BELOW it overwrites the
    // right answer. Both were possible against the first version of this check
    // (Codex review `01a033dd-4679-7110-9193-4babb2ba925b`, SHOULD-FIX 4).
    let reader = function_body(&source, "fn read_compact_projects(");
    let flat = reader.split_whitespace().collect::<Vec<&str>>().join(" ");

    let classify = flat
        .find("classify_project_breakthrough_specializations(")
        .expect("the reader must call the classifier");
    let source_unavailable = flat
        .find("return compact_projects_finish(")
        .expect("the reader must have its source-unavailable early return");

    // The NOT_RUN default is the value the source-unavailable arm publishes,
    // so it has to be stamped before that arm returns.
    let not_run = flat
        .find("phase.specialization_scan_state_sample = D41_SPECIALIZATION_SCAN_NOT_RUN;")
        .expect("the reader must stamp the NOT_RUN default");
    assert!(
        not_run < source_unavailable,
        "the NOT_RUN default must be stamped before the source-unavailable return"
    );

    // Every readback must come AFTER the classify call, or it publishes the
    // previous frame's decision.
    for statement in [
        "phase.specialization_scan_gap_restarts = tracker.specialization_dictionary.frame_gap_restarts;",
        "phase.specialization_scan_completions = tracker.specialization_dictionary.frame_completions;",
        "phase.specialization_scan_state_sample = tracker.specialization_dictionary.frame_state;",
        "phase.specialization_scan_handle_restarts = tracker.specialization_dictionary.frame_handle_restarts;",
        "phase.specialization_sweep_us = tracker.specialization_dictionary.frame_sweep_us;",
    ] {
        let at = flat
            .find(statement)
            .unwrap_or_else(|| panic!("read_compact_projects must carry `{statement}`"));
        assert!(
            at > classify,
            "`{statement}` must read the tracker AFTER the classify call"
        );
    }

    // And each phase field is assigned exactly the number of times it should
    // be, so nothing can be overwritten after the correct readback.
    for (field, times) in [
        ("phase.specialization_scan_gap_restarts =", 1usize),
        ("phase.specialization_scan_completions =", 1),
        ("phase.specialization_scan_handle_restarts =", 1),
        ("phase.specialization_sweep_us =", 1),
        // Twice, and only twice: the NOT_RUN default, then the readback.
        ("phase.specialization_scan_state_sample =", 2),
        ("phase.lifecycle_pool_copy_us =", 1),
    ] {
        assert_eq!(
            flat.matches(field).count(),
            times,
            "`{field}` must be assigned exactly {times} time(s) in read_compact_projects"
        );
    }
    assert!(
        flat.contains("phase.lifecycle_pool_copy_us = lifecycle_read.pool_copy_us;"),
        "the pool-copy timer must be carried off the lifecycle read"
    );
}

// ------------------------------------------------------------------
// 5. The cut is still wired into production
// ------------------------------------------------------------------

/// `ProjectLocalIdSeen` must still BE a bounded array, and both decode arms
/// must still use it.
///
/// TWO HALVES, because either alone is a hole. Pinning only the call sites
/// leaves the detector's own representation free to change: swapping the
/// `[u32; 64]` inside `ProjectLocalIdSeen` for a capped `BTreeSet<u32>` keeps
/// every answer, including the fail-closed 65th, and passes tests 1-3 and the
/// call-site half of this one — while giving the lane's entire saving back
/// (Codex review `01a0342a-9961-73e0-afaf-def32f07c96e`, SHOULD-FIX 2).
/// Pinning only the definition leaves the arms free to stop calling it.
///
/// Tests 1-3 are all about behaviour, and reverting either arm to
/// `BTreeSet<u32>` preserves every byte they assert — it only gives the saving
/// back. So all three would pass against a tree in which this lane's one
/// performance cut had been silently undone (Codex review
/// `01a033dd-4679-7110-9193-4babb2ba925b`, SHOULD-FIX 3). Nothing else in the
/// tree can notice, because the container is invisible in every output.
///
/// KILLS: a revert of either arm to `BTreeSet::new()` / `HashSet::new()`; a
/// merge that resolves this hunk in favour of the old side on one arm only,
/// which is how the two arms come apart.
#[test]
fn both_lifecycle_decode_arms_still_use_the_bounded_seen_detector() {
    let source = attach_log_source();

    // (a) THE REPRESENTATION, pinned WHOLE rather than by properties.
    //
    // Property assertions do not close a representation swap, and two rounds
    // of trying proved it. Requiring the two fields to be PRESENT allowed a
    // third field `store: SeenStore` -- an alias for `BTreeSet<u32>` declared
    // outside the inspected body -- to carry the real predicate; requiring the
    // live-prefix expression to OCCUR allowed it to sit there unused. Every
    // answer, including the fail-closed 65th, stayed identical, both arms
    // still instantiated `ProjectLocalIdSeen`, and no banned spelling appeared
    // inside the bodies (Codex review
    // `01a03436-18dc-7112-ac3f-660d5d5c15aa`, SHOULD-FIX 2).
    //
    // So the whole of both bodies is pinned, comments stripped and whitespace
    // collapsed. This is twenty lines of code that ARE the lane's cut, and a
    // pin over all of it is the only form that closes the class rather than
    // one member of it. It is deliberately brittle: any change to the
    // detector, including a harmless rename, must come here and be looked at.
    let normalised = |text: &str| -> String {
        text.lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<&str>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<&str>>()
            .join(" ")
    };
    assert_eq!(
        normalised(function_body(&source, "struct ProjectLocalIdSeen {")),
        "ids: [u32; MAX_PROJECTS_PER_COUNTRY], len: usize,",
        "the detector must be exactly a bounded array plus a length -- no \
         extra field, whatever it is called"
    );
    assert_eq!(
        normalised(function_body(&source, "impl ProjectLocalIdSeen {")),
        "fn new() -> Self { Self { ids: [0; MAX_PROJECTS_PER_COUNTRY], len: 0, \
         } } fn insert(&mut self, id: u32) -> bool { if \
         self.ids[..self.len].contains(&id) { return false; } let Some(slot) \
         = self.ids.get_mut(self.len) else { return false; }; *slot = id; \
         self.len += 1; true }",
        "the detector's `new` and `insert` must be exactly the array scan -- \
         no allocation, no second store, no other state transition"
    );

    // (b) THE CALL SITES.
    for signature in [
        "fn decode_country_project_lifecycle_from_bytes(",
        "fn decode_country_project_lifecycle_scalar(",
    ] {
        let body = function_body(&source, signature);
        // CODE ONLY, and for EVERY assertion below -- not just the banned-name
        // scan. The comment above the declaration names the container this
        // lane replaced, so an unstripped search for the declaration finds it
        // in that comment: a commented-out declaration plus
        // `let mut seen_ids = SeenIds::new();`, where `SeenIds` aliases
        // `BTreeSet<u32>`, passed the first version of this test and all four
        // others (Codex review 01a033e9-b478-7141-96c8-880dd283e66c,
        // SHOULD-FIX 3).
        let code: String = body
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<&str>>()
            .join("\n");
        assert!(
            code.contains("let mut seen_ids = ProjectLocalIdSeen::new();"),
            "{signature} must build its duplicate detector with ProjectLocalIdSeen"
        );
        // EXACTLY TWO MENTIONS OF THE IDENTIFIER: the declaration asserted
        // above, and the use site asserted below. Nothing else.
        //
        // Counting a spelling does not work, and two rounds of trying proved
        // it. `let mut seen_ids` missed `let seen_ids = &mut SeenIds::new();`;
        // `seen_ids =` then missed `let (seen_ids,) = (&mut replacement,);`,
        // which contains no `seen_ids =` substring at all (Codex reviews
        // `01a0340e-63cb-7142-87b6-08edafe374d5` SHOULD-FIX 6 and
        // `01a0341f-b115-7322-adc1-1961da7f8b61` SHOULD-FIX 2). Every shadow
        // has to NAME the binding it introduces, whatever syntax it uses, so
        // counting mentions of the name closes the family rather than one
        // member of it.
        //
        // It is deliberately tight: a third legitimate use of `seen_ids` fails
        // this and should, because the whole argument for byte-neutrality is
        // that the detector is consulted in exactly one expression.
        assert_eq!(
            code.matches("seen_ids").count(),
            2,
            "{signature} must mention `seen_ids` exactly twice -- the \
             declaration and the abort test. A third mention is either a \
             shadowing binding or a second consultation, and both break the \
             byte-identity argument"
        );
        // And the thing it builds must be the thing the abort test consults.
        // A declaration nothing reads is not a cut, it is a warning.
        assert!(
            code.contains("if !seen_ids.insert(local_id) {"),
            "{signature} must gate the country abort on `seen_ids.insert(local_id)`"
        );
        for heap_container in ["BTreeSet", "HashSet", "BTreeMap", "HashMap"] {
            assert!(
                !code.contains(heap_container),
                "{signature} must not reintroduce a heap {heap_container} on the \
                 per-project path"
            );
        }
    }
}
