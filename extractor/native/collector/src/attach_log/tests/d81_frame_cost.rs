//! What d81 stops paying for, and what it must still refuse.
//!
//! `rail_replay_state_live` is the capture's most expensive read: t26 measured
//! 51.36 ms/frame p50 (64.0 p95), more than the whole 47.9 ms free-run
//! game-hour budget, for 186 delivered domain-hours against 177 refusals. Three
//! costs were removed here and each has a different proof obligation:
//!
//!  * the private map-control topology walk becomes lane BH's ONE walk per
//!    frame, so the obligation is that the fail-closed rule is not weakened and
//!    that a shared walk the engine REFUSED still refuses d81;
//!  * the per-(province, POV) controller read becomes one read per province per
//!    frame, so the obligation is that the read really is deduplicated, that its
//!    closing witness survives, and that nothing is carried between frames;
//!  * the confirmation copy inside `d81_read_stable_bytes` moves into a
//!    caller-owned buffer, so the obligation is that reusing that buffer cannot
//!    make a later read agree with an earlier one's bytes.
//!
//! WHAT THESE TESTS DO NOT PROVE. d81 has no end-to-end fixture: staging a
//! `CGameState` complete enough for its rail manager, active-province table,
//! anchor selector map and the whole `CSupplySystem` / `CCountrySupplySystem`
//! companion tree is a fixture nobody has built, and lane BG said so rather
//! than invent a decomposition. What the topology tests below drive is
//! therefore the REAL `read_compact_rail_replay_state` up to and including its
//! topology stage, over the staged full-size map; the read then refuses at
//! `RailRoots`, which is exactly the boundary of what the fixture stages. The
//! controller and scratch-buffer tests drive their production functions
//! directly over staged memory and are complete for what they claim.
//!
//! Offsets are not proved here, for the reason stated at the head of
//! `state_fixture.rs`: the fixture writes through the same constants the reader
//! reads through.

use super::state_fixture::{fixture_game_hour_raw, FixtureRegion};
use super::*;

// ---------------------------------------------------------------------
// Job 1 -- the shared topology build
// ---------------------------------------------------------------------

/// The staged map, plus the two build-profile bools d81's prerequisite mask
/// requires before it will reach the topology stage at all.
///
/// Staging the real sentinel BYTES would need both profile RVAs declared to the
/// fixture, whose window then spans the ~51 MB between the image head and the
/// `CGameState` singleton — on top of a map fixture that is already full-size.
/// The bools are what the prerequisite mask reads, the mask is verified
/// elsewhere, and it is the gate IN FRONT of the topology stage rather than the
/// subject of anything here.
fn d81_state(fixture: &super::state_fixture::CollectorStateFixture) -> CollectorState {
    let mut state = fixture.collector_state();
    state.daily_match = true;
    state.long_string_match = true;
    state
}

/// Drive the production d81 reader and report what its topology stage did.
///
/// Returns `(first_failure_stage, first_failure_index)`. Every call here fails
/// -- the fixture stages a map and no rail tree -- and WHICH failure is the
/// whole signal: `Topology` means the topology stage refused d81, anything
/// later means the topology stage handed it a map.
fn d81_topology_outcome(
    state: &CollectorState,
    cache: &CompactFrameMapTopologyCache,
) -> (D81FailureStage, u32) {
    let read = d81_finish_rail_replay_state(read_compact_rail_replay_state(state, 0, cache, false));
    let failure = read
        .first_failure
        .expect("the fixture stages no rail tree, so d81 always refuses");
    (failure.stage, failure.index)
}

/// d81 publishes a map another domain in the same frame already walked, and
/// pays for no walk of its own.
///
/// This is the change's entire point: seven readers wanted one traversal of
/// every province the province array names, and d81 was the seventh still doing
/// its own.
///
/// MUTATION RUN: the `SharedMapTopologyOffer::Accepted(shared)` arm in
/// `read_compact_rail_replay_state` replaced by `Err(0usize)`, so an accepted
/// share is discarded and d81 refuses. Observed in `--release`:
///
/// ```text
/// assertion `left == right` failed: an accepted share must carry d81 PAST its topology stage
///   left: Topology
///  right: RailRoots
/// ```
///
/// Restored.
#[test]
fn d81_publishes_the_frames_shared_walk_instead_of_performing_its_own() {
    page_cache_new_frame();
    let fixture = super::shared_map_topology::staged_map_fixture();
    let state = d81_state(&fixture);
    let hour = fixture_game_hour_raw();
    let lanes = super::shared_map_topology::lanes_for(&state);
    let cache = super::shared_map_topology::cache_for(&state, hour);

    // THE SHAPE OF A REAL FRAME, and it is load-bearing rather than
    // decoration. `spawn_gated` opens every worker's bracket as that worker
    // starts, and the seven map consumers run concurrently, so whichever one
    // performs the walk performs it AFTER the others' brackets are already
    // open — which is exactly what BH's ordinal rule requires before a consumer
    // may vouch for a build it did not perform. d81's bracket is therefore
    // opened FIRST here and another domain builds inside it; a build that began
    // before d81's bracket opened is `OutsideConsumerBracket` and would
    // correctly be refused.
    let _d81_bracket =
        ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE);
    {
        let _builder = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_CONTROL);
        assert!(matches!(
            cache.accepted_build(&state),
            SharedMapTopologyOffer::Accepted(_)
        ));
    }
    let after_builder = cache.counts();
    assert_eq!(
        (after_builder.builds, after_builder.builder_kept_own),
        (1, 1),
        "the other domain performed the frame's one walk"
    );

    let (stage, _) = d81_topology_outcome(&state, &cache);
    assert_eq!(
        stage,
        D81FailureStage::RailRoots,
        "an accepted share must carry d81 PAST its topology stage"
    );

    let counts = cache.counts();
    assert_eq!(
        counts.accepted, 1,
        "d81 consulted the share and took it: this is the consultation the \
         change exists to produce"
    );
    assert_eq!(
        counts.builds, 1,
        "and the frame still walked the map exactly ONCE -- d81 added no \
         second traversal"
    );
    assert_eq!((counts.rebuilt, counts.failed_through), (0, 0));
}

/// A d81 that cannot vouch for the shared build walks the map itself, exactly
/// as it did before the cache existed.
///
/// The fallback is the pre-cache line verbatim, so a refusal costs d81 its
/// saving and never its hour.
///
/// MUTATION RUN: the `SharedMapTopologyOffer::Rebuild` arm's
/// `build_compact_map_control_topology(state)` replaced by a typed
/// `Err("refused")`. Observed in `--release`:
///
/// ```text
/// assertion `left == right` failed: a refused share must leave d81 building its own map, not refusing the hour
///   left: Topology
///  right: RailRoots
/// ```
///
/// (The same mutation also killed
/// `d81_reaches_the_map_walk_only_through_the_frame_cache`, which counts the
/// call it removed.) Restored.
#[test]
fn a_d81_that_cannot_vouch_for_the_share_builds_its_own_map() {
    page_cache_new_frame();
    let fixture = super::shared_map_topology::staged_map_fixture();
    let state = d81_state(&fixture);
    let hour = fixture_game_hour_raw();
    let cache = super::shared_map_topology::cache_for(&state, hour);

    // No bracket is held on a bare test thread, so the pre-build admission
    // refuses d81 before the frame mints anything.
    let (stage, index) = d81_topology_outcome(&state, &cache);
    assert_eq!(
        stage,
        D81FailureStage::RailRoots,
        "a refused share must leave d81 building its own map, not refusing the \
         hour"
    );
    assert_eq!(index, 0);

    let counts = cache.counts();
    assert_eq!(
        (counts.rebuilt, counts.rejected_no_bracket),
        (1, 1),
        "and it is refused for the reason the rule gives, on the wire"
    );
    assert_eq!(
        counts.builds, 0,
        "a consumer that could never accept a build must not make the frame \
         mint one"
    );
}

/// A shared walk the ENGINE refused, performed by d81 itself, refuses d81 --
/// it does not buy d81 a second traversal.
///
/// This is the arm that is easy to get wrong by collapsing it into `Rebuild`.
/// `BuiltAndFailed` is handed only to the consumer whose own attempt WAS the
/// failing walk. Treating it as a rebuild would hand d81 a second traversal the
/// pre-cache code never gave it, so a transient engine refusal that used to cost
/// d81 its hour would be silently survived and d81's failure count would stop
/// meaning what it meant before the cache.
///
/// `d81_failure_index_sample` is what tells the two topology refusals apart on
/// the wire, and it is what this mutation moves.
///
/// MUTATION RUN: `SharedMapTopologyOffer::BuiltAndFailed(_) => Err(1usize)`
/// replaced by `SharedMapTopologyOffer::BuiltAndFailed(_) => match
/// build_compact_map_control_topology(state) { Ok((topology, _, _)) =>
/// Ok(topology), Err(_) => Err(0usize) }`, i.e. the arm collapsed into a
/// rebuild. Observed in `--release`:
///
/// ```text
/// assertion `left == right` failed: d81 must inherit the refusal of the walk it PERFORMED, and say so on the wire
///   left: 0
///  right: 1
/// ```
///
/// (The same mutation also killed
/// `d81_reaches_the_map_walk_only_through_the_frame_cache`, which counts the
/// SECOND traversal it introduced: left 2, right 1. That pair is the honest
/// division of labour — this test proves the wire says which refusal happened,
/// and the source test proves no second walk was performed.) Restored.
#[test]
fn a_shared_walk_d81_performed_and_the_engine_refused_refuses_d81() {
    page_cache_new_frame();
    let mut fixture = super::shared_map_topology::staged_map_fixture();
    let hour = fixture_game_hour_raw();
    // A province slot holding a non-heap pointer: the walk reaches it and
    // returns `province_pointer_invalid`. A REAL refusal of the engine walk,
    // not one injected into the cache.
    fixture
        .heap
        .write_u64(super::shared_map_topology::MAP_PROVINCE_ARRAY + 8, 0x1234);
    let state = d81_state(&fixture);
    let lanes = super::shared_map_topology::lanes_for(&state);
    let cache = super::shared_map_topology::cache_for(&state, hour);

    // d81 is the FIRST consumer, so d81 performs the frame's walk and the
    // engine's refusal is d81's own spent attempt.
    let (stage, index) = {
        let _bracket = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE);
        d81_topology_outcome(&state, &cache)
    };
    assert_eq!(stage, D81FailureStage::Topology);
    assert_eq!(
        index, 1,
        "d81 must inherit the refusal of the walk it PERFORMED, and say so on \
         the wire"
    );

    let counts = cache.counts();
    assert_eq!(
        (counts.failed_through, counts.builds),
        (1, 1),
        "one walk, spent by d81, and no second one"
    );
    assert_eq!((counts.accepted, counts.rebuilt), (0, 0));
}

// ---------------------------------------------------------------------
// Job 2 -- one controller read per province, not per (province, POV)
// ---------------------------------------------------------------------

/// One staged `CProvince` per id, far enough apart that a reader confusing two
/// of them reads a different value rather than the same one by luck.
const CONTROLLER_PROVINCE_STRIDE: usize = 0x200;
const CONTROLLER_PROVINCE_COUNT: usize = 8;

fn controller_heap() -> FixtureRegion {
    FixtureRegion::new(CONTROLLER_PROVINCE_STRIDE * (CONTROLLER_PROVINCE_COUNT + 1))
}

fn controller_slot(province_id: u32) -> usize {
    province_id as usize * CONTROLLER_PROVINCE_STRIDE
}

/// Every raw handle the tests use resolves to a canonical id that is NOT equal
/// to it, so a resolution that returned the raw handle unchanged cannot pass.
fn controller_authority() -> BTreeMap<u32, (u32, usize)> {
    (1u32..64)
        .map(|raw| (raw, (raw + 1000, 0usize)))
        .collect()
}

/// The read is paid for once per province however many POVs ask, and the
/// witness that closes it is minted once too.
///
/// Route maps overlap heavily across POVs, so this is the call-count reduction
/// the whole job is: lane BG established that a guarded call costs ~80-90 ns
/// whatever its width, so the money is in the number of calls and nowhere else.
/// Witness count is the faithful proxy for reads here -- exactly one witness is
/// pushed per guarded resolution and none is pushed on the cached path.
///
/// MUTATION RUN: the `if let Some(&resolved) = frame_map.get(&province_id)`
/// early return deleted from `d81_resolve_province_controller`, so every POV
/// reads again. Observed in `--release`:
///
/// ```text
/// assertion `left == right` failed: three POVs over the same two provinces must produce TWO reads, not six
///   left: 6
///  right: 2
/// ```
///
/// Restored.
#[test]
fn one_controller_read_serves_every_pov_that_asks_for_that_province() {
    let mut heap = controller_heap();
    heap.write_u32(controller_slot(3) + 0x188, 7);
    heap.write_u32(controller_slot(5) + 0x188, 9);
    let authority = controller_authority();

    let mut frame_map = BTreeMap::new();
    let mut witnesses = Vec::new();
    let mut answers = Vec::new();
    // Three POVs whose route maps both touch provinces 3 and 5.
    for _pov in 0..3 {
        for province_id in [3u32, 5] {
            answers.push(
                d81_resolve_province_controller(
                    heap.at(controller_slot(province_id)),
                    province_id,
                    &authority,
                    &mut frame_map,
                    &mut witnesses,
                )
                .expect("the staged controller slot is readable"),
            );
        }
    }

    assert_eq!(
        witnesses.len(),
        2,
        "three POVs over the same two provinces must produce TWO reads, not six"
    );
    assert_eq!(
        answers,
        vec![(7, 1007), (9, 1009), (7, 1007), (9, 1009), (7, 1007), (9, 1009)],
        "and every POV is handed the same answer the first one paid for, \
         resolved through the equivalence authority rather than echoed raw"
    );
    assert_eq!(
        witnesses
            .iter()
            .map(|witness| witness.address)
            .collect::<BTreeSet<_>>(),
        [
            heap.at(controller_slot(3)) + 0x188,
            heap.at(controller_slot(5)) + 0x188
        ]
        .into_iter()
        .collect::<BTreeSet<_>>(),
        "the two witnesses name the two controller slots, one each"
    );
}

/// The witness the dedup keeps is a REAL closing witness: it still refuses a
/// controller the engine changed after it was read.
///
/// The dedup's whole safety argument is that one witness spanning
/// [first read, close] contains every per-POV span it replaces. That is only
/// worth anything if the witness is still checked and still fails, so this
/// changes the staged memory under it and requires the refusal.
///
/// MUTATION RUN: the `frame_witnesses.push(...)` in
/// `d81_resolve_province_controller` replaced by `let _ = &mut
/// *frame_witnesses;`. Observed in `--release`:
///
/// ```text
/// a controller changed after the frame read it must still refuse the frame; a dedup that dropped the witness would publish it
/// ```
///
/// (It also killed `one_controller_read_serves_every_pov_that_asks_for_that_province`
/// — left 0, right 2 — and `a_controller_resolved_in_one_frame_is_never_reused_in_the_next`
/// — left 0, right 1.) Restored.
#[test]
fn the_deduplicated_controller_still_carries_a_witness_that_can_refuse() {
    let mut heap = controller_heap();
    heap.write_u32(controller_slot(3) + 0x188, 7);
    let authority = controller_authority();

    let mut frame_map = BTreeMap::new();
    let mut witnesses = Vec::new();
    d81_resolve_province_controller(
        heap.at(controller_slot(3)),
        3,
        &authority,
        &mut frame_map,
        &mut witnesses,
    )
    .expect("the staged controller slot is readable");
    assert!(
        witnesses.iter().all(|witness| witness.unchanged()),
        "nothing has moved yet"
    );

    // The engine rewrites the controller after every POV has taken the shared
    // answer. The closing gate must still see it.
    heap.write_u32(controller_slot(3) + 0x188, 11);
    assert!(
        witnesses.iter().any(|witness| !witness.unchanged()),
        "a controller changed after the frame read it must still refuse the \
         frame; a dedup that dropped the witness would publish it"
    );
}

/// Nothing survives the frame. A controller observed in one frame is never
/// published in another.
///
/// The map is a caller-owned local of the hub read, which `spawn_gated` runs
/// wholly inside one `ReaderClockBracketGuard`; a value carried across would be
/// a controller observed in one hour and published under another, which is the
/// owner's stop condition.
///
/// MUTATION RUN: `frame_map` replaced by a `thread_local!` `RefCell<BTreeMap<..>>`
/// consulted and populated in place of the parameter, so the map outlives the
/// bracket that filled it. Observed in `--release`:
///
/// ```text
/// assertion `left == right` failed: a fresh frame must READ the controller again, never reuse the last frame's
///   left: (7, 1007)
///  right: (11, 1011)
/// ```
///
/// Restored.
#[test]
fn a_controller_resolved_in_one_frame_is_never_reused_in_the_next() {
    let mut heap = controller_heap();
    heap.write_u32(controller_slot(3) + 0x188, 7);
    let authority = controller_authority();

    let mut first_frame_map = BTreeMap::new();
    let mut first_frame_witnesses = Vec::new();
    let first = d81_resolve_province_controller(
        heap.at(controller_slot(3)),
        3,
        &authority,
        &mut first_frame_map,
        &mut first_frame_witnesses,
    )
    .expect("the staged controller slot is readable");
    assert_eq!(first, (7, 1007));

    // The hour turns and the province changes hands.
    heap.write_u32(controller_slot(3) + 0x188, 11);

    let mut second_frame_map = BTreeMap::new();
    let mut second_frame_witnesses = Vec::new();
    let second = d81_resolve_province_controller(
        heap.at(controller_slot(3)),
        3,
        &authority,
        &mut second_frame_map,
        &mut second_frame_witnesses,
    )
    .expect("the staged controller slot is readable");
    assert_eq!(
        second,
        (11, 1011),
        "a fresh frame must READ the controller again, never reuse the last \
         frame's"
    );
    assert_eq!(
        second_frame_witnesses.len(),
        1,
        "and it mints its own witness for the value it actually read"
    );
}

/// A raw handle the equivalence authority does not admit refuses the frame, and
/// names the province it refused on.
///
/// The dedup moved this resolution behind a cache, so the test states that the
/// cached path is not a way around it: the FIRST POV to reach the province is
/// the one that fails, and it fails.
///
/// MUTATION RUN: the `.ok_or_else(...)` on the `related_authority` lookup in
/// `d81_resolve_province_controller` replaced by `.unwrap_or(0)`. Observed in
/// `--release`:
///
/// ```text
/// 63000 is not an admitted handle: (63000, 0)
/// ```
///
/// Restored.
#[test]
fn a_controller_handle_the_authority_does_not_admit_refuses_the_frame() {
    let mut heap = controller_heap();
    heap.write_u32(controller_slot(3) + 0x188, 63_000);
    let authority = controller_authority();

    let mut frame_map = BTreeMap::new();
    let mut witnesses = Vec::new();
    let failure = d81_resolve_province_controller(
        heap.at(controller_slot(3)),
        3,
        &authority,
        &mut frame_map,
        &mut witnesses,
    )
    .expect_err("63000 is not an admitted handle");
    assert_eq!(failure.stage, D81FailureStage::HubRoot);
    assert_eq!(failure.reason, D81FailureReason::IdentityMismatch);
    assert_eq!(failure.subject_id, 3);
    assert_eq!(failure.raw_value, 63_000);
    assert!(
        frame_map.is_empty(),
        "a resolution that refused must not be cached as an answer"
    );
}

// ---------------------------------------------------------------------
// Job 3 -- the confirmation copy stops allocating
// ---------------------------------------------------------------------

