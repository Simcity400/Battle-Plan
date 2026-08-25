// Lane SQ (2026-08-25): d53's clock bracket covers its engine reads and
// nothing after them.
//
// WHAT THIS FILE IS FOR. Lane SO measured 4,828 us/frame of PAYLOAD work inside
// d53's `ReaderClockBracketGuard` (`runs/so_snapshot_diff/MEASUREMENT.md`
// section 3) and showed the builder that dominates it cannot read engine memory
// -- `build_domain53_payload_with_status` takes no `Domain53Memory`. It also
// found the one thing in that block that DOES read: the A30 authority publish,
// which takes the game hour, the `CHuman` pointer and the player tag whenever
// A30 is armed, either by an evidence request or by `A30ProductionOwner::begin`
// installing the controller. So the split could not be justified by the
// builder's signature alone.
//
// The shape that closes it: the A30 reads are separated from the report they
// were bundled with and taken as the LAST engine read of the observe half, and
// everything downstream of every read -- the whole payload phase, the A30
// publish included -- runs after the bracket.
//
// THE LICENCE IS "NO READ IS REACHABLE FROM THE DEFERRED HALF", not "no address
// crosses". An earlier draft of this file and of `SPLIT.md` claimed the second,
// and it is FALSE: `A30Domain53EngineReads::game_state` is the game-state
// pointer, carried across as the authority's identity (Codex round 1, blocking
// 2). It is dereferenced nowhere, and it already outlives every bracket in the
// process-global authority slot, so carrying it inside one worker adds no
// exposure -- but the guard has to enforce the true statement, so it walks the
// in-file callee closure for the three primitives every live read in this file
// bottoms out in, and pins the readings struct's field list so a SECOND address
// cannot arrive unannounced.

use super::state_fixture::CollectorStateFixture;
use super::*;
use std::collections::BTreeSet;
use std::sync::atomic::Ordering;

/// The source of one item, from its signature to its own closing brace.
///
/// A top-level item in this file is the only thing that closes on a `}` in
/// column zero, so that is the terminator; ending at "the next `fn`" instead
/// would swallow the following item's doc comment, and these guards scan for
/// identifiers those comments discuss.
fn item_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("{signature} must exist"));
    let end = source[start..]
        .find("\n}\n")
        .map(|at| start + at + 3)
        .unwrap_or_else(|| panic!("{signature} must close on a column-zero brace"));
    &source[start..end]
}

