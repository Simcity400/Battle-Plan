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
    let read = read_compact_rail_replay_state(state, 0, cache);
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
        gate.1.contains("return Err(failure(D81FailureReason::ClosureChanged));"),
        "and a controller that moved must refuse the frame"
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
            "\n            let closing_selector_ptr = match d81_read_u64_pair(selector_route.province_slot) {"
        ),
        "the read that survives must be the SOURCE close -- the later of the \
         two -- or the deduplicated span is a prefix of the one it replaces"
    );
    let after_surviving_read = walk
        .split_once(
            "let closing_source_province_ptr = match d81_read_u64_pair(source_province_slot) {",
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
            "if selector_slot_is_source_slot {\n                    return fail(status_flags, selector_close_read_failure(read_failure));"
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
/// process-global, so this must not run beside other tests -- which is why its
/// only caller is `#[ignore]`d.
fn guarded_reads(body: impl FnOnce()) -> u64 {
    const UNUSED_DOMAIN_ID: u16 = 219;
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
        d81_read_u64_pair(heap.at(0x00)).expect("the staged pointer is readable");
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

/// The index the fold built BEFORE the restriction: every province of every
/// related country's table, unconditionally.
///
/// This is an INDEPENDENT ORACLE, deliberately written out here rather than
/// obtained by calling the production fold with a permissive argument. A test
/// whose two arms are the same function in two configurations cannot fail when
/// that function is wrong in both; this one can. It is a faithful transcription
/// of the loop body that `d81_merge_foreign_values` replaced.
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

/// The restricted fold answers every query the unrestricted one answered, with
/// the same value and the same per-province order.
///
/// THE SHAPES ARE THE POINT. `d81_merge_foreign_values` picks between driving
/// from the needed set and driving from the table by comparing their sizes, so
/// a differential that only ever exercised one side would leave the other
/// unproved. These shapes deliberately straddle the threshold: needed sets from
/// 0 to 200 against tables from 1 to 400, over a province space small enough
/// that the two sets genuinely overlap.
///
/// TWO MUTATION RUNS, because the test makes two separate claims and each needs
/// its own falsification. Both were run; the excerpts below are abridged only
/// where the harness prints the full thread path.
///
/// 1. The fold keeps too MUCH -- `queried_provinces.contains(&province_id)`
///    deleted from the table-driven branch:
///
/// ```text
/// thread '...the_restricted_foreign_fold_agrees_with_the_unrestricted_one_on_every_shape'
/// panicked at collector\src\attach_log\tests\d81_frame_cost.rs:1900:13:
/// trial 1: province 96 was indexed but never queried
/// ```
///
/// 2. The fold keeps too LITTLE -- the lookup branch given an `if value > 0`
///    guard, so it silently drops entries it should carry:
///
/// ```text
/// assertion `left == right` failed: trial 0: the restricted fold must publish
/// exactly the unrestricted fold's entries for every queried province (province 58)
///   left: []
///  right: [D81ForeignValue { raw_handle: 2, equivalence_id: 1002, value: -94 }]
/// ```
///
/// Both restored. Note what does NOT fail: replacing the
/// `queried_provinces.len() < table_rows.len()` test by `true` forces the
/// lookup branch onto every shape and the test still passes. That is the branch
/// selection doing its job -- the two sides are meant to agree -- and it is why
/// the branch choice is a performance decision and not a semantic one.
#[test]
fn the_restricted_foreign_fold_agrees_with_the_unrestricted_one_on_every_shape() {
    const PROVINCE_SPACE: u32 = 512;
    let mut seed = 0x0D81_0C00_u64;
    let mut shapes_seen = (0usize, 0usize);

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

        let mut restricted: BTreeMap<u32, Vec<D81ForeignValue>> = BTreeMap::new();
        for (raw_handle, equivalence_id, rows) in &tables {
            if queried.len() < rows.len() {
                shapes_seen.0 += 1;
            } else {
                shapes_seen.1 += 1;
            }
            d81_merge_foreign_values(
                &mut restricted,
                rows,
                &queried,
                *raw_handle,
                *equivalence_id,
            );
        }

        let unrestricted = unrestricted_foreign_index(&tables);
        for province_id in &queried {
            let expected = unrestricted.get(province_id).cloned().unwrap_or_default();
            let actual = restricted.get(province_id).cloned().unwrap_or_default();
            assert_eq!(
                actual, expected,
                "trial {trial}: the restricted fold must publish exactly the \
                 unrestricted fold's entries for every queried province \
                 (province {province_id})"
            );
        }
        // And NOTHING outside the queried set is indexed -- that is the whole
        // saving, and a fold that quietly kept indexing everything would still
        // pass the equality above.
        for province_id in restricted.keys() {
            assert!(
                queried.contains(province_id),
                "trial {trial}: province {province_id} was indexed but never queried"
            );
        }
    }

    assert!(
        shapes_seen.0 > 0 && shapes_seen.1 > 0,
        "the shapes must exercise BOTH branches of the fold, saw {shapes_seen:?}"
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

/// Every query the hub can put to the index returns what it returned before.
///
/// `maximum_excluding_controller` is the ONLY reader of the index, and its
/// answer is what lands in a published row's `foreign_max_raw`. Comparing the
/// maps alone would leave open the possibility of a difference the predicate
/// happens to expose; this drives the production predicate over the RESTRICTED
/// index against [`reference_maximum`] over the UNRESTRICTED one, for every
/// controller identity the staged data can produce -- including the two
/// zero-handle special cases the filter singles out.
///
/// MUTATION RUN: the `&& entry.value > 0` term deleted from
/// `maximum_excluding_controller`'s filter -- the exact mutation the first,
/// circular draft of this test could NOT see. Observed:
///
/// ```text
/// assertion `left == right` failed: trial 0: province 0 controller (4, 1002)
/// must report the same foreign maximum before and after the restriction
///   left: -13
///  right: 0
/// ```
///
/// Restored.
#[test]
fn the_restricted_index_reports_the_same_foreign_maximum_for_every_query() {
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

        let mut restricted_map: BTreeMap<u32, Vec<D81ForeignValue>> = BTreeMap::new();
        for (raw_handle, equivalence_id, rows) in &tables {
            d81_merge_foreign_values(
                &mut restricted_map,
                rows,
                &queried,
                *raw_handle,
                *equivalence_id,
            );
        }
        let restricted = D81ForeignSource {
            values_by_province: restricted_map,
            witnesses: Vec::new(),
            // Not under test here: `related_count` prices a fold this assertion
            // does not run, and `maximum_excluding_controller` never reads it.
            ..D81ForeignSource::default()
        };
        let unrestricted = unrestricted_foreign_index(&tables);

        for &province_id in &queried {
            for (controller_raw_handle, controller_equivalence_id, _) in &tables {
                assert_eq!(
                    restricted.maximum_excluding_controller(
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
                     report the same foreign maximum before and after the restriction"
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

/// The production read over staged memory publishes, for every queried
/// province, exactly what the unrestricted fold published -- and the same
/// witnesses, in the same order.
///
/// THIS IS THE READ, NOT A MODEL OF IT. `d81_read_foreign_source` is called
/// exactly as the hub calls it, over real memory, so the witnesses it mints,
/// the order it mints them in, and the table-cache population it performs are
/// all the production ones. The oracle is the independent
/// `unrestricted_foreign_index` transcription of the loop this change replaced.
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
fn the_staged_foreign_read_publishes_the_unrestricted_entries_and_the_same_witnesses() {
    let mut seed = 0xF0_1E_16_4Eu64;
    // Shapes on both sides of the fold's branch threshold.
    for &(related_count, rows_per_table, province_space, queried_len) in &[
        (4u32, 12u32, 64u32, 40usize),
        (6, 200, 512, 24),
        (3, 4, 32, 30),
        (5, 64, 128, 64),
    ] {
        let fixture = stage_foreign_fixture(related_count, rows_per_table, province_space, &mut seed);
        let queried: BTreeSet<u32> = (0..queried_len)
            .map(|_| 1 + (cw_splitmix(&mut seed) % u64::from(province_space)) as u32)
            .collect();

        let mut table_cache = BTreeMap::new();
        let mut scratch = Vec::new();
        let source = d81_read_foreign_source(
            fixture.supply_system,
            0,
            8,
            &fixture.authority,
            &mut table_cache,
            &mut scratch,
            &queried,
            &D81PhaseAccum::default(),
        )
        .expect("the staged foreign tree is complete and readable");

        let oracle = unrestricted_foreign_index(&fixture.staged_rows);
        for province_id in &queried {
            let expected = oracle.get(province_id).cloned().unwrap_or_default();
            let actual = source
                .values_by_province
                .get(province_id)
                .cloned()
                .unwrap_or_default();
            assert_eq!(
                actual, expected,
                "the staged read must publish the unrestricted entries for \
                 queried province {province_id}"
            );
        }
        for province_id in source.values_by_province.keys() {
            assert!(
                queried.contains(province_id),
                "province {province_id} was indexed but never queried"
            );
        }

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
        let observed: Vec<(usize, Vec<u8>)> = source
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
            source.witnesses.iter().all(|witness| witness.unchanged()),
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

/// The wire bytes d81 publishes are identical, byte for byte.
///
/// The two previous tests compare the index and the predicate. This one closes
/// the loop the domain actually cares about: it assembles
/// `RailHubSimulationState` rows from both indexes exactly as the hub's
/// row-emission loop does, encodes them through the production payload encoder,
/// and compares the resulting payloads as bytes. A semantic drift anywhere in
/// the fold reaches `foreign_max_raw`, and `foreign_max_raw` is in the payload.
///
/// WHICH BRANCH THIS FIXTURE EXERCISES (corrected against Codex review
/// 019fd618-5241-73b0-af78-470bbee000d6, finding 3). 48 queried keys against
/// 48-row tables makes `queried_provinces.len() < table_rows.len()` false, so
/// this test drives the TABLE-driven branch. The lookup branch is covered by
/// `the_restricted_foreign_fold_agrees_with_the_unrestricted_one_on_every_shape`
/// and by the staged read above, whose shapes straddle the threshold; the
/// mutation below is therefore applied to the branch this fixture actually
/// takes rather than to the one the first draft of this note named.
///
/// MUTATION RUN: `push(province_id, value)` in the fold's table-driven branch
/// given `value.saturating_add(1)`, i.e. the smallest possible drift in the one
/// field the fold decides. Observed:
///
/// ```text
/// assertion `left == right` failed: the restricted fold must produce a
/// byte-identical d81 payload (offset, restricted byte, unrestricted byte)
///   left: Some((608, 58, 57))
///  right: None
/// ```
///
/// Restored. Offset 608 is inside a hub-simulation continuation row, which is
/// where `foreign_max_raw` is encoded -- the drift reached the wire, and the
/// test names the byte.
/// `hub_query_probes` is `Σ_POV rows(POV) x related(POV)`, and the caller can
/// only form that product if the foreign read hands back the POV's related
/// count. The count therefore has to survive out of
/// [`d81_read_foreign_source`] as a value, not be re-derived at the call site
/// from anything the wire already sums.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: leaving `related_count` at its
/// `Default` (the field was added to a struct that is constructed with `..`
/// in three places, so this is the live failure mode) makes every
/// `hub_query_probes` zero, and the counter that exists to decide between the
/// eager and the query-time fold silently answers "the query-time fold is
/// free". Deriving it from `seen_handles.len()` instead would also pass today
/// and is not what the header authenticated, so the assertion is written
/// against the staged related count.
#[test]
fn the_foreign_read_reports_the_related_count_its_probes_are_priced_from() {
    let mut seed = 0x0081_C0DEu64;
    let fixture = stage_foreign_fixture(6, 48, 96, &mut seed);
    let queried: BTreeSet<u32> = (1..=48u32).collect();

    let source = d81_read_foreign_source(
        fixture.supply_system,
        0,
        8,
        &fixture.authority,
        &mut BTreeMap::new(),
        &mut Vec::new(),
        &queried,
        &D81PhaseAccum::default(),
    )
    .expect("the staged foreign tree is complete and readable");

    assert_eq!(
        source.related_count, 6,
        "the POV's related-country count must reach the caller that prices its probes"
    );
    // The product the production call site forms. Stated here so the counter's
    // meaning is pinned to an example rather than only to its declaration: 48
    // queried provinces answered against 6 related tables is 288 probes, which
    // is what a query-time fold would pay for this POV.
    assert_eq!(queried.len() as u64 * source.related_count, 288);
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
///  * charging a constant instead of `self.related_count` — the totals stop
///    being a sum of per-POV products, which is the only reason the unit is on
///    the wire rather than computed offline (here that shows up as 12 or 6
///    instead of 14);
///  * hoisting the charge back to the caller, which no longer type-checks
///    without also removing it from the query, i.e. the first mutation above.
#[test]
fn the_probe_charges_for_queries_performed_not_for_routes_planned() {
    let phases = D81PhaseAccum::default();
    let source = |related_count: u64| D81ForeignSource {
        related_count,
        ..D81ForeignSource::default()
    };

    // A POV with many related countries answering few routes, then one with the
    // opposite shape. DELIBERATELY ASYMMETRIC: the correlated sum is 14 while
    // the uncorrelated estimate anyone would form from the published totals
    // (calls x related_total / povs = 6 x 6 / 2 = 18) is not, and telling those
    // two apart is the whole reason the counter exists.
    let wide = source(5);
    for _ in 0..2 {
        wide.maximum_excluding_controller(&phases, 1, 0, 0);
    }
    let narrow = source(1);
    for _ in 0..4 {
        narrow.maximum_excluding_controller(&phases, 1, 0, 0);
    }
    // The POV that refused before its first query -- one of the four failure
    // paths between a route and its query. It is never asked, so it pays
    // nothing, which is precisely what the pre-loop charge got wrong.
    let _aborted = source(9);

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
}

/// A POV with no related countries folds nothing, so a query-time fold would
/// probe nothing — and the counter must say ZERO rather than inherit a count
/// from the header it never used.
///
/// WHAT PRODUCTION CHANGE MAKES THIS FAIL: setting `related_count` from
/// `country_count` (8 here) or from the related vector's CAPACITY rather than
/// from its authenticated count. Either reads plausibly and would charge this
/// POV for probes no query-time fold would ever perform, biasing the whole
/// decision toward keeping the index.
#[test]
fn a_pov_with_no_related_countries_prices_no_probes() {
    let mut seed = 0x0081_0000u64;
    let fixture = stage_foreign_fixture(0, 48, 96, &mut seed);
    let queried: BTreeSet<u32> = (1..=48u32).collect();

    let source = d81_read_foreign_source(
        fixture.supply_system,
        0,
        8,
        &fixture.authority,
        &mut BTreeMap::new(),
        &mut Vec::new(),
        &queried,
        &D81PhaseAccum::default(),
    )
    .expect("a POV with an empty related vector is a complete read, not a refusal");

    assert_eq!(source.related_count, 0);
    assert!(source.values_by_province.is_empty());
}

#[test]
fn the_restricted_fold_produces_a_byte_identical_d81_payload() {
    use compact_contract::rail_replay::{
        encode_payload_with_hub_simulation, RailHubSimulationState, RailNeighborState,
        RailProvinceState,
    };

    let mut seed = 0xB17E_1DE5u64;
    let fixture = stage_foreign_fixture(6, 48, 96, &mut seed);
    let queried: BTreeSet<u32> = (1..=48u32).collect();

    let mut table_cache = BTreeMap::new();
    let mut scratch = Vec::new();
    let restricted = d81_read_foreign_source(
        fixture.supply_system,
        0,
        8,
        &fixture.authority,
        &mut table_cache,
        &mut scratch,
        &queried,
        &D81PhaseAccum::default(),
    )
    .expect("the staged foreign tree is complete and readable");
    let unrestricted = D81ForeignSource {
        values_by_province: unrestricted_foreign_index(&fixture.staged_rows),
        witnesses: Vec::new(),
        ..D81ForeignSource::default()
    };

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

    let rows_from = |source: &D81ForeignSource| -> Vec<RailHubSimulationState> {
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
                    foreign_max_raw: source.maximum_excluding_controller(
                        &D81PhaseAccum::default(),
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

    let restricted_rows = rows_from(&restricted);
    let unrestricted_rows = rows_from(&unrestricted);

    let encode = |rows: &[RailHubSimulationState]| {
        encode_payload_with_hub_simulation(&provinces, rows, 0, 0, provinces.len() as u32, 8)
            .expect("the staged snapshot encodes")
    };
    let restricted_payload = encode(&restricted_rows);
    let unrestricted_payload = encode(&unrestricted_rows);

    // THE BYTES ARE CHECKED FIRST, and reported as ONE offset rather than as two
    // whole buffers. `assert_eq!` on the payloads (or on the row vectors) prints
    // both sides in full -- a 48 KB dump that says "they differ" and nothing
    // about WHERE, which is useless as recorded evidence for a byte-identity
    // claim. The first differing offset is the diagnostic that identifies the
    // field.
    assert_eq!(
        restricted_payload.len(),
        unrestricted_payload.len(),
        "the restricted fold must produce a d81 payload of the same length"
    );
    let first_difference = restricted_payload
        .iter()
        .zip(&unrestricted_payload)
        .position(|(left, right)| left != right);
    assert_eq!(
        first_difference.map(|at| (at, restricted_payload[at], unrestricted_payload[at])),
        None,
        "the restricted fold must produce a byte-identical d81 payload \
         (offset, restricted byte, unrestricted byte)"
    );
    // Only now the rows, as the finer-grained statement behind the bytes.
    assert_eq!(
        restricted_rows, unrestricted_rows,
        "the restricted fold must produce identical hub simulation rows"
    );
    // Non-vacuous: the payload has to actually carry a non-zero foreign
    // maximum, or byte-identity would be a statement about zeros.
    assert!(
        restricted_rows.iter().any(|row| row.foreign_max_raw > 0),
        "the fixture must produce at least one non-zero foreign maximum"
    );
}

/// Price the foreign-value fold at t27's shape, before and after the
/// restriction.
///
/// WHY A BENCH AND NOT AN ASSERTION. `#[ignore]` on purpose, for the reason
/// `d81_guarded_read_cost_model` gives: this is a measurement, and a wall-clock
/// gate in this tree flakes under lane load. Run it deliberately:
///
/// ```text
/// cargo test -p collector --release d81_foreign_fold_cost_model -- --ignored --nocapture
/// ```
///
/// TWO OF THE THREE SHAPE NUMBERS ARE t27's; THE THIRD IS A PROXY, and the
/// difference matters (corrected against Codex review
/// 019fd618-5241-73b0-af78-470bbee000d6, findings 4 and, in the second round,
/// 1). MEASURED, decoded from t27's own d81 payloads: 93 POV countries per
/// frame, and 11,570 hub output rows per frame. ASSUMED: that each POV is
/// related to the other 92 — the related-country count is not on the wire.
/// PROXY: `QUERIED_PER_POV = 124` is the per-POV MEAN of output rows,
/// 11,570/93 rounded down from 124.41. It is NOT the p50 (that is 83), and it
/// is NOT a measured count of distinct queried provinces, which the wire never
/// carried at all.
///
/// THE DIRECTION OF THIS PROXY'S ERROR IS UNKNOWN, and saying otherwise was the
/// last thing wrong with this note (Codex review
/// 019fd618-5241-73b0-af78-470bbee000d6, third round). An earlier draft called
/// 124 an upper bound and the speedup columns "no better than this". Neither
/// follows: 124 is a rounded-down mean rather than any kind of bound, per-POV
/// output rows run from 1 to 365 so 124 bounds no individual POV's set, and
/// deduplicating the draws only relates this bench to a hypothetical
/// exact-124 model, not to the real distribution. Read the columns as a
/// measurement of THIS shape, and read the shape as a guess.
///
/// The bench draws 124 province ids and collects them into a `BTreeSet`, so the
/// queried set it actually uses is 124 DRAWS and fewer distinct ids than that.
///
/// `d81_phase_hub_queried_provinces` replaces the 124 proxy on the next capture,
/// and `d81_phase_hub_related_visits` replaces the 92 assumption; both are units
/// this lane put on the wire precisely because neither number could be
/// recovered from anything d81 already published. The only
/// free parameter is `R`, the number of provinces in one country's supply
/// table, which is not on the wire; the sweep brackets it rather than guessing
/// one value, and the per-frame cost is what the sweep reports.
///
/// WHAT IS AND IS NOT MEASURED. This times the fold and nothing else: no
/// guarded reads, no witnesses, no hash-table decode. Those are unchanged by
/// this lane and are measured elsewhere (`d81_guarded_read_cost_model`). The
/// number to carry to the wire is the DIFFERENCE between the two arms, which is
/// the part of `d81_phase_hub_us` this change removes.
///
/// ARMS INTERLEAVED, MEDIAN OF SEVEN, same discipline as the guarded-read
/// model: running one arm's trials and then the other confounds the arm with
/// anything that drifts across the run.
#[test]
#[ignore]
fn d81_foreign_fold_cost_model() {
    use std::time::Instant;

    // t27, decoded from the capture's own d81 payloads.
    const POV_COUNT: usize = 93;
    const RELATED_PER_POV: usize = 92;
    const QUERIED_PER_POV: usize = 124;
    const PROVINCE_SPACE: u32 = 13_413; // provinces_visited, t27
    const TRIALS: usize = 7;

    println!(
        "d81 foreign-value fold, per FRAME \
         ({POV_COUNT} POVs measured, {RELATED_PER_POV} related assumed, \
         {QUERIED_PER_POV} query draws as a proxy of unknown error direction)"
    );
    println!(
        "{:>8}  {:>14}  {:>14}  {:>9}  {:>14}",
        "R", "unrestricted", "restricted", "speedup", "entries kept"
    );

    // THE LOW END IS THE INTERESTING END, and that is a finding rather than a
    // convenience. The unrestricted arm costs 49 ms/frame at R=50 -- MORE than
    // the whole 35,317 us `d81_phase_hub_us` t27 actually measured -- so the
    // real table size must sit BELOW 50, and a sweep that started there would
    // have been extrapolating above the observed point instead of bracketing
    // it. These values bracket it from underneath.
    for rows_per_table in [5usize, 10, 20, 30, 40, 50, 100, 250] {
        let mut seed = 0xC0_57_0D_81u64 ^ (rows_per_table as u64);
        // One table per related country, shared across POVs exactly as the
        // production `foreign_table_cache` shares them within a frame.
        let tables: Vec<(u32, u32, BTreeMap<u32, i64>)> = (0..RELATED_PER_POV)
            .map(|index| {
                let raw_handle = index as u32 + 1;
                let rows = (0..rows_per_table)
                    .map(|_| {
                        (
                            1 + (cw_splitmix(&mut seed) % u64::from(PROVINCE_SPACE)) as u32,
                            (cw_splitmix(&mut seed) % 4_000) as i64 - 1_000,
                        )
                    })
                    .collect::<BTreeMap<u32, i64>>();
                (raw_handle, raw_handle + 1000, rows)
            })
            .collect();
        // A different queried set per POV, as in production: every POV's routes
        // reach a different slice of the map. `QUERIED_PER_POV` DRAWS, collected
        // into a set, so the realised set is smaller than the draw count. How
        // that compares to a real POV's queried set is unknown -- see the proxy
        // note in this test's doc.
        let queried_per_pov: Vec<BTreeSet<u32>> = (0..POV_COUNT)
            .map(|_| {
                (0..QUERIED_PER_POV)
                    .map(|_| 1 + (cw_splitmix(&mut seed) % u64::from(PROVINCE_SPACE)) as u32)
                    .collect()
            })
            .collect();

        let unrestricted_frame = || {
            let mut kept = 0usize;
            for _pov in 0..POV_COUNT {
                let index = unrestricted_foreign_index(&tables);
                kept += index.values().map(Vec::len).sum::<usize>();
            }
            kept
        };
        let restricted_frame = || {
            let mut kept = 0usize;
            for queried in &queried_per_pov {
                let mut index: BTreeMap<u32, Vec<D81ForeignValue>> = BTreeMap::new();
                for (raw_handle, equivalence_id, rows) in &tables {
                    d81_merge_foreign_values(
                        &mut index,
                        rows,
                        queried,
                        *raw_handle,
                        *equivalence_id,
                    );
                }
                kept += index.values().map(Vec::len).sum::<usize>();
            }
            kept
        };

        let mut unrestricted_ns = Vec::new();
        let mut restricted_ns = Vec::new();
        let mut unrestricted_kept = 0usize;
        let mut restricted_kept = 0usize;
        for trial in 0..TRIALS {
            // Interleaved, and the order flipped every other trial so neither
            // arm systematically runs on a warmer cache than the other.
            if trial % 2 == 0 {
                let start = Instant::now();
                unrestricted_kept = unrestricted_frame();
                unrestricted_ns.push(start.elapsed().as_nanos() as u64);
                let start = Instant::now();
                restricted_kept = restricted_frame();
                restricted_ns.push(start.elapsed().as_nanos() as u64);
            } else {
                let start = Instant::now();
                restricted_kept = restricted_frame();
                restricted_ns.push(start.elapsed().as_nanos() as u64);
                let start = Instant::now();
                unrestricted_kept = unrestricted_frame();
                unrestricted_ns.push(start.elapsed().as_nanos() as u64);
            }
        }
        unrestricted_ns.sort_unstable();
        restricted_ns.sort_unstable();
        let before = unrestricted_ns[TRIALS / 2];
        let after = restricted_ns[TRIALS / 2];
        println!(
            "{rows_per_table:>8}  {:>11.2} ms  {:>11.2} ms  {:>8.1}x  {:>6} -> {:<6}",
            before as f64 / 1e6,
            after as f64 / 1e6,
            before as f64 / after.max(1) as f64,
            unrestricted_kept,
            restricted_kept,
        );
    }
    println!(
        "\nThe DIFFERENCE between the two arms is the share of d81_phase_hub_us \
         (35,317 us p50 in t27) this change removes."
    );
}