/// A reused scratch buffer cannot make a later read agree with an earlier one's
/// bytes -- neither by comparing stale content nor by returning it.
///
/// The buffer is only ever GROWN, so after a long read it is longer than a
/// following short one. Comparing the whole of it, rather than its first `len`
/// bytes, would report a perfectly coherent short read as `Torn`; and any path
/// that returned buffer content instead of the first read's would publish the
/// PREVIOUS call's bytes.
///
/// MUTATION RUN: `if first.as_slice() != &*second` in
/// `d81_read_stable_bytes_with` replaced by `if first.as_slice() !=
/// scratch.as_slice()`, i.e. compared against the whole buffer instead of its
/// first `len` bytes. Observed in `--release`:
///
/// ```text
/// the staged window is readable: D81FirstFailure { stage: HubObject, reason: Torn, subject_id: 0, related_id: 0, index: 0, raw_value: 2428889650928, anchor_province_ptr: 0, anchor_address: 0, anchor_variant: 0, read_address: 0, read_width: 0, read_attempt: 0, read_success_mask: 0, read_first_value: 0, read_second_value: 0 }
/// ```
///
/// Restored.
#[test]
fn the_shared_scratch_buffer_never_leaks_one_read_into_the_next() {
    let mut heap = FixtureRegion::new(0x400);
    heap.write_bytes(0x00, &[0xAAu8; 0x40]);
    heap.write_bytes(0x80, &[0xBBu8; 0x40]);

    let mut scratch = Vec::new();
    // Long first, short second: the short read must be compared against the
    // first `len` bytes of the buffer and nothing else.
    let long = d81_read_stable_bytes(heap.at(0x00), 0x40, D81FailureStage::HubObject, &mut scratch)
        .expect("the staged window is readable");
    assert_eq!(long, vec![0xAAu8; 0x40]);

    let short = d81_read_stable_bytes(heap.at(0x80), 0x10, D81FailureStage::HubObject, &mut scratch)
        .expect("the staged window is readable");
    assert_eq!(
        short,
        vec![0xBBu8; 0x10],
        "the second read must report the bytes at ITS address, not the bytes \
         the buffer still held from the first"
    );

    // And growing again afterwards reads the full new length rather than
    // serving the tail the shorter read left behind.
    let long_again =
        d81_read_stable_bytes(heap.at(0x80), 0x40, D81FailureStage::HubObject, &mut scratch)
            .expect("the staged window is readable");
    assert_eq!(long_again, vec![0xBBu8; 0x40]);
}

/// The confirmation read is still a READ, and a window that moved between the
/// two copies is still `Torn`.
///
/// Moving the confirmation copy into a caller-owned buffer removed an
/// ALLOCATION, not a check. An offline fixture cannot schedule an engine write
/// into the gap between the two copies, so the tear is injected -- the same
/// answer `d81_read_u64_pair_with` and `d81_read_coherent_active_entry_with`
/// already give to the same problem in this domain.
///
/// MUTATION RUN: the `if first.as_slice() != &*second { return Err(..Torn..) }`
/// block in `d81_read_stable_bytes_with` replaced by `let _ = &*second;`.
/// Observed in `--release`:
///
/// ```text
/// a window that moved between the two copies is torn: [17, 17, 17, 17, 17, 17, 17, 17]
/// ```
///
/// Restored.
#[test]
fn a_window_that_moved_between_the_two_copies_is_still_torn() {
    let mut scratch = Vec::new();
    let failure = d81_read_stable_bytes_with(
        0x4000,
        8,
        D81FailureStage::HubObject,
        &mut scratch,
        |_, len| Some(vec![0x11u8; len]),
        // The engine rewrote the window between the two copies.
        |_, buf| {
            buf.fill(0x22);
            true
        },
    )
    .expect_err("a window that moved between the two copies is torn");
    assert_eq!(failure.reason, D81FailureReason::Torn);
    assert_eq!(failure.stage, D81FailureStage::HubObject);
    assert_eq!(failure.raw_value, 0x4000);
}

/// A confirmation read that FAILS is a refusal, not a pass -- and the buffer it
/// may have half-written is never inspected.
///
/// `read_into` documents its buffer as unspecified when it returns `false`, so
/// the refusal has to come before the comparison. A confirmation that wrote
/// bytes which WOULD compare equal and then failed is the exact shape that
/// slips past an implementation comparing first and checking the flag after.
///
/// MUTATION RUN: `if !confirm(address, second) { return Err(..) }` in
/// `d81_read_stable_bytes_with` replaced by `let _ = confirm(address, second);`.
/// Observed in `--release`:
///
/// ```text
/// a failed confirmation refuses: [17, 17, 17, 17, 17, 17, 17, 17]
/// ```
///
/// Restored.
#[test]
fn a_confirmation_read_that_failed_refuses_before_anything_is_compared() {
    let mut scratch = Vec::new();
    let failure = d81_read_stable_bytes_with(
        0x4000,
        8,
        D81FailureStage::HubObject,
        &mut scratch,
        |_, len| Some(vec![0x11u8; len]),
        |_, buf| {
            buf.fill(0x11);
            false
        },
    )
    .expect_err("a failed confirmation refuses");
    assert_eq!(failure.reason, D81FailureReason::Unreadable);
    assert_eq!(failure.index, 1, "and it is the SECOND read that failed");
}

/// A first read that fails refuses without touching the shared buffer at all.
///
/// The buffer is shared across every call in the hub read, so the arm that
/// returns earliest is the one where a stale byte would be cheapest to leave
/// behind. It never grows, is never written, and the failure is the one the
/// two-`read_bytes` form gave: `Unreadable` with the address as its witness and
/// index 0, not 1.
///
/// MUTATION RUN: the `if scratch.len() < len { scratch.resize(len, 0); }` block
/// in `d81_read_stable_bytes_with` moved ABOVE the first read, so the buffer is
/// staged before the read that may refuse. Observed in `--release`:
///
/// ```text
/// assertion `left == right` failed: a refused first read must not grow or write the shared buffer
///   left: [204, 204, 204, 204, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
///  right: [204, 204, 204, 204]
/// ```
///
/// Restored.
#[test]
fn a_first_read_that_failed_refuses_without_touching_the_buffer() {
    let mut scratch = vec![0xCCu8; 4];
    let before = scratch.clone();
    let failure = d81_read_stable_bytes_with(
        0x4000,
        64,
        D81FailureStage::HubRoot,
        &mut scratch,
        |_, _| None,
        |_, _| panic!("the confirmation must not run once the first read failed"),
    )
    .expect_err("an unreadable first read refuses");
    assert_eq!(failure.reason, D81FailureReason::Unreadable);
    assert_eq!(failure.stage, D81FailureStage::HubRoot);
    assert_eq!(failure.raw_value, 0x4000);
    assert_eq!(
        failure.index, 0,
        "index 0 is the FIRST read; index 1 is reserved for the confirmation"
    );
    assert_eq!(
        scratch, before,
        "a refused first read must not grow or write the shared buffer"
    );
}

/// A zero-length window is refused by the first read, so the buffer arithmetic
/// is never reached with `len == 0`.
///
/// `read_bytes` declines a zero-length range, which is what production supplies,
/// and the helper must not paper over that with an empty-slice comparison that
/// trivially succeeds.
///
/// NO MUTATION RUN, and this is a CHARACTERISATION test rather than a proven
/// one: the behaviour it pins belongs to `read_bytes`, which no mutation of the
/// code this lane touched can change. It exists because the review asked for the
/// `len == 0` arm to be stated, and it fails the day `read_bytes` starts
/// accepting an empty range and the buffer arithmetic below it is reached with a
/// zero-length slice.
#[test]
fn a_zero_length_window_is_refused_by_the_first_read() {
    let mut scratch = Vec::new();
    let failure = d81_read_stable_bytes(0x4000, 0, D81FailureStage::HubRoot, &mut scratch)
        .expect_err("production's `read_bytes` declines a zero-length range");
    assert_eq!(failure.reason, D81FailureReason::Unreadable);
    assert_eq!(failure.index, 0);
    assert!(
        scratch.is_empty(),
        "and nothing was staged in the shared buffer for it"
    );
}

/// The comparison is against the first `len` bytes of the shared buffer and
/// never against the whole of it.
///
/// The buffer is only ever GROWN, so a long read leaves it longer than a
/// following short one. Comparing the whole buffer would report a perfectly
/// coherent short read as `Torn` -- a refusal manufactured by the optimisation
/// itself, on a frame the pre-change code published.
///
/// MUTATION RUN: `if first.as_slice() != &*second` in
/// `d81_read_stable_bytes_with` replaced by `if first.as_slice() !=
/// scratch.as_slice()`. Observed in `--release`:
///
/// ```text
/// a stable short window is accepted even though the buffer is longer: D81FirstFailure { stage: HubObject, reason: Torn, subject_id: 0, related_id: 0, index: 0, raw_value: 20480, anchor_province_ptr: 0, anchor_address: 0, anchor_variant: 0, read_address: 0, read_width: 0, read_attempt: 0, read_success_mask: 0, read_first_value: 0, read_second_value: 0 }
/// ```
///
/// Restored.
#[test]
fn a_short_read_is_compared_against_its_own_length_not_the_whole_buffer() {
    let mut scratch = Vec::new();
    let long = d81_read_stable_bytes_with(
        0x4000,
        0x40,
        D81FailureStage::HubObject,
        &mut scratch,
        |_, len| Some(vec![0xAAu8; len]),
        |_, buf| {
            buf.fill(0xAA);
            true
        },
    )
    .expect("a stable long window is accepted");
    assert_eq!(long, vec![0xAAu8; 0x40]);
    assert!(
        scratch.len() >= 0x40,
        "the buffer is now longer than the short read that follows"
    );

    let short = d81_read_stable_bytes_with(
        0x5000,
        0x10,
        D81FailureStage::HubObject,
        &mut scratch,
        |_, len| Some(vec![0xBBu8; len]),
        |_, buf| {
            buf.fill(0xBB);
            true
        },
    )
    .expect("a stable short window is accepted even though the buffer is longer");
    assert_eq!(
        short,
        vec![0xBBu8; 0x10],
        "the short read reports the bytes at ITS address; the 0xAA tail the          previous call left in the buffer takes no part in the comparison"
    );
}

// ---------------------------------------------------------------------
// The structure the wire rule depends on
// ---------------------------------------------------------------------

/// d81 consults the frame cache and never calls the map walk except from the
/// `Rebuild` arm.
///
/// Asserted against the source because the alternative -- a d81 that consults
/// the cache AND then walks the map anyway -- publishes exactly the same rows
/// and is invisible to every behavioural test above. It is only visible as an
/// extra traversal, and the number of traversals a frame performs is what the
/// whole change is about.
#[test]
fn d81_reaches_the_map_walk_only_through_the_frame_cache() {
    let source = collector_source_lf();
    let reader = source
        .split_once("fn read_compact_rail_replay_state(")
        .expect("the d81 reader")
        .1
        .split_once("\nfn ")
        .expect("the end of the d81 reader")
        .0;
    assert_eq!(
        reader.matches("build_compact_map_control_topology(state)").count(),
        1,
        "d81 must reach the map walk from ONE place, the cache's Rebuild arm; \
         a second call is a second traversal of every province in the game"
    );
    assert!(
        reader.contains("topology_cache.accepted_build(state)"),
        "and it must reach it THROUGH the frame cache, or it is back to its \
         own private walk"
    );
    assert!(
        reader.contains("SharedMapTopologyOffer::BuiltAndFailed(_) => Err(1usize)"),
        "a walk d81 performed and the engine refused must fail d81, not fall \
         through to a second traversal the pre-cache code never gave it"
    );
}

/// The deduplicated controller witness is re-read LAST, after every other
/// witness in the hub's closing gate.
///
/// This is the property that makes ONE check stand in for the per-POV checks it
/// replaces. The closing gate is a SEQUENCE of re-reads over real time, not one
/// instant, so a single witness checked EARLY in that sequence catches strictly
/// fewer engine writes than the per-POV witnesses it replaced, the last of which
/// was re-read late in it (Codex review 019fd07c-b759-7aa3-8782-7bfb3a7a7aec).
///
/// Asserted against the source, and it is the honest form of the claim: the
/// defect is a REORDERING of guarded re-reads inside a single expression, and no
/// offline fixture can schedule an engine write between two of them. `||`
/// evaluates left to right, so the position of the disjunct IS the check order.
///
/// MUTATION RUN: the `controller_witnesses` disjunct moved to sit BEFORE
/// `final_witnesses` in the closing gate — which is the shape the review found
/// and this commit fixed. Observed in `--release`:
///
/// ```text
/// the controller witnesses must be re-read AFTER `final_witnesses`, or the one check lands earlier than the per-POV checks it replaces and catches strictly fewer engine writes
/// ```
///
/// Restored.
#[test]
fn the_controller_witnesses_are_the_last_thing_the_hub_closing_gate_re_reads() {
    let source = collector_source_lf();
    let hub = source
        .split_once("fn read_compact_rail_hub_simulation(")
        .expect("the d81 hub read")
        .1
        .split_once("\nfn ")
        .expect("the end of the d81 hub read")
        .0;
    let gate = hub
        .rsplit_once("|| controller_witnesses")
        .expect(
            "the closing gate must re-read the deduplicated controller \
             witnesses; without that disjunct the dedup has no closing check \
             at all",
        );
    assert!(
        gate.0.contains("|| final_witnesses.iter().any(|witness| !witness.unchanged())"),
        "the controller witnesses must be re-read AFTER `final_witnesses`, or \
         the one check lands earlier than the per-POV checks it replaces and \
         catches strictly fewer engine writes"
    );
    assert!(
        gate.0.contains("|| final_fallback_gates"),
        "and after the fallback gates, so it is the last re-read in the gate"
    );
    assert!(
        gate.1.contains("Ok(closure_refused)"),
        "and a controller that moved must reach the gate's returned VERDICT; a \
         gate whose result is computed and dropped refuses nothing"
    );
    // THE REFUSAL ITSELF MOVED (lane EK, 2026-08-07). The gate is a sequence of
    // guarded RE-READS and stays inside the bracket; the `ClosureChanged` it
    // raises is now raised by the post-bracket compute, which ORs this verdict
    // with the sort's strictness check the way the one `||` chain used to. Both
    // halves are pinned, because a verdict nothing consumes is the same defect
    // as a re-read nothing compares.
    let compute = source
        .split_once("fn d81_compute_rail_hub_rows(")
        .expect("the d81 post-bracket hub compute")
        .1
        .split_once("\nfn ")
        .expect("the end of the compute")
        .0;
    assert!(
        compute.contains("|| observation.reread_refused.unwrap_or(false)")
            && compute.contains("return Err(failure(D81FailureReason::ClosureChanged));"),
        "and the compute must consume the gate's verdict and refuse the frame \
         on it"
    );
    assert_eq!(
        hub.matches("&mut controller_witnesses").count(),
        1,
        "exactly one site mints controller witnesses, so there is one vector \
         to keep last"
    );
    assert!(
        !hub.contains("&mut final_witnesses,\n                )?;"),
        "the controller resolution must not push into `final_witnesses`, which \
         is re-read before the fallback gates"
    );
}

/// The production reader really does go through the injectable helper the
/// buffer tests drive.
///
/// The `_with` tests above prove `d81_read_stable_bytes_with`. They would keep
/// passing if `d81_read_stable_bytes` stopped delegating to it and grew a second
/// copy of the logic, which is the one way those tests could go quietly vacuous.
///
/// MUTATION RUN: the delegation replaced by an inlined two-`read_bytes` body
/// that ignores the scratch entirely. Observed in `--release`:
///
/// ```text
/// production must be the injectable helper applied to the real primitives, or the tests that drive the helper prove nothing about what the collector runs
/// ```
///
/// Restored.
#[test]
fn the_production_stable_read_delegates_to_the_helper_the_tests_drive() {
    let source = collector_source_lf();
    let body = source
        .split_once("fn d81_read_stable_bytes(")
        .expect("the production stable read")
        .1
        .split_once("\n}")
        .expect("the end of its body")
        .0;
    assert!(
        body.contains(
            "d81_read_stable_bytes_with(address, len, stage, scratch, read_bytes, read_into)"
        ),
        "production must be the injectable helper applied to the real \
         primitives, or the tests that drive the helper prove nothing about \
         what the collector runs"
    );
}

// ---------------------------------------------------------------------
// What `d81_phase_topology_us` should read at t27
// ---------------------------------------------------------------------

/// Price d81's topology stage on both sides of the change, so the number the
/// next capture publishes is PREDICTED rather than merely hoped for.
///
/// `d81_phase_topology_us` (lane BG) times exactly the span this commit
/// replaced. t26 predates that unit, so there is no before-value on the wire and
/// the before-value has to come from somewhere: this runs the two paths over the
/// staged full-size map (10,001 provinces, 1,200 states) and reports both.
///
/// MEASURED, 9 interleaved trials, `--release`:
///
/// ```text
/// d81 topology phase, staged map of 10,001 provinces / 1,200 states
///   REBUILD  (the pre-change line): 389 us median
///   ACCEPTED (share + clone):       35 us median
///   raw rebuild:  [298, 310, 321, 356, 389, 548, 552, 564, 594]
///   raw accepted: [30, 31, 31, 32, 35, 37, 38, 40, 43]
/// ```
///
/// -91% on the phase, and the surviving 35 us is almost entirely the `Vec`
/// clone rather than any guarded read.
///
/// AND THE UNCOMFORTABLE READING, which is the one worth keeping: 389 us is
/// 0.76% of d81's 51.36 ms t26 frame. Even allowing several times that live for
/// the region-lookup cost this fixture flatters, the map walk is NOT where d81's
/// 51 ms lives. This phase was the cut that could be made; it is not the cut
/// that explains the domain. `d81_phase_hub_us` at t27 is where to look next.
///
/// THE TWO ARMS:
///  * REBUILD -- `build_compact_map_control_topology`, the line d81 ran before,
///    and the line it still runs whenever it cannot vouch for the share;
///  * ACCEPTED -- `accepted_build` returning a share d81 may publish, plus the
///    `topology.clone()` that hands it over. The clone is deliberately inside
///    the measured span because it is inside the timed phase in production, and
///    it is the entire residual cost of the accepted arm.
///
/// WHERE IT UNDERSTATES THE PRODUCTION SAVING: the fixture's map is ONE
/// allocation, so `range_confirmed_good` resolves on its MRU fast path on
/// essentially every call; production chases province pointers across many
/// VirtualQuery regions, so the per-call component -- and therefore the walk
/// this removes -- costs MORE live. The live map is also larger than 10,001
/// provinces.
///
/// WHERE IT OVERSTATES: it runs alone, on a warm map, with no other reader lane
/// competing for L3.
///
/// WHAT IT CANNOT SAY: which arm a given live frame takes. Seven consumers race
/// for one `OnceLock`, so d81 performs the walk on some frames and shares it on
/// others; the frame's own `lw_map_topo_*` probe is what reports that split.
///
/// `#[ignore]` on purpose -- a measurement, never a wall-clock gate:
///
/// ```text
/// cargo test -p collector --release d81_topology_phase_cost_model -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn d81_topology_phase_cost_model() {
    const TRIALS: usize = 9;

    page_cache_new_frame();
    let fixture = super::shared_map_topology::staged_map_fixture();
    let state = d81_state(&fixture);
    let hour = fixture_game_hour_raw();
    let lanes = super::shared_map_topology::lanes_for(&state);

    let mut rebuild_us = Vec::new();
    let mut accepted_us = Vec::new();
    // Arms interleaved, so a drift in machine state hits both equally.
    for _ in 0..TRIALS {
        let mark = Instant::now();
        let built = build_compact_map_control_topology(&state).expect("the staged map builds");
        rebuild_us.push(mark.elapsed().as_micros() as u64);
        assert_eq!(built.0.provinces.len(), 10_000);

        // A FRESH cache each trial: one walk per frame is the whole point, so a
        // cache reused across trials would time the `OnceLock` and nothing else.
        let cache = super::shared_map_topology::cache_for(&state, hour);
        let outer =
            ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE);
        {
            let _builder = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_CONTROL);
            assert!(matches!(
                cache.accepted_build(&state),
                SharedMapTopologyOffer::Accepted(_)
            ));
        }
        let mark = Instant::now();
        let SharedMapTopologyOffer::Accepted(shared) = cache.accepted_build(&state) else {
            panic!("the share must be accepted for the accepted arm to be timed");
        };
        let shared = shared.topology.clone();
        accepted_us.push(mark.elapsed().as_micros() as u64);
        assert_eq!(shared.provinces.len(), 10_000);
        drop(outer);
    }

    rebuild_us.sort_unstable();
    accepted_us.sort_unstable();
    let median = |v: &Vec<u64>| v[v.len() / 2];
    println!("d81 topology phase, staged map of 10,001 provinces / 1,200 states");
    println!(
        "  REBUILD  (the pre-change line): {} us median",
        median(&rebuild_us)
    );
    println!(
        "  ACCEPTED (share + clone):       {} us median",
        median(&accepted_us)
    );
    println!("  raw rebuild:  {rebuild_us:?}");
    println!("  raw accepted: {accepted_us:?}");
}