/// `body` with every whole-line `//` comment and every blank line removed, so a
/// scan for a token cannot be satisfied -- or defeated -- by prose. The
/// declarations under test discuss the very identifiers they must not USE.
fn code_only(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// THE WHOLE LIVE-READ SURFACE OF `attach_log.rs`, established by ENUMERATING
/// CALL SITES rather than by trusting a list or a comment.
///
/// An earlier draft was a list someone assembled, and it was incomplete (Codex
/// round 1, blocking 2). This is what the file actually contains:
///
/// * `copy_selected` (`attach_log.rs`, the SEH/`ReadProcessMemory` copy) is the
///   function that moves bytes out of the game's address space on the DECODE
///   path, and it has exactly THREE callers: `read_scalar`,
///   `read_bytes_with_reason` and `read_into`. That is what makes THE DECODE
///   SEAM's "exactly three primitives" a fact about the file rather than a
///   claim about it -- every named reader (`read_u64`, `read_u32`, `read_i32`,
///   `read_u8`, `read_u16`, `read_f32`, every `*_stable` wrapper) is a
///   composition of those three.
/// * The decode path is not the whole file. `read_bytes_scan_reporting` calls
///   `ReadProcessMemory` DIRECTLY, for the heap sweep's bulk scan. `rpm_exact`
///   and `seh_read_exact` are `copy_selected`'s two arms, and `ntrpm_exact` is a
///   third raw wrapper the compiler currently reports as never used. All four
///   are named below, so the guard does not lean on the catch-all to see them.
///
/// COUNTED, NOT BELIEVED. `attach_log.rs` holds exactly three call sites of the
/// OS read primitives, excluding their `extern` declarations: one
/// `ReadProcessMemory` in `rpm_exact`, one `NtReadVirtualMemory` in the unused
/// `ntrpm_exact`, and one `ReadProcessMemory` in `read_bytes_scan_reporting`.
/// `seh_read_exact` is the SEH arm and has exactly one caller,
/// `copy_selected`. There is no fifth route out.
///
/// `unsafe` IS THE CATCH-ALL, AND IT IS DELIBERATELY OVER-STRICT. Calling an
/// `unsafe fn` requires an `unsafe` block in the CALLER, so any new raw read
/// primitive is visible in the body of whatever reaches it, whatever it is
/// named. It will also fail on an `unsafe` that has nothing to do with reading
/// -- which is the right failure rather than a false one: a new `unsafe` on the
/// post-bracket side of a clock bracket is a question somebody should answer.
const LIVE_READ_PRIMITIVES: &[&str] = &[
    "read_scalar",
    "read_bytes_with_reason",
    "read_into(",
    "read_bytes_scan",
    "copy_selected",
    "rpm_exact",
    "seh_read_exact",
    "ReadProcessMemory",
    "NtReadVirtualMemory",
    "unsafe",
];

/// The handle types a value would have to carry for a callee to reach memory
/// through it, and the field a `CollectorState` is reached through.
///
/// Distinct from [`LIVE_READ_PRIMITIVES`] because they answer different
/// questions: these say what may not be DECLARED, those say what may not be
/// CALLED.
const ENGINE_HANDLES: &[&str] = &[
    "CollectorState",
    "Domain53InProcessMemory",
    "Domain53Discovery",
    "date_ptr",
];

/// EVERY identifier token in `body`, deduplicated — not only the ones followed
/// by `(`.
///
/// IT USED TO COLLECT CALL SITES ONLY, AND THAT WAS A HOLE. Codex round 2
/// (blocking 2) named the live instance: `compact_domain53_apply_emit_mode`
/// hands `domain53_deleted_country`, `domain53_deleted_war` and eight siblings
/// to `domain53_emit_map_delta` as `fn` VALUES. A function passed by name has no
/// parenthesis after it, so the walk reached the higher-order function and never
/// reached a single callback body. They are read-free today; a read added to one
/// would have passed a guard that claimed to cover them.
///
/// Taking every token is the fix and it is sound in the direction that matters:
/// the walk only follows a token that RESOLVES to a top-level free function of
/// this file, so a local variable, a field, a type or a keyword that happens to
/// share a name is either skipped or scanned needlessly. Over-scanning can only
/// make the guard stricter. Under-scanning is what let a callback through.
///
/// It also picks up names that appear only in a `use` or a turbofish, and names
/// merely MENTIONED in code (never in comments — `code_only` has already
/// stripped those). Both are over-approximation, and both are fine.
fn called_identifiers(body: &str) -> BTreeSet<String> {
    let bytes = body.as_bytes();
    let mut out = BTreeSet::new();
    let mut start = None;
    for (at, byte) in bytes.iter().enumerate() {
        let word = byte.is_ascii_alphanumeric() || *byte == b'_';
        match (word, start) {
            (true, None) => start = Some(at),
            (false, Some(from)) => {
                out.insert(body[from..at].to_string());
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        out.insert(body[from..].to_string());
    }
    out
}

/// Every top-level FREE FUNCTION this file defines, by name, as source text.
///
/// Built once: the walk below asks for hundreds of names and a `find` over six
/// megabytes per ask is a test nobody would keep. Only column-zero `fn` items
/// are indexed -- methods live in `impl` blocks and are handled by naming the
/// one `impl` the deferred half reaches as a walk root.
///
/// A duplicated name would be an ambiguity, so the first definition wins and
/// the second is ignored; there are none today and one appearing would make the
/// guard weaker rather than wrong.
///
/// `unsafe fn` IS INDEXED FOR RECURSION DEPTH ONLY. Every raw read primitive in
/// this file is one, and reaching one already trips the `unsafe` token in the
/// CALLER's body, so indexing them cannot be what catches a read -- it only
/// lets the walk keep going through one instead of stopping at its name.
fn free_function_bodies(source: &str) -> BTreeMap<String, &str> {
    let mut out: BTreeMap<String, &str> = BTreeMap::new();
    for prefix in [
        "\nfn ",
        "\npub fn ",
        "\npub(super) fn ",
        "\npub(crate) fn ",
        "\nunsafe fn ",
        "\npub unsafe fn ",
    ] {
        let mut cursor = 0usize;
        while let Some(offset) = source[cursor..].find(prefix) {
            // `+ 1` skips the leading newline the prefix carries, so the body
            // starts at the item's own first column.
            let item_start = cursor + offset + 1;
            let head = cursor + offset + prefix.len();
            cursor = head;
            let name_len = source[head..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(0);
            if name_len == 0 {
                continue;
            }
            let Some(to) = source[item_start..].find("\n}\n") else {
                continue;
            };
            out.entry(source[head..head + name_len].to_string())
                .or_insert(&source[item_start..item_start + to + 3]);
        }
    }
    out
}

/// NO ENGINE READ IS REACHABLE FROM d53'S DEFERRED HALF, walked rather than
/// asserted.
///
/// THIS IS THE WHOLE LICENCE FOR THE SPLIT. A clock bracket certifies one
/// thing: that every engine read the worker made fell inside one game hour.
/// Running work after the bracket is safe exactly when that work makes no such
/// read -- and "exactly when" has to be checkable, because the failure is
/// silent.
///
/// An earlier version of this guard scanned two declarations for a hand-written
/// list of read primitives and stopped there. Codex round 1 (blocking 2) showed
/// both halves of that were wrong: the list omitted primitives, and a read
/// through a callee would have passed regardless. This walks the CALL GRAPH
/// inside this file, from the finish half and the two declarations that feed it,
/// and fails on any of [`LIVE_READ_PRIMITIVES`] -- the three functions THE
/// DECODE SEAM says every live read in this file bottoms out in, plus the
/// `copy_selected`/`unsafe` a new primitive would need.
///
/// `impl Domain53BaselineCommit` is a root because `baseline_commit.commit(..)`
/// is a METHOD call and the walk resolves free functions; it is the only in-file
/// method the deferred half reaches.
///
/// WHAT IS OUTSIDE THIS FILE is audited in writing rather than here, in
/// `runs/sq_d53_phase_split/SPLIT.md`: the payload builder, the payload
/// encoder, the status-origin recorder and the A30 network witness. That audit
/// is per-callee REACHABILITY and not a claim about the crate as a whole --
/// "the `diplomacy53` crate's only memory interface is `Domain53Memory`" was an
/// earlier version of it and is false, because `a30_visibility.rs` carries its
/// own `ReadProcessMemory` route through `read_value`/`read_stable` (Codex round
/// 2, blocking 2). That route exists; nothing the deferred half calls reaches
/// it.
///
/// kills: a read primitive called anywhere in the finish half OR in anything in
/// this file it calls; a `state`/memory parameter added to
/// `finish_compact_diplomacy_intel_operatives_live`; an engine handle added to
/// `Domain53PayloadWork`; the A30 engine reads moved back into the publish they
/// were split out of.
#[test]
fn no_engine_read_is_reachable_from_the_deferred_half_of_d53() {
    let source = super::collector_source_lf();
    let roots = [
        "fn finish_compact_diplomacy_intel_operatives_live(",
        "struct Domain53PayloadWork<'t> {",
        "struct A30Domain53EngineReads {",
        "impl Domain53BaselineCommit {",
    ];

    // DECLARATIONS FIRST: what may not be held. `game_state` is the one address
    // that crosses and it is pinned by its own test, so the handles below are
    // the types a callee could reach memory THROUGH.
    for signature in roots {
        let code = code_only(item_body(&source, signature));
        assert!(
            code.len() > 60,
            "`{signature}` must resolve to the real item, not a fragment; saw {code}",
        );
        for token in ENGINE_HANDLES {
            assert!(
                !code.contains(token),
                "`{signature}` declares `{token}`, which is a handle the deferred half could \
                 reach engine memory through:\n{code}",
            );
        }
    }

    // THEN THE WALK: what may not be CALLED, transitively, inside this file.
    let defined = free_function_bodies(&source);
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut resolved: BTreeSet<String> = BTreeSet::new();
    let mut frontier: Vec<String> = Vec::new();
    let mut bodies_walked = 0usize;

    let inspect = |named: &str, body: &str, frontier: &mut Vec<String>,
                   visited: &mut BTreeSet<String>| {
        let code = code_only(body);
        for token in LIVE_READ_PRIMITIVES {
            assert!(
                !code.contains(token),
                "`{named}` is reachable from d53's deferred half and reaches live process \
                 memory through `{token}`. That read would happen AFTER the clock bracket \
                 that certified this frame's hour, which is the deferred-read \
                 misattribution shape:\n{code}",
            );
        }
        for name in called_identifiers(&code) {
            if visited.insert(name.clone()) {
                frontier.push(name);
            }
        }
    };

    for signature in roots {
        bodies_walked += 1;
        let body = item_body(&source, signature);
        inspect(signature, body, &mut frontier, &mut visited);
    }
    while let Some(name) = frontier.pop() {
        let Some(body) = defined.get(&name) else {
            // Not a free function of this file: a constructor, a variant, a
            // macro, a method, or a callee defined elsewhere. The out-of-file
            // set is audited in SPLIT.md; see this test's header.
            continue;
        };
        bodies_walked += 1;
        resolved.insert(name.clone());
        assert!(
            bodies_walked < 4_000,
            "the reachable set exploded past any plausible bound -- the resolver is \
             matching something it should not",
        );
        inspect(&name, body, &mut frontier, &mut visited);
    }
    assert!(
        bodies_walked > 8,
        "the walk must actually reach the deferred half's callees; it visited only \
         {bodies_walked} bodies, which means the resolver stopped matching",
    );
    // THE CALLBACK CHECK, and it is the one that kills the specific hole Codex
    // round 2 found. `domain53_deleted_war` is reachable ONLY as a `fn` VALUE
    // handed to `domain53_emit_map_delta`; a scanner that collects call sites
    // rather than tokens reaches the higher-order function and never opens a
    // single callback. If this name is missing, the walk's coverage claim is
    // false whatever the body count says.
    assert!(
        resolved.contains("domain53_deleted_war"),
        "the walk must reach functions passed BY NAME as `fn` values, not only ones \
         spelled with a call parenthesis; it resolved {} bodies and this was not one",
        resolved.len(),
    );
}

/// The A30 readings struct carries EXACTLY these four fields.
///
/// `game_state` is an engine ADDRESS and it crosses the bracket. That is sound
/// for reasons that are specific to THAT pointer: nothing in this worker
/// dereferences it, the A30 owners that do dereference it downstream already do
/// so from the capture loop outside every bracket, and it is the
/// process-lifetime game-state singleton rather than a per-frame object. None
/// of the three would automatically hold of a second address someone added
/// here later. The other fields are readings and a flag, not addresses.
///
/// Pinning the list is what turns "no other address crosses" from a sentence in
/// a header into something that fails.
///
/// kills: any field added to, removed from or retyped in
/// `A30Domain53EngineReads` -- which is exactly the review that a new address
/// needs.
#[test]
fn the_a30_engine_readings_are_exactly_these_four() {
    let source = super::collector_source_lf();
    let declared = code_only(item_body(&source, "struct A30Domain53EngineReads {"))
        .lines()
        .filter(|line| line.ends_with(','))
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(
        declared,
        vec![
            "game_state: usize,".to_string(),
            "frame_hour: u64,".to_string(),
            "owner_tag_db_id: u32,".to_string(),
            "evidence_requested: bool,".to_string(),
        ],
        "the readings that cross d53's bracket are fixed: one address carried as the \
         authority's identity, two readings and one arming flag",
    );
}

/// The A30 authority's engine reads are all in the half that runs in the
/// bracket, and none are in the half that runs after it.
///
/// The publish used to be one function that read three values and then consumed
/// the domain's built report. That pairing is what pinned the payload phase
/// inside the bracket: the report is built by the payload phase, so deferring
/// the payload phase deferred the reads with it. Splitting it is only a fix if
/// the reads really did all land on the observing side.
///
/// kills: a read primitive reintroduced into `publish_a30_domain53_authority`;
/// a read moved out of `read_a30_domain53_engine_authority` (which would then
/// have to happen somewhere else, and the only somewhere else is after the
/// bracket).
#[test]
fn every_a30_domain53_engine_read_is_in_the_observing_half() {
    let source = super::collector_source_lf();
    let observing = code_only(item_body(&source, "fn read_a30_domain53_engine_authority("));
    for token in ["read_i32_stable", "read_u64_stable", "read_u32_stable"] {
        assert!(
            observing.contains(token),
            "the observing half must still take the reading `{token}` makes; saw {observing}",
        );
    }
    let consuming = code_only(item_body(&source, "fn publish_a30_domain53_authority("));
    assert!(
        consuming.contains("A30_DOMAIN53_AUTHORITY"),
        "`publish_a30_domain53_authority` must resolve to the half that writes the \
         authority slot; saw {consuming}",
    );
    for token in LIVE_READ_PRIMITIVES.iter().chain(ENGINE_HANDLES) {
        assert!(
            !consuming.contains(token),
            "`publish_a30_domain53_authority` reaches engine memory through `{token}`, and it \
             runs after d53's clock bracket has closed:\n{consuming}",
        );
    }
}

/// A DISARMED A30 PERFORMS ZERO GUARDED READS, counted rather than inferred.
///
/// The early return is what keeps the canonical passive capture read-free on
/// this path, and it is the reason the observe half can call this
/// unconditionally without adding three reads to every frame of every capture.
///
/// AN EARLIER VERSION ASSERTED ONLY `is_none()`, and Codex round 1
/// (non-blocking 3) was right that this proves nothing about reads: a mutation
/// that reads and THEN answers `None` stayed green. This arms the collector's
/// own read counter and asserts it does not move. `rc_read_probe` bumps
/// `RC_READS` at the top of each of the three primitives every guarded read in
/// this file bottoms out in, so an unmoved counter is a direct observation that
/// no read happened, not an inference from the answer.
///
/// The pointer is staged READABLE so the counter is the only thing doing the
/// work: were the early return deleted, the call would succeed at the game hour
/// and bump the counter rather than failing early for an unrelated reason.
///
/// `lock_rc_counters()` is mandatory and not caution: `RC_ENABLED` is
/// process-global and `cargo test` runs this binary's tests concurrently. The
/// guard serialises this test against every other test that arms the gate or
/// drives a frame, and disarms the gate on the way out even if an assertion
/// unwinds. `RC_LOCAL` is thread-local, so the counter itself belongs to this
/// test alone -- which is also why `RC_COUNTER_LOCK`'s rule that asserted
/// DOMAIN SINK slots must live in 213..=219 does not apply here: this asserts
/// on the thread-local counter and drains no sink.
///
/// kills: deleting the disarmed early return in
/// `observe_a30_domain53_authority`; moving the arming test after the first
/// read; adding any guarded read to the disarmed path at all, whether or not it
/// changes the answer.
#[test]
fn a_disarmed_a30_takes_no_domain53_engine_read() {
    let _rc = lock_rc_counters();
    let mut fixture = CollectorStateFixture::new(
        &[compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA],
        0x1000,
        0x2000,
    );
    // Stages a READABLE game hour at `date_ptr + GS_CURRENT_DATE_OFFSET`, which
    // is the first thing the reading half would touch.
    fixture.stage_scheduler_clock_at(0x100);
    let state = fixture.collector_state();

    let reads_now = || RC_LOCAL.with(|local| local.v[RC_READS].get());
    RC_ENABLED.store(true, Ordering::Relaxed);
    // The counter is armed but nothing has read yet on this thread; take the
    // baseline AFTER arming so whatever the fixture did is excluded.
    let before = reads_now();
    let observed = observe_a30_domain53_authority(&state);
    let after = reads_now();
    RC_ENABLED.store(false, Ordering::Relaxed);

    assert_eq!(
        after, before,
        "a disarmed A30 must perform NO guarded read; the counter moved by {}, which is \
         that many reads added to every frame of every passive capture",
        after.saturating_sub(before),
    );
    assert!(
        observed.is_none(),
        "and it must answer `None`, so the consuming half does nothing at all",
    );

    // THE COUNTER REALLY IS WATCHING, or the assertion above is vacuous: one
    // guarded read of the staged hour, taken here rather than assumed, has to
    // move it. This is the same address the reading half would start from.
    let before_probe = reads_now();
    RC_ENABLED.store(true, Ordering::Relaxed);
    let hour = read_i32_stable((state.date_ptr as usize) + GS_CURRENT_DATE_OFFSET);
    RC_ENABLED.store(false, Ordering::Relaxed);
    assert!(
        hour.is_some(),
        "the fixture must stage a readable game hour, or the zero above is a refused \
         read rather than an absent one",
    );
    assert!(
        reads_now() > before_probe,
        "`RC_READS` must move for a read that DID happen; if it does not, the zero above \
         proves nothing",
    );
}

/// d53's bracket covers the observe half and NOT ONE STATEMENT MORE.
///
/// The guard's `Drop` takes the closing clock observation, so every statement
/// inside the guarded block is time the bracket has to vouch for. This helper
/// exists to make that block exactly the reads; a statement added to it puts
/// non-read work back inside the span the split was built to empty, and a
/// `finish` call hoisted into it puts all 4.8 ms back.
///
/// kills: `finish` called inside the guarded block; the payload work built
/// inside it; any bookkeeping statement added between the guard and `observe`.
#[test]
fn the_finishing_helpers_bracket_holds_the_observe_call_alone() {
    let source = super::collector_source_lf();
    let helper = item_body(&source, "fn spawn_gated_finishing<");
    let open = helper
        .find("let observed = {")
        .expect("the guarded block must be bound to `observed`");
    let close = helper[open..]
        .find("\n        };")
        .map(|at| open + at)
        .expect("the guarded block must close");
    let guarded = code_only(&helper[open..close]);
    assert_eq!(
        guarded,
        "let observed = {\nlet _clock_guard = ReaderClockBracketGuard::new(lanes, tag);\nobserve()",
        "the bracket must hold the guard and the observe call and nothing else",
    );
    let finish_at = helper
        .find("let out = finish(observed);")
        .expect("`finish` must be called");
    assert!(
        finish_at > close,
        "`finish` must run after the guarded block has closed, not inside it",
    );
    let stamp_at = helper
        .find("let bracket_closed = lanes.origin.elapsed();")
        .expect("the split boundary must be stamped");
    assert!(
        close < stamp_at && stamp_at < finish_at,
        "the split boundary must be stamped after the guard drops and before `finish`, so \
         `lw_d*_inb_us` carries the guard's own closing clock read",
    );
    // THE MARKER IS THE FIRST STATEMENT AFTER THE GUARD, with nothing between.
    // It stood one line later, after the boundary stamp, and Codex round 2
    // (blocking 1) was right that the gap was real: a panic in that statement
    // would have lost the `worker_panicked` verdict exactly as a panic in
    // `finish` would. Whether `Instant::elapsed()` can panic is now moot.
    //
    // COMPARED AS AN EXACT SLICE, not as an ordering. `close < marker < stamp`
    // was the first spelling and Codex round 3 (non-blocking 4) was right that
    // it admits any statement someone inserts in the gap -- which is precisely
    // the defect round 2 found, re-openable. This asserts the code between the
    // guarded block's close and the marker is EMPTY.
    let marker = "let _panic_marker = ClosedBracketPanicMarker { lanes, tag };";
    let marker_at = helper
        .find(marker)
        .expect("the closed-bracket panic marker must be armed");
    assert!(
        marker_at > close,
        "the marker must be armed after the guarded block, not inside it",
    );
    let between = code_only(&helper[close + "\n        };".len()..marker_at]);
    assert!(
        between.is_empty(),
        "nothing may run between the guard's drop and the panic marker; found:\n{between}",
    );
    assert!(
        marker_at < stamp_at,
        "and the marker must precede the boundary stamp",
    );
}

/// The helper keeps the permit across `finish`, prices the WHOLE worker, and
/// adds exactly ONE statement under the permit that `spawn_gated` did not have.
///
/// The first two are deliberate refusals of a wider change. Releasing the permit
/// before `finish` would admit another domain's READS into a window this worker
/// is still burning CPU in -- a scheduler change, with its own hour-boundary
/// consequences, that this lane did not measure and therefore did not make.
/// Ranking the reader-gate EWMA or the lane span on the observe half alone would
/// tell the frame this domain got cheaper when only its BRACKET did.
///
/// The third is Codex round 1, non-blocking 4. The write-up claims no other
/// reader's window moves; that claim is only true if the added work under the
/// permit is the `bracket_closed` stamp and nothing else. `record_bracket_split`
/// is a mutex push and it now runs AFTER `drop(permit)` -- which is where
/// `spawn_gated_split` does NOT put it, deliberately, because a convention is
/// worth less than the claim.
///
/// kills: `drop(permit)` hoisted above `finish`; `lane_cost_observe` or
/// `lanes.record` narrowed to `bracket_closed`; the split pair dropped; the
/// split pair moved back under the permit.
#[test]
fn the_finishing_helper_holds_the_permit_and_prices_the_whole_worker() {
    let source = super::collector_source_lf();
    let helper = item_body(&source, "fn spawn_gated_finishing<");
    let at = |needle: &str| {
        helper
            .find(needle)
            .unwrap_or_else(|| panic!("`{needle}` must appear in the helper"))
    };
    let marker = at("let _panic_marker = ClosedBracketPanicMarker { lanes, tag };");
    let stamp = at("let bracket_closed = lanes.origin.elapsed();");
    let finish = at("let out = finish(observed);");
    let end = at("let end = lanes.origin.elapsed();");
    let rank = at("lane_cost_observe(tag, duration_micros_u64(end.saturating_sub(start)));");
    let release = at("drop(permit);");
    let split = at("lanes.record_bracket_split(");
    let record = at("lanes.record(tag, lane, start, end);");
    assert!(
        marker < stamp && stamp < finish && finish < end && end < rank && rank < release,
        "order under the permit must be: panic marker, boundary stamp, finish, end stamp, \
         EWMA over the whole worker, permit release -- and those first three plus the \
         guard's own closing clock read are the WHOLE of what the permit window gained",
    );
    assert!(
        release < split && split < record,
        "both lane-tracker pushes must happen AFTER the permit is released, or a queued \
         reader's READ starts later than it did before this helper existed",
    );
}

/// A PANIC IN THE DEFERRED HALF STILL REACHES RECONCILIATION AS A WORKER PANIC.
///
/// Under `spawn_gated` the guard was alive across the whole worker body, so a
/// panic in the payload phase recorded a `worker_panicked` bracket ->
/// `BracketHourVerdict::WorkerPanic` -> a `DomainGapReason::WorkerPanic` gap ->
/// `reset_trackers_after_rejected_bracket` rolling the domain's delta baseline
/// back. Moving the payload phase past the guard silently ended all four: the
/// join substitute alone would have been PUBLISHED, over a baseline
/// `baseline_commit.commit` may already have advanced (Codex round 1, blocking
/// 1). [`ClosedBracketPanicMarker`] is what restores it.
///
/// This drives the real helper in a real scope rather than reading the source,
/// because what has to hold is a property of the recorded bracket, not of the
/// text that records it.
///
/// THE TAG IS PAST `LANE_COST_TABLE_LEN` (512) ON PURPOSE, here and in the test
/// below. `lane_cost_hint` and `lane_cost_observe` index that table with `.get`,
/// so an out-of-range tag reads and writes nothing: these tests drive the real
/// helper without perturbing the process-global reader-gate EWMA that a real
/// domain tag would.
///
/// A SECOND BRACKET UNDER THE SAME TAG IS STAGED FIRST, and it must come back
/// unmarked. `mark_bracket_worker_panicked` marks the LAST bracket carrying the
/// tag, on the reasoning that a tag's other brackets are recorded EARLIER in the
/// frame — d47's prewarm is the real instance. With one bracket in the tracker
/// that reasoning is untested and the test claims coverage it does not have
/// (Codex round 3, non-blocking 4); with two, marking the wrong one fails here.
///
/// kills: deleting the marker; constructing it after `finish` instead of before;
/// dropping the `std::thread::panicking()` test (which would mark EVERY
/// bracket); `mark_bracket_worker_panicked` selecting the FIRST same-tag bracket
/// instead of the last, or marking every same-tag bracket.
#[test]
fn a_panic_in_the_deferred_half_marks_the_closed_bracket() {
    const TAG: u16 = 0xD53;
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    // The earlier same-tag bracket, standing in for a prewarm. Recorded before
    // the worker runs, so it is the one the marker must NOT touch.
    lanes.record_clock_bracket(TAG, None, None, false);
    let joined = thread::scope(|scope| {
        let handle = spawn_gated_finishing(
            scope,
            None,
            &lanes,
            TAG,
            || 1u32,
            |_observed: u32| -> u32 { panic!("a latent bug in the payload phase") },
        );
        handle.join()
    });
    assert!(
        joined.is_err(),
        "the panic must still propagate to the join, which is where the domain's panic \
         substitute is minted",
    );
    let brackets = lanes.take_clock_brackets();
    let ours: Vec<bool> = brackets
        .iter()
        .filter(|bracket| bracket.tag == TAG)
        .map(|bracket| bracket.worker_panicked)
        .collect();
    assert_eq!(
        ours,
        vec![false, true],
        "exactly the LAST bracket carrying the tag -- this worker's -- must carry the \
         panic; the earlier one is a different read and marking it would gap a domain \
         whose bracket was fine",
    );
}

/// And a worker that does NOT panic leaves its bracket clean.
///
/// The other half of the marker's contract, and the one a careless
/// implementation breaks: a `Drop` that marked unconditionally would gap every
/// d53 frame in every capture. Nothing else in the crate would notice until a
/// capture came back with no d53 data at all.
///
/// kills: `ClosedBracketPanicMarker::drop` marking without testing
/// `std::thread::panicking()`.
#[test]
fn a_clean_finishing_worker_leaves_its_bracket_unmarked() {
    const TAG: u16 = 0xD54;
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    let out = thread::scope(|scope| {
        spawn_gated_finishing(scope, None, &lanes, TAG, || 2u32, |observed: u32| observed * 3)
            .join()
            .expect("a clean worker joins cleanly")
    });
    assert_eq!(out, 6, "the finish half's value is what the join returns");
    let brackets = lanes.take_clock_brackets();
    let ours = brackets
        .iter()
        .find(|bracket| bracket.tag == TAG)
        .expect("the worker recorded its bracket");
    assert!(
        !ours.worker_panicked,
        "a worker that completed must not be reported as panicked; every d53 frame would \
         become a gap",
    );
    let splits = lanes.take_bracket_splits();
    assert!(
        splits.iter().any(|(tag, _, _)| *tag == TAG),
        "and the split pair must be recorded, or `lw_d53_inb_us` never reaches a capture",
    );
}

/// d53 is spawned split, and its finish half is a NAMED FREE FUNCTION.
///
/// A closure would be the easy spelling and it is the one that breaks the
/// proof: a closure at this site can capture `state` and nothing in its
/// signature would say so, which would move an engine read past the bracket
/// with no declaration to check.
/// `the_deferred_half_of_d53_carries_no_engine_handle` reads the free
/// function's signature; this is what keeps the free function the thing that
/// actually runs.
///
/// kills: reverting d53 to `spawn_gated` (which would put the payload phase back
/// in the bracket); passing a closure as `finish`.
#[test]
fn d53_is_spawned_with_the_free_finish_function() {
    let source = super::collector_source_lf();
    let at = source
        .find("COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE,\n                move || {\n                    read_compact_diplomacy_intel_operatives_live_observe(")
        .expect("d53 must spawn its observe half");
    let spawn = source[..at]
        .rfind("spawn_gated")
        .expect("the d53 spawn call must be found");
    assert!(
        source[spawn..].starts_with("spawn_gated_finishing("),
        "d53 must be spawned by `spawn_gated_finishing`; saw {}",
        &source[spawn..spawn + 40],
    );
    let tail = &source[at..];
    let finish = tail
        .find("\n                finish_compact_diplomacy_intel_operatives_live,\n")
        .expect("the finish argument must be the named free function, not a closure");
    let next_spawn = tail[1..]
        .find("spawn_gated")
        .map(|found| found + 1)
        .unwrap_or(tail.len());
    assert!(
        finish < next_spawn,
        "the finish argument must belong to d53's own spawn call, not a later one",
    );
}