// ---------------------------------------------------------------------
// Job 4 -- the POV loop stops re-reading the roster loop's two windows
// ---------------------------------------------------------------------
//
// The hub read walks the country roster to authenticate every
// `(CCountry, CCountrySupplySystem)` pair, then walks the authenticated pairs
// again as POVs. The second walk used to RE-READ `[country, country+0x0C)` and
// `[country_system, country_system+0x88)` -- the identical addresses, the
// identical lengths -- and re-check the same fields against the same expected
// values. Four guarded calls per country per frame for bytes already in hand.
//
// The obligation the elision takes on is that every one of those checks still
// runs and still refuses, so what the tests below drive is the production gate
// `d81_admit_pov_country_windows` over windows that are WRONG in each of the
// ways a mis-served cache entry could make them wrong.

const POV_COUNTRY_VTABLE: usize = 0x7ff7_0000_1000;
const POV_CCSS_VTABLE0: usize = 0x7ff7_0000_2000;
const POV_CCSS_VTABLE1: usize = 0x7ff7_0000_3000;
const POV_SUPPLY_SYSTEM: usize = 0x0000_1000_0000;
const POV_COUNTRY: usize = 0x0000_2000_0000;
const POV_COUNTRY_SYSTEM: usize = 0x0000_3000_0000;
const POV_COMPANION: usize = 0x0000_4000_0000;
const POV_RAW_HANDLE: u32 = 37;
/// Deliberately NOT equal to the raw handle, so a gate that compared the wrong
/// pair of identities cannot pass by coincidence.
const POV_CANONICAL: u32 = 12;

fn put_u64(window: &mut [u8], offset: usize, value: usize) {
    window[offset..offset + 8].copy_from_slice(&(value as u64).to_le_bytes());
}

fn pov_authority() -> D81CountryAuthority {
    let mut country_window = vec![0u8; D81_COUNTRY_IDENTITY_WINDOW_LEN];
    put_u64(&mut country_window, 0, POV_COUNTRY_VTABLE);
    country_window[OBJ_TAG_DB_ID..OBJ_TAG_DB_ID + 4]
        .copy_from_slice(&POV_RAW_HANDLE.to_le_bytes());

    let mut country_system_window = vec![0u8; D81_COUNTRY_SYSTEM_WINDOW_LEN];
    put_u64(&mut country_system_window, 0, POV_CCSS_VTABLE0);
    put_u64(&mut country_system_window, 8, POV_CCSS_VTABLE1);
    put_u64(
        &mut country_system_window,
        CCOUNTRY_SUPPLY_SYSTEM_OWNER_CSUPPLY,
        POV_SUPPLY_SYSTEM,
    );
    put_u64(
        &mut country_system_window,
        CCOUNTRY_SUPPLY_SYSTEM_OWNER_COUNTRY,
        POV_COUNTRY,
    );
    put_u64(
        &mut country_system_window,
        CCOUNTRY_SUPPLY_SYSTEM_OWNER_COMPANION,
        POV_COMPANION,
    );

    D81CountryAuthority {
        country: POV_COUNTRY,
        country_system: POV_COUNTRY_SYSTEM,
        companion: POV_COMPANION,
        raw_handle: POV_RAW_HANDLE,
        country_window,
        country_system_window,
    }
}

fn admit_pov(authority: &D81CountryAuthority) -> Result<(), D81FirstFailure> {
    d81_admit_pov_country_windows(
        POV_CANONICAL,
        authority,
        POV_COUNTRY_VTABLE,
        POV_CCSS_VTABLE0,
        POV_CCSS_VTABLE1,
        POV_SUPPLY_SYSTEM,
        Some(POV_CANONICAL),
    )
}

/// The bytes the roster loop authenticated are admitted unchanged.
///
/// This is the arm the elision has to keep working: every frame, for every
/// country, the POV loop is now handed the roster loop's windows and must
/// accept them. A gate that refused here would cost d81 every hour.
#[test]
fn the_roster_loops_own_country_windows_are_admitted_by_the_pov_gate() {
    admit_pov(&pov_authority())
        .expect("the windows the roster loop authenticated must be admitted");
}

/// Every field the POV gate checks still refuses when the window carries the
/// wrong value -- with the same reason, subject and raw witness it carried when
/// the POV loop read the window itself.
///
/// THIS IS THE ELISION'S VALUE TEST. The pre-elision code obtained these bytes
/// from its own guarded read; it now obtains them from a keyed lookup, and the
/// failure a keyed lookup invites -- handing back ANOTHER entry's window -- is
/// exactly what these arms catch. Each case below is a byte another country's
/// window would plausibly hold.
///
/// MUTATION RUN: the `!= Some(authority.country)` disjunct (the `OWNER_COUNTRY`
/// tie) deleted from `d81_admit_pov_country_windows`, i.e. the bytes are no
/// longer required to belong to THIS country's supply system. Observed in
/// `--release`:
///
/// ```text
/// a supply-system window belonging to another country: ()
/// ```
///
/// Restored.
#[test]
fn a_country_window_that_is_not_this_countrys_refuses_field_by_field() {
    type Corrupt = Box<dyn Fn(&mut D81CountryAuthority)>;
    let cases: Vec<(&str, Corrupt, D81FailureReason, u64)> = vec![
        (
            "a CCountry window whose vtable is not the country vtable",
            Box::new(|a| put_u64(&mut a.country_window, 0, 0xdead_0000)),
            D81FailureReason::IdentityMismatch,
            u64::from(POV_RAW_HANDLE),
        ),
        (
            "a CCountry window bearing another country's tag handle",
            Box::new(|a| {
                a.country_window[OBJ_TAG_DB_ID..OBJ_TAG_DB_ID + 4]
                    .copy_from_slice(&(POV_RAW_HANDLE + 1).to_le_bytes())
            }),
            D81FailureReason::IdentityMismatch,
            u64::from(POV_RAW_HANDLE),
        ),
        (
            "a CCountrySupplySystem window whose first vtable is wrong",
            Box::new(|a| put_u64(&mut a.country_system_window, 0, 0xdead_0000)),
            D81FailureReason::VtableMismatch,
            0,
        ),
        (
            "a CCountrySupplySystem window whose second vtable is wrong",
            Box::new(|a| put_u64(&mut a.country_system_window, 8, 0xdead_0000)),
            D81FailureReason::VtableMismatch,
            0,
        ),
        (
            "a supply-system window owned by a different CSupplySystem",
            Box::new(|a| {
                put_u64(
                    &mut a.country_system_window,
                    CCOUNTRY_SUPPLY_SYSTEM_OWNER_CSUPPLY,
                    POV_SUPPLY_SYSTEM + 0x1000,
                )
            }),
            D81FailureReason::VtableMismatch,
            0,
        ),
        (
            "a supply-system window belonging to another country",
            Box::new(|a| {
                put_u64(
                    &mut a.country_system_window,
                    CCOUNTRY_SUPPLY_SYSTEM_OWNER_COUNTRY,
                    POV_COUNTRY + 0x1000,
                )
            }),
            D81FailureReason::VtableMismatch,
            0,
        ),
        (
            "a supply-system window naming another country's companion",
            Box::new(|a| {
                put_u64(
                    &mut a.country_system_window,
                    CCOUNTRY_SUPPLY_SYSTEM_OWNER_COMPANION,
                    POV_COMPANION + 0x1000,
                )
            }),
            D81FailureReason::IdentityMismatch,
            u64::from(POV_RAW_HANDLE),
        ),
    ];

    for (what, corrupt, reason, raw_value) in cases {
        let mut authority = pov_authority();
        corrupt(&mut authority);
        let failure = admit_pov(&authority).expect_err(what);
        assert_eq!(failure.stage, D81FailureStage::HubRoot, "{what}");
        assert_eq!(failure.reason, reason, "{what}");
        assert_eq!(failure.subject_id, POV_CANONICAL, "{what}");
        assert_eq!(failure.raw_value, raw_value, "{what}");
    }
}

/// A whole window served from the wrong country -- the exact shape of a keyed
/// lookup returning the neighbouring entry -- refuses rather than publishing one
/// country's supply topology under another's POV.
#[test]
fn a_pov_served_another_countrys_windows_entirely_refuses() {
    let mut authority = pov_authority();
    let mut other = pov_authority();
    put_u64(
        &mut other.country_system_window,
        CCOUNTRY_SUPPLY_SYSTEM_OWNER_COUNTRY,
        POV_COUNTRY + 0x8000,
    );
    other.country_window[OBJ_TAG_DB_ID..OBJ_TAG_DB_ID + 4]
        .copy_from_slice(&(POV_RAW_HANDLE + 9).to_le_bytes());
    // This POV's own identity, the neighbour's bytes.
    authority.country_window = other.country_window;
    authority.country_system_window = other.country_system_window;
    let failure = admit_pov(&authority)
        .expect_err("a POV handed another country's windows must refuse the frame");
    assert_eq!(failure.reason, D81FailureReason::IdentityMismatch);
    assert_eq!(failure.subject_id, POV_CANONICAL);
}

/// A tag handle the equivalence snapshot no longer resolves to this POV refuses,
/// exactly as it did when the POV loop performed its own read.
#[test]
fn a_pov_whose_handle_no_longer_resolves_to_it_refuses() {
    let authority = pov_authority();
    let failure = d81_admit_pov_country_windows(
        POV_CANONICAL,
        &authority,
        POV_COUNTRY_VTABLE,
        POV_CCSS_VTABLE0,
        POV_CCSS_VTABLE1,
        POV_SUPPLY_SYSTEM,
        // The equivalence snapshot now maps this raw handle somewhere else.
        Some(POV_CANONICAL + 1),
    )
    .expect_err("a handle that no longer resolves to this POV must refuse");
    assert_eq!(failure.stage, D81FailureStage::HubRoot);
    assert_eq!(failure.reason, D81FailureReason::IdentityMismatch);
    assert_eq!(failure.subject_id, POV_CANONICAL);
    assert_eq!(failure.raw_value, u64::from(POV_RAW_HANDLE));
}

/// A window SHORTER than the fields the gate reads refuses rather than passing
/// on absent bytes.
///
/// `read_u64_from_slice` returns `None` past the end, and `None != Some(x)` is
/// true, so the arms refuse -- but that is a property worth pinning rather than
/// assuming, because the retained windows are now sized at a different call site
/// than the one that consumes them.
#[test]
fn a_truncated_retained_window_refuses_instead_of_reading_absent_bytes() {
    let mut authority = pov_authority();
    authority
        .country_system_window
        .truncate(CCOUNTRY_SUPPLY_SYSTEM_OWNER_COMPANION);
    let failure = admit_pov(&authority).expect_err("a truncated window cannot be admitted");
    assert_eq!(failure.reason, D81FailureReason::IdentityMismatch);

    let mut authority = pov_authority();
    authority.country_window.truncate(OBJ_TAG_DB_ID);
    let failure = admit_pov(&authority).expect_err("a truncated window cannot be admitted");
    assert_eq!(failure.reason, D81FailureReason::IdentityMismatch);
}

/// A witness minted from the ROSTER read still refuses a change that lands after
/// it -- which is the whole safety argument for serving the POV loop from those
/// bytes.
///
/// The elision moves only the START of the span each witness vouches for,
/// earlier; the re-read stays where it was in the closing gate. This drives the
/// real `d81_read_stable_bytes` over staged memory, mints the witness the POV
/// loop mints, and then changes the memory under it.
#[test]
fn a_witness_minted_from_the_roster_read_still_refuses_a_later_change() {
    let mut heap = FixtureRegion::new(0x200);
    heap.write_u64(0x00, 0x1234_5678);
    let mut scratch = Vec::new();
    let window = d81_read_stable_bytes(
        heap.at(0x00),
        D81_COUNTRY_IDENTITY_WINDOW_LEN,
        D81FailureStage::HubRoot,
        &mut scratch,
    )
    .expect("the staged window is readable");
    let witness = D81BytesWitness::new(heap.at(0x00), window);
    assert!(witness.unchanged(), "nothing has moved yet");

    heap.write_u64(0x00, 0x9999_9999);
    assert!(
        !witness.unchanged(),
        "a window changed after the roster loop read it must still refuse the \
         frame; serving the POV loop from those bytes is only sound because \
         this check survives"
    );
}

/// The POV loop reads NEITHER window again, and still closes both.
///
/// Asserted against the source because a POV loop that consults the retained
/// windows AND re-reads them anyway publishes exactly the same rows and is
/// invisible to every behavioural test above. It is only visible as guarded
/// calls, and the number of guarded calls is what this change is about.
///
/// MUTATION RUN: the two `D81BytesWitness::new(...)` pushes in the POV loop
/// replaced by `let mut pov_witnesses: Vec<D81BytesWitness> = Vec::new();`, so
/// the retained windows are never closed. Observed in `--release`:
///
/// ```text
/// the POV loop must still mint the CCountry witness, from the roster bytes; without it the elided read has no closing check at all
/// ```
///
/// Restored.
#[test]
fn the_pov_loop_never_re_reads_the_windows_the_roster_loop_retained() {
    let source = collector_source_lf();
    let hub = source
        .split_once("fn read_compact_rail_hub_simulation(")
        .expect("the d81 hub read")
        .1
        .split_once("\nfn ")
        .expect("the end of the d81 hub read")
        .0;
    let (roster, pov) = hub
        .split_once("for (&pov_country_id, authority) in &country_authority {")
        .expect("the POV loop");

    assert!(
        roster.contains("D81_COUNTRY_IDENTITY_WINDOW_LEN"),
        "the roster loop is where the CCountry identity window is read"
    );
    assert!(
        roster.contains("D81_COUNTRY_SYSTEM_WINDOW_LEN"),
        "and where the CCountrySupplySystem window is read"
    );
    assert!(
        !pov.contains("D81_COUNTRY_IDENTITY_WINDOW_LEN"),
        "the POV loop must not read the CCountry window a second time; the \
         roster loop already read that exact address and length"
    );
    assert!(
        !pov.contains("D81_COUNTRY_SYSTEM_WINDOW_LEN"),
        "and must not read the CCountrySupplySystem window a second time"
    );
    assert!(
        pov.contains("D81BytesWitness::new(country, authority.country_window.clone())"),
        "the POV loop must still mint the CCountry witness, from the roster \
         bytes; without it the elided read has no closing check at all"
    );
    assert!(
        pov.contains(
            "D81BytesWitness::new(country_system, authority.country_system_window.clone())"
        ),
        "and the CCountrySupplySystem witness likewise"
    );
    assert!(
        pov.contains("final_witnesses.extend(pov_witnesses);"),
        "and both must reach the closing gate, which is where the span they \
         vouch for ends"
    );
    assert!(
        pov.contains("d81_admit_pov_country_windows("),
        "and every check the POV loop ran against its own read must still run \
         against the retained bytes"
    );
}

// ---------------------------------------------------------------------
// Job 5 -- the province-array slot the walk already read
// ---------------------------------------------------------------------
//
// `d81_resolve_anchor_selector_key` computes `province_array + selector_key*8`
// and reads it. The walk that calls it has ALREADY read
// `province_array + province_id*8` and compared it against the topology
// pointer, a few hundred nanoseconds earlier in the same bracket -- and those
// two addresses are the SAME whenever the engine's selector key is the walking
// province's own id. So is the CLOSING re-read: the walk closed the selector
// slot and then closed the source slot, twice over one address.

const SELECTOR_CPROVINCE_VTABLE: usize = 0x7ff7_99b1_8170;
const SELECTOR_PROVINCE_ID: u32 = 260;

/// A staged province array whose live slot for `SELECTOR_PROVINCE_ID` holds a
/// DIFFERENT pointer than the topology recorded, so a resolve that reads the
/// slot must refuse and a resolve that is served the caller's value must not.
fn selector_fixture() -> (FixtureRegion, FixtureRegion, FixtureRegion) {
    let mut resolved = FixtureRegion::new(0xA8);
    resolved.write_u64(0, SELECTOR_CPROVINCE_VTABLE as u64);
    resolved.write_u32(CPROVINCE_OFF_ID, SELECTOR_PROVINCE_ID);
    let replacement = FixtureRegion::new(0xA8);
    let mut province_array = FixtureRegion::new(0x828);
    province_array.write_u64(SELECTOR_PROVINCE_ID as usize * 8, replacement.addr() as u64);
    (resolved, replacement, province_array)
}

fn selector_topology(
    resolved: &FixtureRegion,
    province_array: &FixtureRegion,
) -> CompactMapControlTopology {
    CompactMapControlTopology {
        province_array: province_array.addr(),
        province_count: SELECTOR_PROVINCE_ID + 1,
        states: Vec::new(),
        provinces: vec![CompactMapProvinceRoot {
            province_id: SELECTOR_PROVINCE_ID,
            province_ptr: resolved.addr(),
            state_id: 0,
        }],
    }
}

/// The walk's own slot read serves the selector resolve, and the resolve does
/// not read the slot again.
///
/// The staged live slot holds the WRONG pointer, so this is not a test that
/// merely agrees with itself: a resolve that read the slot refuses, and a
/// resolve served the caller's already-validated value succeeds. The two arms
/// are the same fixture and differ only in the hint.
///
/// MUTATION RUN: the `Some((observed_address, observed_value)) if
/// observed_address == province_slot` arm in `d81_resolve_anchor_selector_key`
/// deleted, so the slot is always read. Observed in `--release`:
///
/// ```text
/// the caller's own read of this exact slot must serve the resolve: D81FirstFailure { stage: AnchorSelector, reason: ClosureChanged, subject_id: 6338, related_id: 260, index: 3, raw_value: 1287452008224, anchor_province_ptr: 0, anchor_address: 0, anchor_variant: 0, read_address: 0, read_width: 0, read_attempt: 0, read_success_mask: 0, read_first_value: 0, read_second_value: 0 }
/// ```
///
/// Restored.
#[test]
fn the_walks_own_slot_read_serves_the_selector_resolve_for_the_same_slot() {
    let (resolved, _replacement, province_array) = selector_fixture();
    let topology = selector_topology(&resolved, &province_array);
    let slot = province_array.at(SELECTOR_PROVINCE_ID as usize * 8);

    let route = d81_resolve_anchor_selector_key(
        &crate::attach_log::d81_copy_bracket::D81ReadSource::Live,
        6338,
        &topology,
        SELECTOR_CPROVINCE_VTABLE,
        SELECTOR_PROVINCE_ID,
        Some((slot, resolved.addr())),
    )
    .expect("the caller's own read of this exact slot must serve the resolve");
    assert_eq!(route.selector_key, SELECTOR_PROVINCE_ID);
    assert_eq!(route.province_slot, slot);
    assert_eq!(route.province_ptr, resolved.addr());
}

/// Without the hint the same fixture refuses, which is what makes the arm above
/// a measurement of the elision rather than of the fixture.
#[test]
fn the_same_fixture_refuses_when_the_resolve_reads_the_slot_itself() {
    let (resolved, replacement, province_array) = selector_fixture();
    let topology = selector_topology(&resolved, &province_array);
    let failure = d81_resolve_anchor_selector_key(
        &crate::attach_log::d81_copy_bracket::D81ReadSource::Live,
        6338,
        &topology,
        SELECTOR_CPROVINCE_VTABLE,
        SELECTOR_PROVINCE_ID,
        None,
    )
    .expect_err("a live slot that no longer holds the topology pointer must refuse");
    assert_eq!(failure.stage, D81FailureStage::AnchorSelector);
    assert_eq!(failure.reason, D81FailureReason::ClosureChanged);
    assert_eq!(failure.index, 3);
    assert_eq!(failure.raw_value, replacement.addr() as u64);
}

/// A hint for a DIFFERENT address is ignored and the slot is read.
///
/// The hint is honoured on address identity and nothing else. A caller that
/// passed the wrong province's slot -- or a future caller that passed a slot it
/// had not validated at THIS address -- changes no outcome.
///
/// MUTATION RUN: the `if observed_address == province_slot` guard deleted, so
/// any hint is honoured. Observed in `--release`:
///
/// ```text
/// a hint for a different address must be ignored and the slot read: D81AnchorSelectorRoute { selector_key: 260, province_slot: 2347307032464, province_ptr: 2347307020496 }
/// ```
///
/// Restored.
#[test]
fn a_hint_that_names_another_address_is_ignored_and_the_slot_is_read() {
    let (resolved, replacement, province_array) = selector_fixture();
    let topology = selector_topology(&resolved, &province_array);
    let other_slot = province_array.at(SELECTOR_PROVINCE_ID as usize * 8 - 8);
    let failure = d81_resolve_anchor_selector_key(
        &crate::attach_log::d81_copy_bracket::D81ReadSource::Live,
        6338,
        &topology,
        SELECTOR_CPROVINCE_VTABLE,
        SELECTOR_PROVINCE_ID,
        Some((other_slot, resolved.addr())),
    )
    .expect_err("a hint for a different address must be ignored and the slot read");
    assert_eq!(failure.reason, D81FailureReason::ClosureChanged);
    assert_eq!(failure.index, 3);
    assert_eq!(failure.raw_value, replacement.addr() as u64);
}

/// The surviving closing read is the LATER of the two, and the elided one's
/// comparison runs first.
///
/// This is the same property lane BS's controller witnesses have, in the same
/// shape and for the same reason: a deduplicated re-read must land no EARLIER
/// than the latest re-read it stands in for, or the span it vouches for is a
/// prefix of theirs instead of a superset. Here the two sites are separated by a
/// comparison and nothing else, so no offline fixture can distinguish them --
/// the position of the read IS the claim, and it is asserted against the source.
///
/// MUTATION RUN: the elision moved to the EARLIER position -- the selector
/// close made unconditional and the SOURCE close replaced by
/// `if selector_slot_is_source_slot { closing_selector_ptr } else { ..read.. }`,
/// so the surviving read is the earlier of the two and the span it vouches for
/// ends sooner than the span it replaced. That is the defect this test exists
/// for, and it is invisible to every behavioural test. Observed in `--release`:
///
/// ```text
/// the SELECTOR close is the one that is skipped when the two slots coincide
/// ```
///
/// Restored.
#[test]
fn the_surviving_closing_slot_read_is_the_later_of_the_two() {
    let source = collector_source_lf();
    let walk = source
        .split_once("fn read_compact_rail_replay_state(")
        .expect("the d81 walk")
        .1;
    assert!(
        walk.contains(
            "let selector_slot_is_source_slot = selector_route.province_slot == source_province_slot;"
        ),
        "the elision must be keyed on the two slots being ONE address, never \
         assumed"
    );
    let selector_close = walk
        .split_once("if !selector_slot_is_source_slot {")
        .expect(
            "the SELECTOR close is the one that is skipped when the two slots \
             coincide",
        )
        .1;
    assert!(
        selector_close.starts_with(
            "\n            let closing_selector_ptr = match d81_read_u64_pair_at(\n                source,\n                selector_route.province_slot,\n                d81_copy_bracket::D81Image::Closing,\n            ) {"
        ),
        "the read that survives must be the SOURCE close -- the later of the \
         two -- or the deduplicated span is a prefix of the one it replaces"
    );
    let after_surviving_read = walk
        .split_once(
            "let closing_source_province_ptr = match d81_read_u64_pair_at(\n            source,\n            source_province_slot,\n            d81_copy_bracket::D81Image::Closing,\n        ) {",
        )
        .expect("the surviving closing read")
        .1;
    let selector_comparison = after_surviving_read
        .find("closing_source_province_ptr != selector_route.province_ptr")
        .expect("the elided selector comparison, run against the surviving read");
    let source_comparison = after_surviving_read
        .find("if closing_source_province_ptr != root.province_ptr {")
        .expect("the source comparison");
    assert!(
        selector_comparison < source_comparison,
        "the selector close reported FIRST before the elision, so its \
         comparison must still run first"
    );
    assert!(
        after_surviving_read.contains(
            "if selector_slot_is_source_slot {\n                    return Err(selector_close_read_failure(read_failure));"
        ),
        "and a refusal of the surviving read must be dressed in the SELECTOR's \
         clothes when the two slots are one, because the selector close is the \
         site that reported first"
    );
    assert!(
        walk.contains("Some((source_province_slot, opening_source_province_ptr)),"),
        "the walk must hand its own already-validated slot read to the selector \
         resolve, or the opening half of the elision does not happen"
    );
}

// ---------------------------------------------------------------------
// What the elisions cost, counted rather than modelled
// ---------------------------------------------------------------------

/// Guarded reads performed by `body`, taken from the collector's OWN per-domain
/// read counter under a domain id production never tags.
///
/// `rc_domain_take_and_reset` drains every domain slot, and `RC_ENABLED` is
/// process-global, so this must not run beside anything else that touches the
/// read-counter family. `#[ignore]` on the only caller is not that guarantee --
/// `--ignored` runs the ignored tests concurrently with each other, and with
/// everything else under `--include-ignored`. The lock is.
fn guarded_reads(body: impl FnOnce()) -> u64 {
    const UNUSED_DOMAIN_ID: u16 = 219;
    let _rc_counters = super::super::lock_rc_counters();
    RC_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = rc_domain_take_and_reset();
    {
        let _scope = RcDomainScope::new(UNUSED_DOMAIN_ID);
        body();
    }
    rc_flush_current_thread();
    RC_ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
    rc_domain_take_and_reset()
        .iter()
        .find(|(id, _)| *id == UNUSED_DOMAIN_ID)
        .map(|(_, counters)| counters[RC_PD_READS])
        .unwrap_or(0)
}

/// What each elision removes, in GUARDED CALLS PER FRAME.
///
/// COUNTS, NOT WALL TIME. Lane BG established that a guarded call costs ~80-90
/// ns almost independently of width, because the cost is the per-call
/// `range_confirmed_good` + `copy_selected` rather than the bytes moved. The
/// call count is deterministic and this machine's wall clock is not (lanes run
/// concurrently on it), so the count is the measurement and microseconds are a
/// secondary note.
///
/// MEASURED, `--release`, counted through the collector's own `rc_d219_reads`:
///
/// ```text
/// d81 guarded reads per call, counted
///   read_u32_stable                                : 2
///   d81_read_stable_bytes, 0x0C window             : 2
///   d81_read_stable_bytes, 0x88 window             : 2
///   d81_read_u64_pair                              : 2
///   d81_resolve_anchor_selector_key, no hint       : 6
///   d81_resolve_anchor_selector_key, caller's slot : 4
/// ```
///
/// THE PER-FRAME ARITHMETIC that follows from those counts:
///
///  * SITE 1, the anchor-selector slot. The resolve drops 6 -> 4, and the two
///    closing `d81_read_u64_pair`s over one address drop 4 -> 2. Four guarded
///    calls per railed province whose selector key is its own id; at t26's 3510
///    railed provinces that is -14,040 calls/frame, ~1.19 ms at 85 ns/call.
///    Lane BG's own measurement of a four-call-per-province cut on this walk was
///    ~0.87-0.92 ms/frame, so ~0.9-1.2 ms is the band.
///  * SITES 2+3, the two country windows. The roster loop's 4-byte tag read
///    widens to the 0x0C window at NO change in call count (2 -> 2), and the POV
///    loop's own 0x0C and 0x88 reads disappear (4 -> 0). Four guarded calls per
///    country per frame; at ~200 countries that is -800 calls/frame, ~68 us.
///
/// TOTAL ~-14,840 guarded calls per frame.
///
/// WHICH `d81_phase_*` UNITS MOVE, for t27 comparability: none of the work
/// changes phase. Site 1's resolve elision lands wholly inside
/// `d81_phase_walk_selector_us` and its closing elision wholly inside
/// `d81_phase_walk_close_us`; sites 2 and 3 are both inside `d81_phase_hub_us`,
/// and site 3's widened read stays in the same phase as the scalar it replaces.
/// Every unit keeps meaning exactly what it meant at t26 -- three of them simply
/// get smaller.
///
/// `#[ignore]` on purpose -- it toggles the process-global read-counter gate:
///
/// ```text
/// cargo test -p collector --release d81_elided_read_count_model -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn d81_elided_read_count_model() {
    page_cache_new_frame();
    let mut heap = FixtureRegion::new(0x400);
    heap.write_u64(0x00, 0x1122_3344_5566_7788);

    let scalar = guarded_reads(|| {
        read_u32_stable(heap.at(0x00)).expect("the staged scalar is readable");
    });
    let window_0c = guarded_reads(|| {
        let mut scratch = Vec::new();
        d81_read_stable_bytes(
            heap.at(0x00),
            D81_COUNTRY_IDENTITY_WINDOW_LEN,
            D81FailureStage::HubRoot,
            &mut scratch,
        )
        .expect("the staged window is readable");
    });
    let window_88 = guarded_reads(|| {
        let mut scratch = Vec::new();
        d81_read_stable_bytes(
            heap.at(0x00),
            D81_COUNTRY_SYSTEM_WINDOW_LEN,
            D81FailureStage::HubObject,
            &mut scratch,
        )
        .expect("the staged window is readable");
    });
    let u64_pair = guarded_reads(|| {
        d81_read_u64_pair_at(
            &crate::attach_log::d81_copy_bracket::D81ReadSource::Live,
            heap.at(0x00),
            crate::attach_log::d81_copy_bracket::D81Image::Opening,
        )
        .expect("the staged pointer is readable");
    });

    // The resolve, over a fixture whose live slot AGREES with the topology, so
    // both arms succeed and the only difference between them is the read.
    let mut resolved = FixtureRegion::new(0xA8);
    resolved.write_u64(0, SELECTOR_CPROVINCE_VTABLE as u64);
    resolved.write_u32(CPROVINCE_OFF_ID, SELECTOR_PROVINCE_ID);
    let mut province_array = FixtureRegion::new(0x828);
    province_array.write_u64(SELECTOR_PROVINCE_ID as usize * 8, resolved.addr() as u64);
    let topology = selector_topology(&resolved, &province_array);
    let slot = province_array.at(SELECTOR_PROVINCE_ID as usize * 8);
    let resolve = |hint: Option<(usize, usize)>| {
        guarded_reads(|| {
            d81_resolve_anchor_selector_key(
                &crate::attach_log::d81_copy_bracket::D81ReadSource::Live,
                6338,
                &topology,
                SELECTOR_CPROVINCE_VTABLE,
                SELECTOR_PROVINCE_ID,
                hint,
            )
            .expect("the staged province resolves");
        })
    };
    let resolve_reading = resolve(None);
    let resolve_served = resolve(Some((slot, resolved.addr())));

    println!("d81 guarded reads per call, counted");
    println!("  read_u32_stable                                : {scalar}");
    println!("  d81_read_stable_bytes, 0x0C window             : {window_0c}");
    println!("  d81_read_stable_bytes, 0x88 window             : {window_88}");
    println!("  d81_read_u64_pair                              : {u64_pair}");
    println!("  d81_resolve_anchor_selector_key, no hint       : {resolve_reading}");
    println!("  d81_resolve_anchor_selector_key, caller's slot : {resolve_served}");

    // Deterministic, so they are checked as well as printed.
    assert_eq!(scalar, 2, "a stable scalar read is two guarded calls");
    assert_eq!(
        (window_0c, window_88),
        (2, 2),
        "and a confirmed window is two guarded calls whatever its width -- \
         which is why widening the roster read costs nothing"
    );
    assert_eq!(u64_pair, 2);
    assert_eq!(
        (resolve_reading, resolve_served),
        (6, 4),
        "the caller's own slot read must remove exactly one stable read from \
         the resolve"
    );
}

// ---------------------------------------------------------------------
// Job 4 -- the foreign-value index is built for the provinces that are
// asked about, and for no others (lane CW, 2026-08-06)
// ---------------------------------------------------------------------
//
// WHAT THIS JOB IS ABOUT. t27's on-the-wire phase timers put 68.77% of d81's
// 51,357 us p50 read inside `d81_phase_hub_us` (35,317 us p50) -- not in the
// province/rail walk, which is 20.09%. Inside the hub, the only loop whose trip
// count reaches the millions is the one that copied EVERY row of EVERY related
// country's supply table into a per-POV index, once per POV: t27 runs 93 POVs
// over a 93-country roster, and the index was then read for only the province
// ids per POV that reached an output row -- of the order of 100, against whole
// tables copied in full.
//
// The change restricts that fold to the queried provinces. Because the fold
// decides what `foreign_max_raw` reports on every emitted row, the obligation
// is byte-for-byte equality of what d81 publishes -- not "close enough".

/// One entry of the per-POV index the production reader used to materialise.
///
/// TEST-ONLY SINCE LANE DA (2026-08-06). Production stopped building the index
/// — `maximum_excluding_controller` probes the related tables directly — so the
/// entry type survives only here, as the shape the ORACLES are written in. That
/// is the right place for it: an oracle that shared production's data structure
/// would stop being independent of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct D81ForeignValue {
    pub(crate) raw_handle: u32,
    pub(crate) equivalence_id: u32,
    pub(crate) value: i64,
}

/// The index the fold built BEFORE the restriction: every province of every
/// related country's table, unconditionally.
///
/// This is an INDEPENDENT ORACLE, deliberately written out here rather than
/// obtained by calling a production fold with a permissive argument. A test
/// whose two arms are the same function in two configurations cannot fail when
/// that function is wrong in both; this one can. It is a faithful transcription
/// of the loop body lane CW restricted and lane DA deleted.
fn unrestricted_foreign_index(
    tables: &[(u32, u32, BTreeMap<u32, i64>)],
) -> BTreeMap<u32, Vec<D81ForeignValue>> {
    let mut values_by_province: BTreeMap<u32, Vec<D81ForeignValue>> = BTreeMap::new();
    for (raw_handle, equivalence_id, rows) in tables {
        for (&province_id, &value) in rows {
            values_by_province
                .entry(province_id)
                .or_default()
                .push(D81ForeignValue {
                    raw_handle: *raw_handle,
                    equivalence_id: *equivalence_id,
                    value,
                });
        }
    }
    values_by_province
}

/// Deterministic pseudo-random shapes, so the differential covers table sizes
/// on BOTH sides of the branch threshold rather than one convenient case.
fn cw_splitmix(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *seed;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Build the lazy source the production read hands back, from tables a test
/// owns.
///
/// The identity pairs and their ORDER are the ones the related-handle vector
/// gave the read, which is what makes a differential against
/// [`unrestricted_foreign_index`] an entry-for-entry comparison and not just a
/// comparison of maxima.
/// The oracle's tables in the layout PRODUCTION probes.
///
/// The differential tests keep their staged tables as `BTreeMap`s — the oracle
/// arm has to stay written on a container production does not use, or a
/// `D81RowTable` that dropped or reordered a row would move both arms together
/// and the assertion could not see it. This converts, so the production arm is
/// driven through the shipped structure while the oracle is not.
fn probe_tables_over(
    tables: &[(u32, u32, BTreeMap<u32, i64>)],
) -> Vec<(u32, u32, D81RowTable)> {
    tables
        .iter()
        .map(|(raw_handle, equivalence_id, rows)| {
            (
                *raw_handle,
                *equivalence_id,
                rows.iter().map(|(&key, &value)| (key, value)).collect(),
            )
        })
        .collect()
}

/// The collecting form of production's `d81_hub_foreign_source_into`.
///
/// Production refills a scratch vector it owns for the whole POV loop; a test
/// that builds one source and holds it wants the vector back, so this is the
/// same call with a fresh buffer. It is a wrapper and not a transcription: the
/// order and the table bindings are production's.
pub(super) fn d81_hub_foreign_source_for<'cache>(
    plan: &D81HubPovPlan,
    tables: &'cache BTreeMap<usize, D81HashTableSnapshot>,
) -> D81ForeignSource<'cache> {
    let mut related = Vec::new();
    d81_hub_foreign_source_into(plan, tables, &mut related);
    D81ForeignSource { related }
}

fn lazy_source_over<'tables>(
    tables: &'tables [(u32, u32, D81RowTable)],
) -> D81ForeignSource<'tables> {
    D81ForeignSource {
        related: tables
            .iter()
            .map(|(raw_handle, equivalence_id, rows)| D81ForeignRelated {
                raw_handle: *raw_handle,
                equivalence_id: *equivalence_id,
                rows,
            })
            .collect(),
    }
}

/// The probe looks in every related table and reports exactly which of them
/// held the province -- over shapes that cover every population the fold used
/// to branch on.
///
/// WHAT THIS REPLACED, AND WHY IT HAD TO CHANGE (lane DA, 2026-08-06). CW's
/// `the_restricted_foreign_fold_agrees_with_the_unrestricted_one_on_every_shape`
/// asserted that the restricted fold's `values_by_province` equalled the
/// unrestricted one's on every queried province and held nothing outside it.
/// Its subject -- a materialised index -- no longer exists, so the assertion
/// cannot be kept as written. The CLAIM behind it survives as two: the ANSWER
/// must match the unrestricted index's (asserted here and, over the real read,
/// in the two tests that follow) and the probe must have looked in exactly the
/// tables that hold the province, which is what `hub_probe_hits` publishes.
///
/// BOTH ORACLES ARE INDEPENDENT OF PRODUCTION, which the first draft of this
/// test got wrong: it reconstructed the entry list from `source.related` --
/// production's own vector -- and compared it to the oracle, an assertion no
/// production change could break because both sides were test code. The
/// expected hit count is now taken from `unrestricted_foreign_index`, where the
/// number of entries a province carries IS the number of tables that hold it.
///
/// THE SHAPES ARE STILL THE POINT, for a different reason. CW's fold branched
/// on `|queried| < |table|`, so its shapes had to straddle that threshold; the
/// probe has no such branch, but the shapes are kept anyway -- 1..8 tables of
/// 1..400 rows against 0..200 queried provinces over a 512-province space --
/// because they cover empty query sets and provinces held by none, one and
/// several tables.
///
/// MUTATION RUNS, both performed against production and observed:
///
///  * `for related in &self.related` restricted to `.take(1)`:
///
/// ```text
/// assertion `left == right` failed: trial 0: every related table is probed
/// for province 0
///   left: 1
///  right: 2
/// ```
///
///  * the probe's `hits += 1` moved above the `else { continue }`:
///
/// ```text
/// assertion `left == right` failed: trial 0: the hits must be the tables that
/// held province 0
///   left: 2
///  right: 0
/// ```
///
/// Both restored. And one that deliberately does NOT fail: `probes += 1`
/// hoisted out of the loop to a single `bump(self.related.len())`. The two are
/// equal while the loop has no early exit, which is the fact
/// `the_probe_charges_for_queries_performed_not_for_routes_planned` documents.
#[test]
fn the_query_time_probe_counts_exactly_the_tables_it_looked_in() {
    const PROVINCE_SPACE: u32 = 512;
    let mut seed = 0x0D81_0C00_u64;
    let mut provinces_with_several = 0usize;

    for trial in 0..400u32 {
        let table_count = 1 + (cw_splitmix(&mut seed) % 8) as usize;
        let needed_len = (cw_splitmix(&mut seed) % 201) as usize;
        let tables: Vec<(u32, u32, BTreeMap<u32, i64>)> = (0..table_count)
            .map(|index| {
                let raw_handle = index as u32 + 1;
                let rows_len = 1 + (cw_splitmix(&mut seed) % 400) as usize;
                let rows = (0..rows_len)
                    .map(|_| {
                        let province = (cw_splitmix(&mut seed) % u64::from(PROVINCE_SPACE)) as u32;
                        let value = (cw_splitmix(&mut seed) % 1_000) as i64 - 250;
                        (province, value)
                    })
                    .collect::<BTreeMap<u32, i64>>();
                (raw_handle, raw_handle + 1000, rows)
            })
            .collect();
        let queried: BTreeSet<u32> = (0..needed_len)
            .map(|_| (cw_splitmix(&mut seed) % u64::from(PROVINCE_SPACE)) as u32)
            .collect();

        let probe_tables = probe_tables_over(&tables);
        let source = lazy_source_over(&probe_tables);
        let unrestricted = unrestricted_foreign_index(&tables);
        let phases = D81PhaseAccum::default();

        for &province_id in &queried {
            // THE ORACLE. In the unrestricted index a province's entry list has
            // exactly one element per table that holds it, so its length is the
            // hit count the probe must report -- derived from the transcribed
            // fold, never from the vector production walks.
            let expected_hits = unrestricted
                .get(&province_id)
                .map(Vec::len)
                .unwrap_or_default();
            if expected_hits > 1 {
                provinces_with_several += 1;
            }

            let before = phases.snapshot();
            let answer = source.maximum_excluding_controller(&phases, province_id, 0, 0);
            let after = phases.snapshot();
            assert_eq!(
                answer,
                reference_maximum(&unrestricted, province_id, 0, 0),
                "trial {trial}: province {province_id} must answer as the index did"
            );
            assert_eq!(
                after.hub_query_probes - before.hub_query_probes,
                tables.len() as u64,
                "trial {trial}: every related table is probed for province {province_id}"
            );
            assert_eq!(
                after.hub_probe_hits - before.hub_probe_hits,
                expected_hits as u64,
                "trial {trial}: the hits must be the tables that held province {province_id}"
            );
        }
    }

    // Non-vacuous: the hit count is only a claim about LOOKING when some
    // province is held by several of the tables and missing from others.
    assert!(
        provinces_with_several > 0,
        "the shapes must produce provinces held by several related tables"
    );
}

/// The production selection rule, transcribed independently of the production
/// code, over the UNRESTRICTED index.
///
/// THIS EXISTS TO BREAK A CIRCULARITY (Codex review
/// 019fd618-5241-73b0-af78-470bbee000d6, finding 1). The first draft of the
/// test below compared `maximum_excluding_controller` against ITSELF over two
/// indexes. That catches a fold that drops entries, but it is blind to the
/// predicate: a mutation inside the filter moves both arms by the same amount
/// and the assertion still passes. Driving one arm through this transcription
/// makes the predicate itself a thing the test can falsify.
fn reference_maximum(
    index: &BTreeMap<u32, Vec<D81ForeignValue>>,
    province_id: u32,
    controller_raw_handle: u32,
    controller_equivalence_id: u32,
) -> i64 {
    let mut best = 0i64;
    for entry in index.get(&province_id).into_iter().flatten() {
        let excluded_by_handle = entry.raw_handle == controller_raw_handle;
        let excluded_by_equivalence = entry.raw_handle != 0
            && controller_raw_handle != 0
            && entry.equivalence_id == controller_equivalence_id;
        if !excluded_by_handle && !excluded_by_equivalence && entry.value > 0 && entry.value > best {
            best = entry.value;
        }
    }
    best
}

/// Every query the hub can put to the source returns what the index returned.
///
/// `maximum_excluding_controller` is the only thing that ever read the index,
/// and its answer is what lands in a published row's `foreign_max_raw`.
/// Comparing the reachable FACTS alone (the test above) would leave open a
/// difference the predicate happens to expose; this drives the production
/// predicate over the query-time source against [`reference_maximum`] over the
/// UNRESTRICTED index, for every controller identity the staged data can
/// produce -- including the two zero-handle special cases the filter singles
/// out.
///
/// THE TWO ARMS ARE INDEPENDENT, which is the property CW's version had and
/// this one keeps: one arm is production's probe loop, the other is a
/// transcription of the predicate over a map built by a transcription of the
/// deleted fold. A mutation inside the production filter moves one arm only.
///
/// MUTATION RUN: the `related.raw_handle != controller_raw_handle` term deleted
/// from `maximum_excluding_controller` -- the controller's own supply then wins
/// its own query. Observed:
///
/// ```text
/// assertion `left == right` failed: trial 0: province 8 controller (0, 1000)
/// must report the same foreign maximum as the index did
///   left: 11
///  right: 0
/// ```
///
/// Restored. Note WHICH case caught it: controller handle 0, where the
/// remaining disjunction admits everything, so the deleted term was the only
/// thing keeping the controller's own table out of its own answer.
#[test]
fn the_query_time_probe_reports_the_same_foreign_maximum_for_every_query() {
    const PROVINCE_SPACE: u32 = 64;
    let mut seed = 0x51_9F_C0_DEu64;

    for trial in 0..200u32 {
        let table_count = 1 + (cw_splitmix(&mut seed) % 6) as usize;
        let tables: Vec<(u32, u32, BTreeMap<u32, i64>)> = (0..table_count)
            .map(|index| {
                // Handle 0 is a real identity here: the filter has explicit
                // `raw_handle == 0` and `controller_raw_handle == 0` arms.
                let raw_handle = index as u32;
                let rows_len = 1 + (cw_splitmix(&mut seed) % 40) as usize;
                let rows = (0..rows_len)
                    .map(|_| {
                        let province = (cw_splitmix(&mut seed) % u64::from(PROVINCE_SPACE)) as u32;
                        let value = (cw_splitmix(&mut seed) % 41) as i64 - 20;
                        (province, value)
                    })
                    .collect::<BTreeMap<u32, i64>>();
                // Two handles deliberately share an equivalence id, so the
                // `equivalence_id != controller_equivalence_id` term is live.
                (raw_handle, 1000 + raw_handle / 2, rows)
            })
            .collect();
        let queried: BTreeSet<u32> = (0..PROVINCE_SPACE)
            .filter(|_province| cw_splitmix(&mut seed) % 3 == 0)
            .collect();

        let probe_tables = probe_tables_over(&tables);
        let source = lazy_source_over(&probe_tables);
        let unrestricted = unrestricted_foreign_index(&tables);

        for &province_id in &queried {
            for (controller_raw_handle, controller_equivalence_id, _) in &tables {
                assert_eq!(
                    source.maximum_excluding_controller(
                        &D81PhaseAccum::default(),
                        province_id,
                        *controller_raw_handle,
                        *controller_equivalence_id,
                    ),
                    reference_maximum(
                        &unrestricted,
                        province_id,
                        *controller_raw_handle,
                        *controller_equivalence_id,
                    ),
                    "trial {trial}: province {province_id} controller \
                     ({controller_raw_handle}, {controller_equivalence_id}) must \
                     report the same foreign maximum as the index did"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------
// Job 4b -- the same proof through the REAL read, over staged memory
// ---------------------------------------------------------------------

/// A complete `CSupplySystem` foreign-source tree in real process memory.
///
/// WHAT IS STAGED, and why it has to be real memory rather than an injected
/// reader: `d81_read_foreign_source` reaches the engine through the same
/// guarded primitives production uses, mints `D81BytesWitness` values that
/// re-read those addresses, and gates every pointer on
/// `is_map_control_heap_pointer`. Staging the tree in an owned `FixtureRegion`
/// is what makes the witnesses, the pointer gates and the two-copy tear check
/// all run for real.
///
/// LAYOUT (offsets are the reader's own constants):
///   supply_system + 0x40      16-byte vector header {rows_data, cap, count}
///   rows_data + slot*0x78     one foreign row per country
///   row + 0x58                16-byte related header {related_data, cap, _}
///   row + 0x64                related count
///   related_data              `related_count` packed u32 country handles
///   companion + 0x08          0x11-byte table header {buckets, count, mask, probe}
///   buckets + i*0x18          bucket: marker at +4, key at +8, value at +0x10
struct ForeignFixture {
    /// Owns the staged bytes. Never read through this field -- the reader
    /// reaches them by ADDRESS -- but it must outlive every read, which is
    /// the whole reason it is held.
    #[allow(dead_code)]
    region: FixtureRegion,
    supply_system: usize,
    /// Handle -> (equivalence id, companion address), the authority the reader
    /// resolves every related handle through.
    authority: BTreeMap<u32, (u32, usize)>,
    /// The rows staged into each companion's table, by raw handle -- the
    /// oracle's input.
    staged_rows: Vec<(u32, u32, BTreeMap<u32, i64>)>,
    /// The addresses the read must witness, in the order it must witness them.
    expected_witness_addresses: [usize; 3],
}

/// Build a fixture with `country_count` countries, whose POV slot 0 is related
/// to handles `1..=related_count`, each of whose tables holds `rows_per_table`
/// provinces drawn from `province_space`.
fn stage_foreign_fixture(
    related_count: u32,
    rows_per_table: u32,
    province_space: u32,
    seed: &mut u64,
) -> ForeignFixture {
    const ROW_STRIDE: usize = 0x78;
    const COUNTRY_COUNT: u32 = 8;
    // Every table gets a power-of-two bucket count comfortably larger than its
    // occupancy, plus the sentinel slot the reader requires.
    let mask: u32 = (rows_per_table * 4).next_power_of_two().max(16) - 1;
    let max_probe: u8 = 1;
    let slot_count = mask + u32::from(max_probe) + 2;
    let table_bytes = slot_count as usize * 0x18;

    // EVERY staged object start is 8-aligned. `is_map_control_heap_pointer`
    // rejects an unaligned pointer, so an odd `related_count` -- whose packed
    // u32 handle vector is not a multiple of 8 bytes -- would otherwise push the
    // table region off alignment and make the reader refuse the FIXTURE rather
    // than the data. (It did: the first draft failed exactly this way on the
    // three- and five-country shapes, which is the gate doing its job.)
    let align8 = |offset: usize| (offset + 7) & !7usize;
    let vector_header_off = 0x40usize;
    let rows_off = 0x1000usize;
    let related_off = align8(rows_off + COUNTRY_COUNT as usize * ROW_STRIDE + 0x100);
    let tables_off = align8(related_off + related_count as usize * 4 + 0x100);
    let total = tables_off + related_count as usize * (0x40 + table_bytes) + 0x1000;

    let mut region = FixtureRegion::new(total);
    let base = region.addr();
    assert!(
        is_map_control_heap_pointer(base),
        "the staged region must satisfy the reader's heap-pointer gate; \
         got {base:#x}, which is outside [{HEAP_LO:#x}, {HEAP_HI:#x})"
    );

    region.write_u64(vector_header_off, (base + rows_off) as u64);
    region.write_u32(vector_header_off + 8, COUNTRY_COUNT);
    region.write_u32(vector_header_off + 12, COUNTRY_COUNT);

    // POV slot 0's related vector.
    let pov_row = rows_off;
    region.write_u64(pov_row + 0x58, (base + related_off) as u64);
    region.write_u32(pov_row + 0x58 + 8, related_count);
    region.write_u32(pov_row + 0x64, related_count);

    let mut authority = BTreeMap::new();
    let mut staged_rows = Vec::new();
    for index in 0..related_count {
        let raw_handle = index + 1;
        region.write_u32(related_off + index as usize * 4, raw_handle);

        let companion_off = tables_off + index as usize * (0x40 + table_bytes);
        let buckets_off = companion_off + 0x40;
        let companion = base + companion_off;
        // Table header lives at companion + 0x08.
        region.write_u64(companion_off + 0x08, (base + buckets_off) as u64);
        region.write_u32(companion_off + 0x08 + 8, rows_per_table);
        region.write_u32(companion_off + 0x08 + 12, mask);
        region.write_u8(companion_off + 0x08 + 16, max_probe);

        // Occupy the first `rows_per_table` non-sentinel slots with distinct,
        // non-zero province keys.
        let mut rows = BTreeMap::new();
        let mut slot = 0usize;
        while (rows.len() as u32) < rows_per_table {
            let province = 1 + (cw_splitmix(seed) % u64::from(province_space)) as u32;
            if rows.contains_key(&province) {
                continue;
            }
            let value = (cw_splitmix(seed) % 200) as i64 - 50;
            let bucket = buckets_off + slot * 0x18;
            region.write_u8(bucket + 4, 1);
            region.write_u32(bucket + 8, province);
            region.write_i64(bucket + 0x10, value);
            rows.insert(province, value);
            slot += 1;
        }
        // The sentinel the reader insists on.
        region.write_u8(buckets_off + (slot_count as usize - 1) * 0x18 + 4, 0xff);

        authority.insert(raw_handle, (1000 + raw_handle, companion));
        staged_rows.push((raw_handle, 1000 + raw_handle, rows));
    }

    ForeignFixture {
        region,
        supply_system: base,
        authority,
        staged_rows,
        // The POV vector header, the related header, the related handle blob.
        expected_witness_addresses: [
            base + vector_header_off,
            base + pov_row + 0x58,
            base + related_off,
        ],
    }
}

/// The production read over staged memory answers, for every province, exactly
/// what the unrestricted fold's index answered -- and mints the same witnesses,
/// in the same order.
///
/// THIS IS THE READ, NOT A MODEL OF IT. `d81_read_foreign_source` is called
/// exactly as the hub calls it, over real memory, so the witnesses it mints,
/// the order it mints them in, and the table-cache population it performs are
/// all the production ones. The oracle is the independent
/// `unrestricted_foreign_index` transcription of the loop this change replaced.
///
/// WHAT LANE DA CHANGED HERE. The read no longer takes a queried set and no
/// longer returns an index, so what is asserted is the ANSWER for every
/// province in the fixture's space against `reference_maximum` over the
/// unrestricted index -- a superset of what the old assertion covered, since
/// the old one only looked at the queried set. Everything about the witnesses
/// and the cache is unchanged, including the mutation below.
///
/// A SECOND MUTATION, on the read rather than on the witnesses:
/// `resolved.push` guarded by `if index != 0`, so one authenticated related
/// table never reaches the probe. Observed:
///
/// ```text
/// assertion `left == right` failed: the staged read must answer province 3 for
/// controller (2, 1002) exactly as the unrestricted index did
///   left: 55
///  right: 84
/// ```
///
/// Restored.
///
/// MUTATION RUN: the related-header and related-blob witness pushes SWAPPED in
/// `d81_read_foreign_source`, i.e. exactly the reordering that would change the
/// span the closing gate vouches for without changing a single published byte.
/// Observed (addresses are the run's staged ones):
///
/// ```text
/// assertion `left == right` failed: the foreign read must mint the vector header,
/// the related header and the related blob -- at these addresses, with these bytes,
/// in this order
///   left: [(2602893242112, [..]), (2602893247360, [1,0,0,0, 2,0,0,0, 3,0,0,0, 4,0,0,0]),
///          (2602893246232, [..])]
///  right: [(2602893242112, [..]), (2602893246232, [..]),
///          (2602893247360, [1,0,0,0, 2,0,0,0, 3,0,0,0, 4,0,0,0])]
/// ```
///
/// Restored. This is the mutation the FIRST draft of this test could not see,
/// and the reason the assertion now compares all three ordered pairs rather
/// than the count plus two spot checks.
#[test]
fn the_staged_foreign_read_answers_like_the_unrestricted_index_and_mints_the_same_witnesses() {
    let mut seed = 0xF0_1E_16_4Eu64;
    // The same four shapes CW used, which straddle every table/query size ratio
    // the deleted fold branched on.
    for &(related_count, rows_per_table, province_space) in &[
        (4u32, 12u32, 64u32),
        (6, 200, 512),
        (3, 4, 32),
        (5, 64, 128),
    ] {
        let fixture = stage_foreign_fixture(related_count, rows_per_table, province_space, &mut seed);

        let mut table_cache = BTreeMap::new();
        let mut scratch = Vec::new();
        let mut budget = D81SnapshotBudget::new(D81_SNAPSHOT_BUDGET_BYTES);
        let plan = d81_read_foreign_source(
            fixture.supply_system,
            0,
            8,
            &fixture.authority,
            &mut table_cache,
            &mut budget,
            &mut scratch,
            &D81PhaseAccum::default(),
        )
        .expect("the staged foreign tree is complete and readable");
        // THE TWO HALVES, JOINED THE WAY PRODUCTION JOINS THEM (lane EK): the
        // read hands back a pointer-free plan, and the probe list is rebuilt
        // against the owned cache on the far side of the bracket close.
        let source = d81_hub_foreign_source_for(
            &D81HubPovPlan {
                foreign_related: plan.related.clone(),
                ..D81HubPovPlan::default()
            },
            &table_cache,
        );

        let oracle = unrestricted_foreign_index(&fixture.staged_rows);
        // EVERY province in the fixture's space, and every controller identity
        // the staged handles can present -- including the handles that are not
        // related to this POV at all, which is how a province held by no table
        // is covered.
        for province_id in 0..=province_space {
            for &(controller_raw_handle, controller_equivalence_id, _) in &fixture.staged_rows {
                assert_eq!(
                    source.maximum_excluding_controller(
                        &D81PhaseAccum::default(),
                        province_id,
                        controller_raw_handle,
                        controller_equivalence_id,
                    ),
                    reference_maximum(
                        &oracle,
                        province_id,
                        controller_raw_handle,
                        controller_equivalence_id,
                    ),
                    "the staged read must answer province {province_id} for \
                     controller ({controller_raw_handle}, \
                     {controller_equivalence_id}) exactly as the unrestricted \
                     index did"
                );
            }
        }
        assert_eq!(
            source.related.len(),
            related_count as usize,
            "the read must offer every related country's table to the probe"
        );

        // THE WITNESSES ARE THE CLOSING GATE. Three of them, in this order, at
        // these addresses -- the order is what makes the gate's re-read span a
        // superset of the read it vouches for, so it is asserted and not
        // assumed.
        // EVERY PAIR IS COMPARED, not a sample of them (Codex review
        // 019fd618-5241-73b0-af78-470bbee000d6, finding 2). Checking only the
        // count, witness 0's address and witness 2's length left a hole a
        // duplicate could walk through: replacing witness 1 with a second copy
        // of witness 0 silently retires the related-header witness from the
        // closing gate, and passed the earlier form of this assertion.
        let expected_lengths = [16usize, 16, related_count as usize * 4];
        let observed: Vec<(usize, Vec<u8>)> = plan
            .witnesses
            .iter()
            .map(|witness| (witness.address, witness.bytes.clone()))
            .collect();
        let expected: Vec<(usize, Vec<u8>)> = fixture
            .expected_witness_addresses
            .iter()
            .zip(expected_lengths)
            .map(|(&address, len)| {
                (
                    address,
                    read_bytes(address, len).expect("the staged window is readable"),
                )
            })
            .collect();
        assert_eq!(
            observed, expected,
            "the foreign read must mint the vector header, the related header \
             and the related blob -- at these addresses, with these bytes, in \
             this order"
        );
        assert!(
            plan.witnesses.iter().all(|witness| witness.unchanged()),
            "every witness must still re-read to the bytes it was minted from"
        );
        // The cache is populated for every related companion, whatever the
        // needed set contained -- a restriction that skipped a table would
        // silently drop that table's closing witnesses too.
        assert_eq!(
            table_cache.len(),
            related_count as usize,
            "every related companion's table must still be decoded and cached"
        );

        drop(fixture);
    }
}

/// The read hands back one probe target per related handle the header
/// authenticated, and the probe charge is that many per query.
///
/// WHY THIS SURVIVED LANE DA IN A CHANGED FORM. CW's version asserted a
/// `related_count` FIELD carried beside the fold, because a fold cannot count
/// probes it does not perform. There is no such field now — the probe walks
/// `source.related`, so the count IS the vector's length, and the failure mode
/// CW's note described (the field left at its `Default` by a `..` construction,
/// silently zeroing `hub_query_probes`) cannot be expressed. What CAN still go
/// wrong is the vector: a read that skipped a related handle, or that pushed a
/// handle twice, changes both what the queries see and what they are charged.
///
/// MUTATION RUN: `resolved.push(...)` guarded by `if index != 0`, i.e. the
/// first authenticated related handle is validated and then never offered to
/// the probe. Observed:
///
/// ```text
/// assertion `left == right` failed: every authenticated related handle must be
/// probeable
///   left: 5
///  right: 6
/// ```
///
/// The same mutation also reaches the wire, which is the point of it being a
/// read-level defect rather than a counting one -- it failed
/// `the_query_time_probe_produces_a_byte_identical_d81_payload` at
/// `Some((1248, 0, 97))` and the staged answer differential at province 3
/// (55 against 84). Restored.
///
/// The other live failure mode: sizing `related` from the related vector's
/// CAPACITY (8 here, against the authenticated count of 6) rather than from the
/// handles the loop admitted. That reads plausibly and would let a POV see
/// supply values from countries it is not related to.
#[test]
fn the_foreign_read_offers_one_probe_target_per_authenticated_handle() {
    let mut seed = 0x0081_C0DEu64;
    let fixture = stage_foreign_fixture(6, 48, 96, &mut seed);

    // The cache is a NAMED local, not a temporary, because the source borrows
    // it -- which is the query-time fold's ownership rule showing up in a test.
    let mut table_cache = BTreeMap::new();
    let plan = d81_read_foreign_source(
        fixture.supply_system,
        0,
        8,
        &fixture.authority,
        &mut table_cache,
        &mut D81SnapshotBudget::new(D81_SNAPSHOT_BUDGET_BYTES),
        &mut Vec::new(),
        &D81PhaseAccum::default(),
    )
    .expect("the staged foreign tree is complete and readable");
    let source = d81_hub_foreign_source_for(
        &D81HubPovPlan {
            foreign_related: plan.related.clone(),
            ..D81HubPovPlan::default()
        },
        &table_cache,
    );

    assert_eq!(
        source.related.len(),
        6,
        "every authenticated related handle must be probeable"
    );
    assert_eq!(
        source
            .related
            .iter()
            .map(|related| related.raw_handle)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6],
        "in the order the related-handle vector listed them"
    );

    // The charge the production call site now pays, stated against an example
    // rather than only against a declaration: 48 queries against 6 related
    // tables is 288 probes.
    let phases = D81PhaseAccum::default();
    for province_id in 1..=48u32 {
        source.maximum_excluding_controller(&phases, province_id, 0, 0);
    }
    let snapshot = phases.snapshot();
    assert_eq!(snapshot.hub_query_calls, 48);
    assert_eq!(snapshot.hub_query_probes, 288);
}

/// The accumulator's own contract: `hub_query_probes` is a sum of per-POV
/// PRODUCTS, so it separates shapes that the per-capture totals cannot.
///
/// WHAT THIS DOES AND DOES NOT COVER, said plainly. It drives the two cells the
/// way the production call site does — one bump pair per
/// `maximum_excluding_controller` invocation — and asserts the sum over two
/// deliberately ASYMMETRIC POVs (2 queries against 5 related, then 4 against 1)
/// is the correlated 14 and not the 18 an uncorrelated estimate from the
/// totals would give. That asymmetry is the whole reason the counter exists on
/// the wire rather than being computed offline, so it is worth pinning.
///
/// It exercises the PRODUCTION query, `maximum_excluding_controller`, which is
/// where the charge now lives. A POV whose routes never reach a query cannot be
/// charged, because charging is not a statement that sits near the query — it
/// is part of it.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL:
///  * deleting either `D81PhaseAccum::bump` from
///    `maximum_excluding_controller` — the counter goes to zero and d81's fold
///    decision is answered by silence;
///  * charging `self.related.len()` once instead of counting the loop's own
///    iterations — indistinguishable TODAY, since the loop has no early exit,
///    and the reason the assertion below also pins the per-POV asymmetry rather
///    than only the total;
///  * hoisting the charge back to the caller, which no longer type-checks
///    without also removing it from the query, i.e. the first mutation above;
///  * counting a miss as a hit (`hits += 1` moved above the probe's
///    `else { continue }`), run and observed:
///
/// ```text
/// assertion `left == right` failed: no table holds province 1, so every one of
/// those 14 probes missed
///   left: 14
///  right: 0
/// ```
#[test]
fn the_probe_charges_for_queries_performed_not_for_routes_planned() {
    let phases = D81PhaseAccum::default();
    // Tables holding nothing: this test is about the CHARGE, and an empty table
    // is probed exactly as a full one is.
    let empty: Vec<(u32, u32, D81RowTable)> = (0..9u32)
        .map(|handle| (handle + 1, handle + 1001, D81RowTable::default()))
        .collect();

    // A POV with many related countries answering few routes, then one with the
    // opposite shape. DELIBERATELY ASYMMETRIC: the correlated sum is 14 while
    // the uncorrelated estimate anyone would form from the published totals
    // (calls x related_total / povs = 6 x 6 / 2 = 18) is not, and telling those
    // two apart is the whole reason the counter exists.
    let wide = lazy_source_over(&empty[..5]);
    for _ in 0..2 {
        wide.maximum_excluding_controller(&phases, 1, 0, 0);
    }
    let narrow = lazy_source_over(&empty[..1]);
    for _ in 0..4 {
        narrow.maximum_excluding_controller(&phases, 1, 0, 0);
    }
    // The POV that refused before its first query -- one of the four failure
    // paths between a route and its query. It is never asked, so it pays
    // nothing, which is precisely what the pre-loop charge got wrong.
    let _aborted = lazy_source_over(&empty[..9]);

    let snapshot = phases.snapshot();
    assert_eq!(snapshot.hub_query_calls, 6);
    assert_eq!(
        snapshot.hub_query_probes, 14,
        "2 queries x 5 related plus 4 queries x 1 related"
    );
    assert_ne!(
        snapshot.hub_query_probes, 18,
        "the counter must not be reconstructible from the per-capture totals"
    );
    assert_eq!(
        snapshot.hub_probe_hits, 0,
        "no table holds province 1, so every one of those 14 probes missed"
    );
}

/// A POV with no related countries folds nothing, so a query-time fold would
/// probe nothing — and the counter must say ZERO rather than inherit a count
/// from the header it never used.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: filling `related` from
/// `country_count` (8 here) or from the related vector's CAPACITY rather than
/// from its authenticated count. Either reads plausibly, and would both charge
/// this POV for probes it never performs and — far worse — let it see supply
/// values from countries it is not related to.
#[test]
fn a_pov_with_no_related_countries_prices_no_probes() {
    let mut seed = 0x0081_0000u64;
    let fixture = stage_foreign_fixture(0, 48, 96, &mut seed);

    let mut table_cache = BTreeMap::new();
    let plan = d81_read_foreign_source(
        fixture.supply_system,
        0,
        8,
        &fixture.authority,
        &mut table_cache,
        &mut D81SnapshotBudget::new(D81_SNAPSHOT_BUDGET_BYTES),
        &mut Vec::new(),
        &D81PhaseAccum::default(),
    )
    .expect("a POV with an empty related vector is a complete read, not a refusal");
    let source = d81_hub_foreign_source_for(
        &D81HubPovPlan {
            foreign_related: plan.related.clone(),
            ..D81HubPovPlan::default()
        },
        &table_cache,
    );

    assert!(source.related.is_empty());

    let phases = D81PhaseAccum::default();
    for province_id in 1..=48u32 {
        assert_eq!(
            source.maximum_excluding_controller(&phases, province_id, 0, 0),
            0,
            "a POV with no related countries has no foreign maximum anywhere"
        );
    }
    let snapshot = phases.snapshot();
    assert_eq!(snapshot.hub_query_calls, 48);
    assert_eq!(snapshot.hub_query_probes, 0);
    assert_eq!(snapshot.hub_probe_hits, 0);
}

/// The wire bytes d81 publishes are identical, byte for byte.
///
/// (THIS DOC WAS ATTACHED TO THE WRONG TEST until lane DA moved it: CW's file
/// had these paragraphs sitting above
/// `the_foreign_read_reports_the_related_count_its_probes_are_priced_from`,
/// whose own doc followed them in the same `///` block, leaving the
/// byte-identity test with no doc at all.)
///
/// The two previous tests compare the reachable facts and the predicate. This
/// one closes the loop the domain actually cares about: it assembles
/// `RailHubSimulationState` rows from both forms exactly as the hub's
/// row-emission loop does, encodes them through the production payload encoder,
/// and compares the resulting payloads as bytes. A semantic drift anywhere in
/// the probe reaches `foreign_max_raw`, and `foreign_max_raw` is in the payload.
///
/// THE TWO ARMS SHARE NEITHER STRUCTURE NOR PREDICATE, which lane DA
/// strengthened from CW's version: CW drove BOTH arms through the production
/// `maximum_excluding_controller`, so a mutation inside the filter moved both
/// arms equally and this test could not see it (it was covered only by the
/// separate maximum differential). Arm one is now the production read over
/// staged memory answering through its probe loop; arm two is
/// `reference_maximum` over `unrestricted_foreign_index`. A mutation in either
/// the probe loop or the filter moves exactly one arm.
///
/// MUTATION RUN: `best = value` given `value.saturating_add(1)` -- the smallest
/// possible drift in the one field the probe decides. Observed:
///
/// ```text
/// assertion `left == right` failed: the query-time probe must produce a
/// byte-identical d81 payload (offset, probe byte, index byte)
///   left: Some((608, 58, 57))
///  right: None
/// ```
///
/// Restored. Offset 608 is inside a hub-simulation continuation row, which is
/// where `foreign_max_raw` is encoded -- the drift reached the wire, and the
/// test names the byte.
///
/// A SECOND MUTATION, run to establish what this test does NOT cover: `hits +=
/// 1` moved above the `let Some(&value) = ... else { continue }`, so misses are
/// counted as hits. No published byte moves and this test correctly passes;
/// `the_query_time_probe_counts_exactly_the_tables_it_looked_in`,
/// `the_probe_charges_for_queries_performed_not_for_routes_planned` and
/// `d81_foreign_maximum_excludes_raw_and_equivalent_controller_handles` are the
/// three that fail on it. Byte-identity and counter-truthfulness are separate
/// claims and are separately tested.
///
/// A THIRD, which this test DOES catch even though it is a read defect rather
/// than a predicate one: `resolved.push` guarded by `if index != 0`, so one
/// authenticated related table never reaches the probe. Observed here as
/// `Some((1248, 0, 97))` -- a foreign maximum that fell to zero because the
/// only table holding that province was the one withheld.
#[test]
fn the_query_time_probe_produces_a_byte_identical_d81_payload() {
    use compact_contract::rail_replay::{
        encode_payload_with_hub_simulation, RailHubSimulationState, RailNeighborState,
        RailProvinceState,
    };

    let mut seed = 0xB17E_1DE5u64;
    let fixture = stage_foreign_fixture(6, 48, 96, &mut seed);
    let queried: BTreeSet<u32> = (1..=48u32).collect();

    let mut table_cache = BTreeMap::new();
    let mut scratch = Vec::new();
    let probed_plan = d81_read_foreign_source(
        fixture.supply_system,
        0,
        8,
        &fixture.authority,
        &mut table_cache,
        &mut D81SnapshotBudget::new(D81_SNAPSHOT_BUDGET_BYTES),
        &mut scratch,
        &D81PhaseAccum::default(),
    )
    .expect("the staged foreign tree is complete and readable");
    let probed = d81_hub_foreign_source_for(
        &D81HubPovPlan {
            foreign_related: probed_plan.related.clone(),
            ..D81HubPovPlan::default()
        },
        &table_cache,
    );
    let index = unrestricted_foreign_index(&fixture.staged_rows);

    // A province carrier, so the payload exercises the whole encoder and not
    // only its hub tail. The encoder REQUIRES every directed edge to have its
    // reciprocal, so the provinces are staged as four mutually-linked pairs at
    // a shared edge level rather than as a ring.
    let provinces: Vec<RailProvinceState> = (1..=8u32)
        .map(|province_id| {
            let pair = (province_id - 1) / 2;
            let partner = if province_id % 2 == 1 {
                province_id + 1
            } else {
                province_id - 1
            };
            RailProvinceState {
                province_id,
                active_terminal: province_id % 2 == 0,
                anchor_variant: (province_id % 2) as u8,
                anchor_bits: [province_id, province_id + 1, province_id + 2],
                neighbors: vec![RailNeighborState {
                    province_id: partner,
                    edge_level: pair % 5 + 1,
                }],
            }
        })
        .collect();

    // ONE row builder, driven by whichever way of obtaining a foreign maximum
    // it is handed -- the production probe for one arm, the transcribed
    // predicate over the transcribed index for the other. Everything else about
    // the row is identical by construction, so the ONLY field that can differ
    // is the one under test.
    let rows_from = |maximum: &dyn Fn(u32, u32, u32) -> i64| -> Vec<RailHubSimulationState> {
        let mut rows = Vec::new();
        for (subtype, route_class) in [(0u8, 2u8), (1u8, 1u8)] {
            for &province_id in &queried {
                // The controller identities the hub would have resolved,
                // including the zero handle the filter special-cases.
                let controller_raw_handle = province_id % 4;
                let controller_country_id = 1000 + controller_raw_handle;
                rows.push(RailHubSimulationState {
                    // Non-zero: the encoder rejects a zero POV identity.
                    pov_country_id: 7,
                    province_id,
                    province_owner_country_id: controller_country_id.max(1),
                    subtype,
                    route_class,
                    node_kind: province_id % 3,
                    node_flag: (province_id % 2) as u8,
                    available_raw: i64::from(province_id) * 7,
                    consumed_raw: i64::from(province_id) * 3,
                    foreign_max_raw: maximum(
                        province_id,
                        controller_raw_handle,
                        controller_country_id,
                    ),
                    foreign_route: false,
                    fallback_record: false,
                });
            }
        }
        rows.sort_unstable_by_key(|row| (row.pov_country_id, row.province_id, row.subtype));
        rows
    };

    let probed_rows = rows_from(&|province_id, raw_handle, equivalence_id| {
        probed.maximum_excluding_controller(
            &D81PhaseAccum::default(),
            province_id,
            raw_handle,
            equivalence_id,
        )
    });
    let index_rows = rows_from(&|province_id, raw_handle, equivalence_id| {
        reference_maximum(&index, province_id, raw_handle, equivalence_id)
    });

    let encode = |rows: &[RailHubSimulationState]| {
        encode_payload_with_hub_simulation(&provinces, rows, 0, 0, provinces.len() as u32, 8)
            .expect("the staged snapshot encodes")
    };
    let probed_payload = encode(&probed_rows);
    let index_payload = encode(&index_rows);

    // THE BYTES ARE CHECKED FIRST, and reported as ONE offset rather than as two
    // whole buffers. `assert_eq!` on the payloads (or on the row vectors) prints
    // both sides in full -- a 48 KB dump that says "they differ" and nothing
    // about WHERE, which is useless as recorded evidence for a byte-identity
    // claim. The first differing offset is the diagnostic that identifies the
    // field.
    assert_eq!(
        probed_payload.len(),
        index_payload.len(),
        "the query-time probe must produce a d81 payload of the same length"
    );
    let first_difference = probed_payload
        .iter()
        .zip(&index_payload)
        .position(|(left, right)| left != right);
    assert_eq!(
        first_difference.map(|at| (at, probed_payload[at], index_payload[at])),
        None,
        "the query-time probe must produce a byte-identical d81 payload \
         (offset, probe byte, index byte)"
    );
    // Only now the rows, as the finer-grained statement behind the bytes.
    assert_eq!(
        probed_rows, index_rows,
        "the query-time probe must produce identical hub simulation rows"
    );
    // Non-vacuous: the payload has to actually carry a non-zero foreign
    // maximum, or byte-identity would be a statement about zeros.
    assert!(
        probed_rows.iter().any(|row| row.foreign_max_raw > 0),
        "the fixture must produce at least one non-zero foreign maximum"
    );
}

// ---------------------------------------------------------------------
// Job 5 -- the query-time fold, priced on t31's MEASURED shape
// ---------------------------------------------------------------------

/// t31's per-frame d81 hub shape, p50 over the 260 frames that published a
/// trailer (`runs/live_unlock/t31`, read back with
/// `sweep_span_audit.exe collector_stream.bin --domain 81`).
///
/// EVERY ONE OF THESE IS A MEASUREMENT, which is what separates this bench from
/// the t27 model it replaces. That model had 93 POVs and 11,570 rows measured,
/// the related count ASSUMED at 92, and the queried-set size a PROXY of
/// unknown error direction. All four are now counters on the wire.
pub(crate) mod t31 {
    pub(crate) const POV_COUNT: u64 = 439;
    pub(crate) const RELATED_VISITS: u64 = 2_005;
    pub(crate) const QUERY_CALLS: u64 = 11_586;
    pub(crate) const QUERY_PROBES: u64 = 345_704;
    pub(crate) const TABLE_ROWS_VISITED: u64 = 552_492;
    /// Published as `d81_phase_hub_index_entries` in t31; lane DA renamed the
    /// unit to `d81_phase_hub_probe_hits` (see `D81PhaseTimes`).
    pub(crate) const PROBE_HITS: u64 = 281_693;
    pub(crate) const READ_US: u64 = 48_449;
    pub(crate) const HUB_US: u64 = 32_766;
    pub(crate) const HUB_FOREIGN_US: u64 = 19_740;
    pub(crate) const HUB_NODES_US: u64 = 2_014;
    pub(crate) const HUB_CLOSE_US: u64 = 1_388;
    pub(crate) const PROVINCES_VISITED: u32 = 13_413;
}

/// The three POV tiers that reproduce t31's four hub totals simultaneously.
///
/// `(count, related countries, queries)`. WHY TIERS AND NOT A MEAN: the totals
/// are only reproducible by a CORRELATED distribution, which is CX's whole
/// point about `hub_query_probes`. A uniform 439-POV shape carrying the
/// measured means (4.57 related, 26.4 queries) yields 52,900 probes against the
/// measured 345,704 -- 6.5x low. These tiers were solved against the four sums,
/// and the realised shape is ASSERTED against t31 below rather than assumed.
const T31_TIERS: [(usize, usize, usize); 3] = [(20, 48, 330), (89, 8, 38), (330, 1, 5)];

/// `hub_table_rows_visited / hub_related_visits` = 552,492 / 2,005 = 275.6.
///
/// One size for every table, which is the fixture's stated simplification: t31
/// publishes the SUM over visits, not the distribution, so per-country table
/// size variance is not recoverable from the wire and is not reproduced here.
const T31_ROWS_PER_TABLE: usize = 276;

/// `hub_probe_hits / hub_query_probes` = 281,693 / 345,704 = 81.48%: a queried
/// province is present in four out of five of the tables its POV probes.
///
/// THIS IS THE FIXTURE'S HARDEST CONSTRAINT and the reason it is not built from
/// uniform random draws. Tables hold 276 of 13,413 provinces, so an
/// independent-draw model gives a 2% hit rate, not 81%. The real cause is
/// geographic: a POV's supply routes run through provinces its neighbours'
/// supply tables also carry. The fixture reproduces that with a CORRIDOR -- a
/// pool of 339 province ids shared by one group of countries -- from which both
/// the tables (276 of 339 = 81.4%) and the queries are drawn.
const T31_CORRIDOR: usize = 339;
/// Countries per corridor. Must be at least the widest tier's related count
/// (48) or a major power's related set would span corridors and the hit rate
/// would collapse.
const T31_GROUP: usize = 55;

/// CW's restricted eager fold, transcribed. This is the BEFORE arm, and it is a
/// transcription for the same reason `unrestricted_foreign_index` is: the
/// production function was deleted, and a bench arm has to be able to outlive
/// the code it prices.
///
/// Returns the entries it added, exactly as the deleted production fold did, so
/// the eager arm can make the `hub_index_entries` charge that fold made.
fn cw_restricted_foreign_fold(
    values_by_province: &mut BTreeMap<u32, Vec<D81ForeignValue>>,
    table_rows: &D81RowTable,
    queried_provinces: &BTreeSet<u32>,
    raw_handle: u32,
    equivalence_id: u32,
) -> u64 {
    let mut added = 0u64;
    let mut push = |province_id: u32, value: i64| {
        values_by_province
            .entry(province_id)
            .or_default()
            .push(D81ForeignValue {
                raw_handle,
                equivalence_id,
                value,
            });
    };
    if queried_provinces.len() < table_rows.len() {
        for &province_id in queried_provinces {
            if let Some(value) = table_rows.get(province_id) {
                push(province_id, value);
                added += 1;
            }
        }
    } else {
        for (province_id, value) in table_rows.iter() {
            if queried_provinces.contains(&province_id) {
                push(province_id, value);
                added += 1;
            }
        }
    }
    added
}

/// A whole frame's hub inputs at t31's shape: one table per country, and per
/// POV the countries it is related to and the provinces it queries.
pub(super) struct T31Frame {
    pub(super) tables: Vec<(u32, u32, D81RowTable)>,
    /// Per POV: indices into `tables`.
    pub(super) related: Vec<Vec<usize>>,
    /// Per POV: the province ids its row-emission loop queries, in order.
    pub(super) queries: Vec<Vec<u32>>,
}

impl T31Frame {
    pub(super) fn build(seed: &mut u64) -> Self {
        let country_count = T31_GROUP * ((t31::POV_COUNT as usize).div_ceil(T31_GROUP));
        let group_count = country_count / T31_GROUP;

        // One corridor per group: 339 distinct province ids out of the 13,413
        // the walk visits.
        let corridors: Vec<Vec<u32>> = (0..group_count)
            .map(|_| {
                let mut pool = BTreeSet::new();
                while pool.len() < T31_CORRIDOR {
                    pool.insert(1 + (cw_splitmix(seed) % u64::from(t31::PROVINCES_VISITED)) as u32);
                }
                pool.into_iter().collect()
            })
            .collect();

        // One supply table per country: 276 of its corridor's 339 provinces.
        let tables: Vec<(u32, u32, D81RowTable)> = (0..country_count)
            .map(|country| {
                let corridor = &corridors[country / T31_GROUP];
                let mut rows = BTreeMap::new();
                while rows.len() < T31_ROWS_PER_TABLE {
                    let province = corridor[(cw_splitmix(seed) % T31_CORRIDOR as u64) as usize];
                    rows.insert(province, (cw_splitmix(seed) % 4_000) as i64 - 1_000);
                }
                // Built as a map because the loop needs `len()` to converge on
                // a fixed row count, then handed over in the layout production
                // probes: the bench must exercise the shipped structure.
                (
                    country as u32 + 1,
                    country as u32 / 2 + 1_000,
                    rows.into_iter().collect::<D81RowTable>(),
                )
            })
            .collect();

        // Tiers, shuffled across the roster so the majors are not all in one
        // corridor.
        let mut tiers: Vec<usize> = T31_TIERS
            .iter()
            .enumerate()
            .flat_map(|(tier, (count, _, _))| std::iter::repeat_n(tier, *count))
            .collect();
        assert_eq!(tiers.len(), t31::POV_COUNT as usize);
        for index in (1..tiers.len()).rev() {
            tiers.swap(index, (cw_splitmix(seed) % (index as u64 + 1)) as usize);
        }

        let mut related = Vec::with_capacity(tiers.len());
        let mut queries = Vec::with_capacity(tiers.len());
        for (pov, &tier) in tiers.iter().enumerate() {
            let (_, related_count, query_count) = T31_TIERS[tier];
            let group = pov / T31_GROUP;
            // The POV's related countries are members of its own group, so
            // every table it probes draws from its own corridor.
            related.push(
                (0..T31_GROUP)
                    .map(|offset| group * T31_GROUP + offset)
                    .filter(|&country| country != pov)
                    .take(related_count)
                    .collect::<Vec<_>>(),
            );
            let corridor = &corridors[group];
            let mut picked = BTreeSet::new();
            while picked.len() < query_count {
                picked.insert(corridor[(cw_splitmix(seed) % T31_CORRIDOR as u64) as usize]);
            }
            queries.push(picked.into_iter().collect::<Vec<_>>());
        }

        Self {
            tables,
            related,
            queries,
        }
    }

    /// The controller identity the hub would have resolved for a province.
    /// Deterministic and shared by both arms, so it cannot bias one of them.
    pub(super) fn controller(province_id: u32) -> (u32, u32) {
        (province_id % 7, 1_000 + province_id % 11)
    }

    pub(super) fn related_visits(&self) -> u64 {
        self.related.iter().map(|list| list.len() as u64).sum()
    }

    fn table_rows_visited(&self) -> u64 {
        self.related
            .iter()
            .flat_map(|list| list.iter())
            .map(|&country| self.tables[country].2.len() as u64)
            .sum()
    }

    pub(super) fn query_calls(&self) -> u64 {
        self.queries.iter().map(|list| list.len() as u64).sum()
    }

    pub(super) fn query_probes(&self) -> u64 {
        self.related
            .iter()
            .zip(&self.queries)
            .map(|(related, queries)| related.len() as u64 * queries.len() as u64)
            .sum()
    }
}

/// Order statistics for a bench arm, in microseconds.
pub(super) fn percentiles_us(mut samples: Vec<u64>) -> String {
    samples.sort_unstable();
    let at = |fraction: f64| {
        let index = ((samples.len() - 1) as f64 * fraction).round() as usize;
        samples[index] as f64 / 1e3
    };
    format!(
        "min {:>8.2}  p25 {:>8.2}  p50 {:>8.2}  p75 {:>8.2}  p90 {:>8.2}  max {:>8.2}",
        at(0.0),
        at(0.25),
        at(0.5),
        at(0.75),
        at(0.9),
        at(1.0),
    )
}

/// Price the eager index against the query-time probe, per FRAME, at t31's
/// measured shape.
///
/// WHY A BENCH AND NOT AN ASSERTION. `#[ignore]` on purpose, for the reason
/// `d81_guarded_read_cost_model` gives: this is a measurement, and a wall-clock
/// gate in this tree flakes under lane load. Run it deliberately:
///
/// ```text
/// cargo test -p collector --release d81_foreign_fold_cost_model -- --ignored --nocapture
/// ```
///
/// WHAT EACH ARM CONTAINS, stated because the boundary is the whole result:
///  * EAGER is what lane CW shipped and lane DA deleted -- per POV, build the
///    queried-province `BTreeSet`, fold every related table into
///    `values_by_province` through it, then answer every query by looking the
///    province up in that map and scanning its vector. The set build is INSIDE
///    the arm because the fold cannot run without it, and its removal is part
///    of what the lazy form buys.
///  * LAZY is production's `maximum_excluding_controller` walking the POV's
///    related tables per query.
///
/// WHAT NEITHER ARM CONTAINS: the guarded reads, the witnesses, and the
/// hash-table decode that fills the frame's table cache. Those are IDENTICAL in
/// both forms -- the same 439 tables are decoded either way -- so including
/// them would only dilute the difference. They are measured separately by
/// `d81_copy_only_manifest_census`, which is where the copy-only architecture
/// question needs them.
///
/// BOTH ARMS ARE CHECKED AGAINST EACH OTHER on every trial: the frame checksum
/// is the sum of every foreign maximum published, and a bench that reported a
/// speedup for a fold that answers differently would be worse than no bench.
///
/// ARMS INTERLEAVED, order flipped every other trial, distributions reported
/// rather than a single number -- same discipline as the guarded-read model.
#[test]
#[ignore]
fn d81_foreign_fold_cost_model() {
    use std::time::Instant;

    const TRIALS: usize = 15;
    let mut seed = 0xD8_1D_A0_31u64;
    let frame = T31Frame::build(&mut seed);

    // THE FIXTURE IS T31's, OR THIS BENCH MEANS NOTHING. Every aggregate the
    // wire published for the hub is reproduced here within 1%, and the
    // assertion is what makes that a fact rather than an intention.
    let close_to = |realised: u64, measured: u64, name: &str| {
        let error = (realised as f64 - measured as f64).abs() / measured as f64;
        assert!(
            error < 0.01,
            "the fixture must reproduce t31's {name}: realised {realised}, \
             measured {measured} ({:.2}% off)",
            error * 100.0
        );
    };
    close_to(frame.related.len() as u64, t31::POV_COUNT, "hub_pov_count");
    close_to(frame.related_visits(), t31::RELATED_VISITS, "hub_related_visits");
    close_to(
        frame.table_rows_visited(),
        t31::TABLE_ROWS_VISITED,
        "hub_table_rows_visited",
    );
    close_to(frame.query_calls(), t31::QUERY_CALLS, "hub_query_calls");
    close_to(frame.query_probes(), t31::QUERY_PROBES, "hub_query_probes");

    // EACH ARM CARRIES ITS OWN PRODUCTION'S INSTRUMENTATION, and that is a
    // correction (Codex review 019fd6d8-8df3-71a1-96db-41c9be0fcf35, round 1).
    // The first draft gave the lazy arm production's three `Cell` updates per
    // query and the eager arm none, which charged the new form for diagnostics
    // the old form also paid. The eager arm now makes, into its own
    // accumulator, every charge the deleted code made inside the work these
    // arms contain: `hub_query_calls` and `hub_query_probes` per query, from
    // the deleted `maximum_excluding_controller`, and the entries-kept charge
    // per related visit, from the deleted fold.
    //
    // WHAT ASYMMETRY REMAINS, since exact is better claimed than assumed: the
    // lazy arm makes 3 cell writes per query (11,632 x 3) against the eager
    // arm's 2 per query plus 1 per related visit (11,632 x 2 + 2,002), i.e.
    // ~9.6k more cell writes -- about 10 us on an 8 ms arm, and it biases
    // AGAINST the lazy arm, so the reported saving is the conservative one.
    // The per-visit `hub_related_visits` / `hub_table_rows_visited` charges are
    // in neither arm because they are in `d81_read_foreign_source`, which is in
    // neither arm.
    let eager_phases = D81PhaseAccum::default();
    let eager_frame = || {
        let mut checksum = 0i64;
        for pov in 0..frame.related.len() {
            let queried: BTreeSet<u32> = frame.queries[pov].iter().copied().collect();
            let mut index: BTreeMap<u32, Vec<D81ForeignValue>> = BTreeMap::new();
            for &country in &frame.related[pov] {
                let (raw_handle, equivalence_id, rows) = &frame.tables[country];
                // The charge the deleted fold made per related visit, which is
                // where the old form's equivalent of `hub_probe_hits` lived.
                D81PhaseAccum::bump(
                    &eager_phases.hub_probe_hits,
                    cw_restricted_foreign_fold(
                        &mut index,
                        rows,
                        &queried,
                        *raw_handle,
                        *equivalence_id,
                    ),
                );
            }
            let related_count = frame.related[pov].len() as u64;
            for &province_id in &frame.queries[pov] {
                let (raw_handle, equivalence_id) = T31Frame::controller(province_id);
                D81PhaseAccum::bump(&eager_phases.hub_query_calls, 1);
                D81PhaseAccum::bump(&eager_phases.hub_query_probes, related_count);
                checksum = checksum.wrapping_add(reference_maximum(
                    &index,
                    province_id,
                    raw_handle,
                    equivalence_id,
                ));
            }
        }
        checksum
    };

    let phases = D81PhaseAccum::default();
    let lazy_frame = || {
        let mut checksum = 0i64;
        for pov in 0..frame.related.len() {
            let source = D81ForeignSource {
                related: frame.related[pov]
                    .iter()
                    .map(|&country| {
                        let (raw_handle, equivalence_id, rows) = &frame.tables[country];
                        D81ForeignRelated {
                            raw_handle: *raw_handle,
                            equivalence_id: *equivalence_id,
                            rows,
                        }
                    })
                    .collect(),
            };
            for &province_id in &frame.queries[pov] {
                let (raw_handle, equivalence_id) = T31Frame::controller(province_id);
                checksum = checksum.wrapping_add(source.maximum_excluding_controller(
                    &phases,
                    province_id,
                    raw_handle,
                    equivalence_id,
                ));
            }
        }
        checksum
    };

    // The set build on its own, so the part of the saving that is NOT the fold
    // is separable rather than inferred.
    let queried_sets_only = || {
        let mut total = 0usize;
        for queries in &frame.queries {
            let queried: BTreeSet<u32> = queries.iter().copied().collect();
            total += queried.len();
        }
        total
    };

    let mut eager_ns = Vec::with_capacity(TRIALS);
    let mut lazy_ns = Vec::with_capacity(TRIALS);
    let mut sets_ns = Vec::with_capacity(TRIALS);
    let mut agreed = 0usize;
    for trial in 0..TRIALS {
        let (eager, lazy) = if trial % 2 == 0 {
            let start = Instant::now();
            let eager = eager_frame();
            eager_ns.push(start.elapsed().as_nanos() as u64);
            let start = Instant::now();
            let lazy = lazy_frame();
            lazy_ns.push(start.elapsed().as_nanos() as u64);
            (eager, lazy)
        } else {
            let start = Instant::now();
            let lazy = lazy_frame();
            lazy_ns.push(start.elapsed().as_nanos() as u64);
            let start = Instant::now();
            let eager = eager_frame();
            eager_ns.push(start.elapsed().as_nanos() as u64);
            (eager, lazy)
        };
        assert_eq!(
            eager, lazy,
            "trial {trial}: the two arms must publish the same foreign maxima"
        );
        agreed += 1;
        let start = Instant::now();
        let distinct = queried_sets_only();
        sets_ns.push(start.elapsed().as_nanos() as u64);
        assert!(distinct > 0);
    }
    assert_eq!(agreed, TRIALS);

    // The probe accounting the lazy arm produced, divided by the trials it ran.
    let snapshot = phases.snapshot();
    let per_frame = |total: u64| total / TRIALS as u64;
    // Both arms charged the same probe total, which is the arithmetic claim
    // behind "the two forms perform the same table lookups" -- and the check
    // that the eager arm's instrumentation really is the deleted one's.
    let eager_snapshot = eager_phases.snapshot();
    assert_eq!(
        eager_snapshot.hub_query_probes, snapshot.hub_query_probes,
        "the eager and lazy arms must be charged the same probe total"
    );
    assert_eq!(eager_snapshot.hub_query_calls, snapshot.hub_query_calls);
    // The entries the index kept against the probes that hit, which is the
    // renamed unit's continuity claim measured rather than asserted in prose:
    // the fold counted a match once per distinct queried province, the probe
    // counts it once per query, and this fixture's queries are distinct, so the
    // two must be EQUAL here.
    assert_eq!(
        eager_snapshot.hub_probe_hits, snapshot.hub_probe_hits,
        "the index's entries-kept and the probe's hits must be the same quantity"
    );

    println!("d81 foreign fold, per FRAME at t31's measured shape");
    println!(
        "  fixture: {} POVs, {} related visits, {} table rows visited, \
         {} query calls, {} probes, {} hits ({:.2}% hit rate)",
        frame.related.len(),
        frame.related_visits(),
        frame.table_rows_visited(),
        frame.query_calls(),
        frame.query_probes(),
        per_frame(snapshot.hub_probe_hits),
        per_frame(snapshot.hub_probe_hits) as f64 * 100.0 / frame.query_probes() as f64,
    );
    println!(
        "  t31   : {} POVs, {} related visits, {} table rows visited, \
         {} query calls, {} probes, {} hits ({:.2}% hit rate)",
        t31::POV_COUNT,
        t31::RELATED_VISITS,
        t31::TABLE_ROWS_VISITED,
        t31::QUERY_CALLS,
        t31::QUERY_PROBES,
        t31::PROBE_HITS,
        t31::PROBE_HITS as f64 * 100.0 / t31::QUERY_PROBES as f64,
    );
    println!("  EAGER (index)  us  {}", percentiles_us(eager_ns.clone()));
    println!("  LAZY  (probe)  us  {}", percentiles_us(lazy_ns.clone()));
    println!("  of which the queried-set build alone:");
    println!("                 us  {}", percentiles_us(sets_ns));

    eager_ns.sort_unstable();
    lazy_ns.sort_unstable();
    let eager_p50 = eager_ns[TRIALS / 2];
    let lazy_p50 = lazy_ns[TRIALS / 2];
    println!(
        "\n  p50 delta {:+.2} ms  ({:.2}x)",
        (lazy_p50 as f64 - eager_p50 as f64) / 1e6,
        eager_p50 as f64 / lazy_p50.max(1) as f64,
    );
    println!(
        "  t31 published hub_us {} us, of which hub_foreign_us {}, hub_nodes_us {}, \
         hub_close_us {}; d81 read_time_us {}.",
        t31::HUB_US,
        t31::HUB_FOREIGN_US,
        t31::HUB_NODES_US,
        t31::HUB_CLOSE_US,
        t31::READ_US,
    );
    println!(
        "  Projected d81 read after the delta: {:.2} ms.",
        (t31::READ_US as f64 - (eager_p50 as f64 - lazy_p50 as f64) / 1e3) / 1e3,
    );
}

// ---------------------------------------------------------------------
// Job 6 -- what a copy-only bracket would have to move, and what it costs
// ---------------------------------------------------------------------

/// One range the hub read consumes: an address and a length.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct D81Range {
    address: usize,
    len: usize,
}

/// Coalesce a manifest: sort, merge overlapping and adjacent ranges, and report
/// what survived.
///
/// This is the step finding 5 of the copy-only architecture review asks for
/// between "what the reader consumed" and "what a scatter/gather pass would
/// copy" -- a reader that reads the same header twice, or reads two adjacent
/// windows, pays for one range here.
fn coalesce(mut ranges: Vec<D81Range>) -> Vec<D81Range> {
    ranges.sort_unstable();
    let mut merged: Vec<D81Range> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.address <= last.address + last.len => {
                let end = (range.address + range.len).max(last.address + last.len);
                last.len = end - last.address;
            }
            _ => merged.push(range),
        }
    }
    merged
}

/// What a copy-only bracket would have to acquire for d81's FOREIGN SUBTREE,
/// and what the optimistic scatter/gather copy of it costs.
///
/// WHY THIS EXISTS (owner architecture ruling + Codex review, 2026-08-06). The
/// proposed rule is that a clock bracket may contain only bulk copies, with all
/// parsing and folding moved after it. The review's finding 5 asks for the
/// cheapest disproof FIRST: take the manifest the validated reader actually
/// consumes, coalesce it, and time a bracket containing nothing but the copies.
/// If that alone exceeds the budget, copy-only is disproved before anyone
/// rewrites a reader.
///
/// WHAT THIS MEASURES, AND WHAT IT DELIBERATELY DOES NOT.
///
///  * IT IS A LOWER BOUND, not d81's manifest. It covers the foreign subtree
///    this lane owns -- the per-POV vector/related headers and handle blobs,
///    and the companion supply tables -- at t31's counters. d81's topology walk
///    over 13,413 provinces, its per-province rail objects, route maps and
///    supply nodes are NOT here and are strictly additive. A lower bound is the
///    right instrument for a disproof: if the bound exceeds the budget the
///    question is settled, and if it does not, nothing is settled.
///  * THE ADDRESSES ARE THE FIXTURE'S, which flatters the copy: a staged region
///    is contiguous and hot, while the engine's tables are scattered across a
///    multi-gigabyte heap with TLB and page-fault costs this cannot see. Again
///    optimistic, again the right direction for a disproof.
///  * THE DOUBLE READ IS INCLUDED. `d81_read_stable_bytes` copies every window
///    TWICE and compares, which is the structural-coherence check finding 3
///    says a copy-only bracket must keep. The manifest below therefore reports
///    unique bytes AND the bytes actually moved.
///
/// THE STRUCTURE CENSUS finding 2 asks for, for this subtree:
///  * `supply_system + 0x40` vector header -- FIXED offset from an already-held
///    pointer: flat-copyable.
///  * the foreign row at `rows_data + slot * 0x78` and its related header --
///    BOUNDED ARENA once `rows_data` is decoded, but `rows_data` comes out of
///    the header above: one level of in-bracket discovery.
///  * the packed related-handle blob -- length decoded from the related header:
///    a second level.
///  * each companion's table header at `companion + 0x08`, where `companion`
///    comes from the country-table join -- a third level, and the join itself
///    is a fourth.
///  * each table's bucket array, whose address AND length are decoded from that
///    header -- a fifth.
/// So d81's foreign subtree is GRAPH-SHAPED at depth five. It cannot be
/// acquired by a bounded set of copies planned before the bracket opens; a
/// copy-only bracket for it is really a copy/discover/copy pipeline, and
/// calling the discovery steps "not compute" would be exactly the silent
/// heavy-parsing-in-bracket the review warns against.
///
/// ```text
/// cargo test -p collector --release d81_copy_only_manifest_census -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn d81_copy_only_manifest_census() {
    use std::time::Instant;

    const TRIALS: usize = 51;
    // The engine's bucket array is a power-of-two-masked open-addressed table
    // plus its probe slack and sentinel; 276 occupied rows sits inside a
    // 512-bucket table at 54% load.
    const BUCKETS: usize = 512;
    const BUCKET_STRIDE: usize = 0x18;
    const TABLE_BLOB: usize = (BUCKETS - 1 + 1 + 2) * BUCKET_STRIDE;
    const TABLE_HEADER: usize = 0x11;
    const POV_HEADER: usize = 16;

    // A gap between every staged object, so the manifest does NOT coalesce
    // into a handful of megabyte runs. The engine's tables are separate heap
    // allocations reached through separate pointers; a fixture that laid them
    // end to end would let `coalesce` merge 2,195 ranges into a few dozen and
    // then report a sequential-copy bandwidth that no real manifest can reach.
    // The real heap's scatter is WORSE than this, never better.
    const GAP: usize = 64;

    let distinct_tables = t31::POV_COUNT as usize;
    let pov_count = t31::POV_COUNT as usize;
    let related_visits = t31::RELATED_VISITS as usize;

    let tables_bytes = distinct_tables * (TABLE_HEADER.next_multiple_of(8) + TABLE_BLOB + 2 * GAP);
    let povs_bytes = pov_count * (2 * POV_HEADER + 3 * GAP) + related_visits * 8;
    let region_len = tables_bytes + povs_bytes + 0x1000;
    let region = FixtureRegion::new(region_len);
    let base = region.addr();

    let mut manifest: Vec<D81Range> = Vec::new();
    let mut cursor = 0usize;
    let mut place = |manifest: &mut Vec<D81Range>, cursor: &mut usize, len: usize| {
        manifest.push(D81Range {
            address: base + *cursor,
            len,
        });
        *cursor += len.next_multiple_of(8) + GAP;
    };
    for _ in 0..distinct_tables {
        place(&mut manifest, &mut cursor, TABLE_HEADER);
        place(&mut manifest, &mut cursor, TABLE_BLOB);
    }
    // The per-POV related counts, taken from the SAME tier table the fold bench
    // fits to t31 rather than from modular arithmetic over the POV index.
    //
    // THE MODULAR FORM WAS WRONG (Codex review
    // 019fd6d8-8df3-71a1-96db-41c9be0fcf35, round 1): `pov % 22 == 0` and
    // `pov % 5 == 1` overlap on four POVs, so it produced 20x48 + 84x8 + 335x1
    // = 1,967 handles against t31's 2,005 -- and nothing checked, because the
    // region was sized from the constant while the manifest was built from the
    // branches. The sum is now asserted.
    let handle_counts: Vec<usize> = T31_TIERS
        .iter()
        .flat_map(|(count, related, _)| std::iter::repeat_n(*related, *count))
        .collect();
    assert_eq!(handle_counts.len(), pov_count);
    let staged_visits: usize = handle_counts.iter().sum();
    let visits_error =
        (staged_visits as f64 - related_visits as f64).abs() / (related_visits as f64);
    assert!(
        visits_error < 0.01,
        "the staged manifest must carry t31's related visits: {staged_visits} against \
         {related_visits} ({:.2}% off)",
        visits_error * 100.0
    );
    for &handles in &handle_counts {
        place(&mut manifest, &mut cursor, POV_HEADER);
        place(&mut manifest, &mut cursor, POV_HEADER);
        // The POV's own related-handle blob.
        place(&mut manifest, &mut cursor, handles * 4);
    }
    assert!(
        cursor < region_len,
        "the staged manifest must fit its region: {cursor} >= {region_len}"
    );
    // The last staged byte must be inside the region, checked through the
    // fixture's own bounds check rather than by arithmetic alone.
    let _ = region.at(cursor - 1);

    let coalesced = coalesce(manifest.clone());
    let unique_bytes: usize = coalesced.iter().map(|range| range.len).sum();
    let moved_bytes = unique_bytes * 2; // the tear-check second copy
    let pages: BTreeSet<usize> = coalesced
        .iter()
        .flat_map(|range| (range.address..range.address + range.len).step_by(4096))
        .map(|address| address / 4096)
        .collect();

    // Preallocated destination, as finding 5 specifies -- the copy pass is
    // being priced, not the allocator.
    let mut destination = vec![0u8; moved_bytes + 4096];
    // A buffer big enough to evict the staged region from L2/L3 between cold
    // trials.
    let mut evictor = vec![0u8; 64 << 20];

    // THE DESTINATION IS MADE OBSERVABLE OR THE WHOLE PASS IS DELETED. The
    // first version of this bench reported 10.39 MiB copied in 0.4 us -- 26
    // TB/s, which is not a memory bandwidth, it is dead-store elimination: the
    // destination was never read, so every `copy_from_slice` was provably
    // unobservable and LLVM removed all of them, leaving only the byte counter
    // it could fold to a constant. `black_box` on the buffer is what makes the
    // stores live, and it is inside the timed span because a copy nobody can
    // observe is not the thing being priced.
    let copy_pass = |destination: &mut [u8]| {
        let mut written = 0usize;
        for range in &coalesced {
            // TWO copies per range: the read and its tear-check confirmation,
            // which is the structural-coherence pair a copy-only bracket must
            // keep (review finding 3).
            for _ in 0..2 {
                let source =
                    unsafe { std::slice::from_raw_parts(range.address as *const u8, range.len) };
                destination[written..written + range.len].copy_from_slice(source);
                written += range.len;
            }
        }
        std::hint::black_box(&destination[..written]);
        written
    };

    let mut hot_ns = Vec::with_capacity(TRIALS);
    let mut cold_ns = Vec::with_capacity(TRIALS);
    for trial in 0..TRIALS {
        // COLD first, so the hot arm is warmed by the cold one rather than the
        // other way round.
        for (index, byte) in evictor.iter_mut().enumerate() {
            *byte = (index ^ trial) as u8;
        }
        std::hint::black_box(&evictor[..]);
        let start = Instant::now();
        let written = copy_pass(&mut destination);
        cold_ns.push(start.elapsed().as_nanos() as u64);
        assert_eq!(written, moved_bytes);

        let start = Instant::now();
        let written = copy_pass(&mut destination);
        hot_ns.push(start.elapsed().as_nanos() as u64);
        assert_eq!(written, moved_bytes);
    }

    let tail = |mut samples: Vec<u64>| {
        samples.sort_unstable();
        let at = |fraction: f64| {
            samples[((samples.len() - 1) as f64 * fraction).round() as usize] as f64 / 1e3
        };
        format!(
            "p50 {:>8.2}  p90 {:>8.2}  p99 {:>8.2}  max {:>8.2}",
            at(0.5),
            at(0.9),
            at(0.99),
            at(1.0)
        )
    };

    println!("d81 copy-only manifest census -- FOREIGN SUBTREE ONLY (a lower bound)");
    println!(
        "  ranges consumed {}, coalesced {}, unique {:.2} MiB, moved (with tear check) {:.2} MiB, \
         4 KiB pages {}",
        manifest.len(),
        coalesced.len(),
        unique_bytes as f64 / (1 << 20) as f64,
        moved_bytes as f64 / (1 << 20) as f64,
        pages.len(),
    );
    println!("  optimistic scatter/gather, HOT   us  {}", tail(hot_ns.clone()));
    println!("  optimistic scatter/gather, COLD  us  {}", tail(cold_ns.clone()));
    println!(
        "  t31 d81: read_time {} us, hub {} us, hub_foreign {} us; the sweep's \
         critical lane sat at 48.5 ms p50.",
        t31::READ_US,
        t31::HUB_US,
        t31::HUB_FOREIGN_US,
    );
    println!(
        "  DEPTH: five levels of pointer discovery stand between the bracket \
         opening and the last of these ranges being addressable (see this \
         test's doc). None of them can be planned from a previous hour without \
         risking a freed or relocated object."
    );
}

// ---------------------------------------------------------------------
// Lane FG -- the row table that replaced the per-table BTreeMap
// ---------------------------------------------------------------------

/// `D81RowTable` answers every key exactly as the `BTreeMap` it replaced did.
///
/// WHY THE PROBE DIFFERENTIALS ABOVE DO NOT COVER THIS. They drive both arms
/// off staged tables and compare the MAXIMUM; a table that lost a row whose
/// value is never the maximum, or that answers a key just outside its range
/// with a neighbour's value, can survive them. This compares the lookup itself,
/// key by key, against the container production stopped using.
///
/// THE KEY SPACE IS DELIBERATELY WIDER THAN THE TABLE'S, and the input order is
/// shuffled: `get`'s two bound comparisons are a short-circuit whose only
/// chance of being wrong is at the ends, and a table that relied on its input
/// arriving sorted would answer wrongly for everything.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL:
///  * searching `values` instead of `keys`, or indexing `values` with the
///    search result of a different array;
///  * `key <= first` or `key >= last` in the bound check, which drops the two
///    endpoints;
///  * a `FromIterator` that does not sort, or that resolves a repeated key to
///    the FIRST value offered instead of the last.
///
/// MUTATION RUN: `get`'s lower bound changed from `key < first` to
/// `key <= first`. Observed:
///
/// ```text
/// assertion `left == right` failed: trial 0: key 2 must answer as the map did
///   left: None
///  right: Some(-2637)
/// ```
///
/// Restored.
#[test]
fn the_row_table_answers_every_key_exactly_as_the_map_it_replaced() {
    const KEY_SPACE: u32 = 400;
    let mut seed = 0x0D81_F607_u64;
    let mut nonempty_tables = 0usize;
    let mut hits = 0usize;
    let mut misses = 0usize;

    for trial in 0..300u32 {
        let rows_len = (cw_splitmix(&mut seed) % 60) as usize;
        let mut map: BTreeMap<u32, i64> = BTreeMap::new();
        for _ in 0..rows_len {
            let key = 1 + (cw_splitmix(&mut seed) % u64::from(KEY_SPACE)) as u32;
            let value = (cw_splitmix(&mut seed) % 10_000) as i64 - 5_000;
            map.insert(key, value);
        }
        let mut shuffled: Vec<(u32, i64)> = map.iter().map(|(&k, &v)| (k, v)).collect();
        for index in (1..shuffled.len()).rev() {
            shuffled.swap(index, (cw_splitmix(&mut seed) % (index as u64 + 1)) as usize);
        }
        let table: D81RowTable = shuffled.into_iter().collect();

        assert_eq!(
            table.len(),
            map.len(),
            "trial {trial}: the table must hold every row the map held"
        );
        assert_eq!(
            table.iter().collect::<Vec<_>>(),
            map.iter().map(|(&k, &v)| (k, v)).collect::<Vec<_>>(),
            "trial {trial}: the table must iterate in ascending key order"
        );
        if !map.is_empty() {
            nonempty_tables += 1;
        }
        // 0 and KEY_SPACE + 1 bracket the whole staged range from outside.
        for key in 0..=KEY_SPACE + 1 {
            let want = map.get(&key).copied();
            assert_eq!(
                table.get(key),
                want,
                "trial {trial}: key {key} must answer as the map did"
            );
            if want.is_some() {
                hits += 1;
            } else {
                misses += 1;
            }
        }
    }

    assert!(
        nonempty_tables > 0 && hits > 0 && misses > 0,
        "the shapes must produce populated tables and both hits and misses, or \
         the comparison is over nothing"
    );
}

/// A table staged with a repeated key refuses at the SAME bucket, naming the
/// SAME key, that the map's `insert` refused at.
///
/// WHY THIS NEEDED A TEST OF ITS OWN. The map form decided the refusal in the
/// bucket scan — the first bucket whose key had already been seen — and the
/// sorted form decides it after reordering. Those agree only because the sort
/// is by `(key, bucket)` and the refusal is taken from each run's SECOND
/// element, minimised over runs. That is an argument, and `subject`/`index` are
/// on the wire, so it is checked against a transcription of the scan instead.
///
/// The oracle is written on `BTreeMap::insert` — the deleted code — so a
/// production change and an oracle change cannot be the same edit.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL:
///  * sorting by `key` alone with an unstable sort, which lets a run report its
///    higher bucket;
///  * reporting the run's FIRST bucket, or the first run in key order rather
///    than the one with the lowest bucket;
///  * detecting duplicates with `dedup` (which would silently drop the row and
///    publish a table the engine never held).
///
/// MUTATION RUN: the minimising `is_none_or(|(_, seen)| index < seen)` replaced
/// by `is_none()`, i.e. "the first duplicate run in KEY order". Observed:
///
/// ```text
/// assertion `left == right` failed: case 2: the refusal must name the key and
/// the bucket the scan stopped at
///   left: (Duplicate, 5, 3)
///  right: (Duplicate, 9, 2)
/// ```
///
/// Restored. Note WHICH layout caught it — `[5, 9, 9, 5, 13]`, where the lower
/// key repeats at the higher bucket.
#[test]
fn a_repeated_table_key_refuses_at_the_bucket_the_map_insert_refused_at() {
    // Bucket layouts chosen so the duplicate runs interleave: in the third the
    // LOWER key's repeat is at a HIGHER bucket than the higher key's, which is
    // the case a "first run in key order" rule gets wrong.
    let layouts: [&[u32]; 6] = [
        &[7, 9, 11, 13],
        &[7, 9, 7, 13],
        &[5, 9, 9, 5, 13],
        &[21, 4, 21, 4],
        &[3],
        // A ZERO KEY BELOW WHICH A DUPLICATE ALREADY SAT (Codex review
        // 019fde70, finding 1). The zero-key refusal is decided in the scan and
        // the duplicate is decided after it, so a scan that returned on the
        // zero key would publish `Duplicate(subject=0, index=2)` where the map
        // published `Duplicate(subject=7, index=1)`.
        &[7, 7, 0],
    ];
    let mut refusals = 0usize;
    let mut clean = 0usize;

    for (case, keys) in layouts.iter().enumerate() {
        // THE ORACLE: the deleted scan, verbatim.
        let mut seen: BTreeMap<u32, i64> = BTreeMap::new();
        let mut expected: Option<(u32, usize)> = None;
        for (index, &key) in keys.iter().enumerate() {
            if seen.insert(key, index as i64).is_some() {
                expected = Some((key, index));
                break;
            }
        }

        let fixture = stage_single_table(keys);
        let outcome = d81_hash_table_rows(fixture.table, &mut Vec::new());
        match expected {
            Some((key, index)) => {
                refusals += 1;
                let failure = outcome.expect_err("a repeated key must refuse the table");
                assert_eq!(
                    (failure.reason, failure.subject_id, failure.index),
                    (
                        D81FailureReason::Duplicate,
                        key,
                        index.min(u32::MAX as usize) as u32
                    ),
                    "case {case}: the refusal must name the key and the bucket \
                     the scan stopped at"
                );
            }
            None => {
                clean += 1;
                let snapshot = outcome.expect("a table with distinct keys is decoded");
                assert_eq!(
                    snapshot.rows.iter().map(|(key, _)| key).collect::<Vec<_>>(),
                    {
                        let mut sorted = keys.to_vec();
                        sorted.sort_unstable();
                        sorted
                    },
                    "case {case}: a clean table holds every staged key, ascending"
                );
            }
        }
    }

    assert!(
        refusals >= 3 && clean >= 2,
        "the layouts must cover both outcomes, or one arm is never taken"
    );
}

/// A duplicate at a LOW bucket outranks a malformed bucket at a HIGHER one.
///
/// THE DEFECT THIS EXISTS FOR (found in self-review of the lane FG change, and
/// present in its first draft). The old scan `return`ed at its first refusal, so
/// nothing past that bucket was ever examined. Collecting the whole scan first
/// and deciding duplicates afterwards inverts that: a malformed marker at bucket
/// 5 would refuse a table whose bucket 2 already repeated a key, publishing
/// `HeaderInvalid`/index 5 where the map form published `Duplicate`/index 2 —
/// a different reason and a different witness on the wire for the same table.
///
/// The fix is that the loop still stops at its first refusal and merely records
/// it, so the duplicate check only ever sees buckets below it. This pins the
/// ordering in both directions: the duplicate wins when it is lower, and the
/// malformed bucket wins when there is no duplicate before it.
///
/// MUTATION RUN: the `break`s in the scan restored to `return Err(...)`, i.e.
/// the first draft. Observed:
///
/// ```text
/// assertion `left == right` failed: a duplicate below the malformed bucket
/// must be the published refusal
///   left: (HeaderInvalid, 0, 5)
///  right: (Duplicate, 9, 2)
/// ```
///
/// Restored.
#[test]
fn a_low_duplicate_outranks_a_malformed_bucket_above_it() {
    // Buckets 0..=4 occupied; key 9 repeats at bucket 2; bucket 5 carries a
    // marker the header cannot license.
    let mut fixture = stage_single_table(&[7, 9, 9, 13, 21]);
    fixture.set_marker(5, 0xfe);
    let failure = d81_hash_table_rows(fixture.table, &mut Vec::new())
        .expect_err("the staged table is malformed twice over");
    assert_eq!(
        (failure.reason, failure.subject_id, failure.index),
        (D81FailureReason::Duplicate, 9, 2),
        "a duplicate below the malformed bucket must be the published refusal"
    );

    // And the control: with the repeat removed, the malformed bucket IS the
    // refusal, so the assertion above is about ORDER and not about the
    // malformed bucket being unreachable.
    let mut clean = stage_single_table(&[7, 9, 11, 13, 21]);
    clean.set_marker(5, 0xfe);
    let failure = d81_hash_table_rows(clean.table, &mut Vec::new())
        .expect_err("a malformed marker refuses on its own");
    assert_eq!(
        (failure.reason, failure.index),
        (D81FailureReason::HeaderInvalid, 5),
        "with no duplicate before it, the malformed bucket is the refusal"
    );
}

/// One staged `CSupplySystem` companion table, for the decode-path tests.
struct SingleTableFixture {
    region: FixtureRegion,
    table: usize,
    buckets_off: usize,
}

impl SingleTableFixture {
    /// Overwrite one bucket's probe marker, so a test can stage a bucket the
    /// header cannot license without disturbing the keys around it.
    fn set_marker(&mut self, slot: usize, marker: u8) {
        let offset = self.buckets_off + slot * 0x18;
        self.region.write_u8(offset + 4, marker);
    }
}

/// Stage `keys` into consecutive buckets of one table, in the given order.
///
/// The header is the reader's own: `mask + 1` a power of two comfortably above
/// the occupancy, `max_probe` 1, and the sentinel in the last slot. `count` is
/// the number of buckets occupied, which for a duplicate layout is deliberately
/// the STAGED length and not the distinct-key count — the duplicate refusal has
/// to fire before the `occupied != count` check, and staging the distinct count
/// would let that check fire first and hide the ordering.
fn stage_single_table(keys: &[u32]) -> SingleTableFixture {
    let mask: u32 = (keys.len() as u32 * 4).next_power_of_two().max(16) - 1;
    let max_probe: u8 = 1;
    let slot_count = mask + u32::from(max_probe) + 2;
    let table_bytes = slot_count as usize * 0x18;
    let buckets_off = 0x40usize;
    let mut region = FixtureRegion::new(buckets_off + table_bytes + 0x100);
    let base = region.addr();
    assert!(
        is_map_control_heap_pointer(base),
        "the staged table must satisfy the reader's heap-pointer gate"
    );
    region.write_u64(0x08, (base + buckets_off) as u64);
    region.write_u32(0x08 + 8, keys.len() as u32);
    region.write_u32(0x08 + 12, mask);
    region.write_u8(0x08 + 16, max_probe);
    for (slot, &key) in keys.iter().enumerate() {
        let bucket = buckets_off + slot * 0x18;
        region.write_u8(bucket + 4, 1);
        region.write_u32(bucket + 8, key);
        region.write_i64(bucket + 0x10, 100 + slot as i64);
    }
    region.write_u8(buckets_off + (slot_count as usize - 1) * 0x18 + 4, 0xff);
    SingleTableFixture {
        region,
        table: base,
        buckets_off,
    }
}
