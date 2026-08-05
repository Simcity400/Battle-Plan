//! The frame's ONE map-control topology build, and the fail-closed rule that
//! decides which readers may publish it.
//!
//! `build_compact_map_control_topology` walks every province and every state
//! the province array names. Seven readers wanted that walk and each did its
//! own; the owner approved (2026-08-05) doing it once per frame and sharing it
//! on a condition that leaves the hour guarantee where it was: a consumer may
//! publish the shared build ONLY if the authenticated clock pair observed
//! across that build lies inside the consumer's own `ReaderClockBracketGuard`
//! and inside the frame's own hour label.
//!
//! These tests answer two different questions and are deliberately split that
//! way:
//!
//!  * [`shared_map_topology_verdict`] is a pure function of four observations,
//!    so the refusal cases can be stated exactly, including the ones a live
//!    engine would produce only by accident (a boundary landing mid-walk).
//!  * The cache and the d18 reader are driven over a STAGED FULL-SIZE MAP with
//!    a real authenticated scheduler clock, so the wiring -- which bracket is
//!    consulted, which rows come back, what the counters say -- is proved
//!    against production code rather than against a restatement of it.
//!
//! What this file does NOT prove is the offsets: the fixture writes through the
//! same constants the reader reads through. See the note at the head of
//! `state_fixture.rs`.

use super::state_fixture::{fixture_game_hour_raw, CollectorStateFixture};
use super::*;

// ---------------------------------------------------------------------
// The predicate
// ---------------------------------------------------------------------

/// An hour the calendar admits, and whose neighbours it admits too -- the
/// straddle and mismatch cases below are stated as `LABEL_HOUR ± 1` and would
/// otherwise be testing `FrameLabelUnusable` by accident.
fn label_hour() -> i32 {
    let hour = fixture_game_hour_raw();
    for candidate in [hour - 1, hour, hour + 1] {
        assert!(
            frame_label_is_a_real_hour(candidate),
            "the fixture hour and its neighbours must all be real hours"
        );
    }
    hour
}

/// A build that began after the consumer's bracket opened and stayed inside the
/// frame's hour, which is the only shape that publishes.
fn clean_pair() -> (ClockObservation, ClockObservation, ClockObservation) {
    let consumer = clock_obs(label_hour(), 10);
    let opening = clock_obs(label_hour(), 11);
    let closing = clock_obs(label_hour(), 12);
    (consumer, opening, closing)
}

/// MUTATION RUN: `shared_map_topology_verdict`'s body replaced by
/// `SharedMapTopologyVerdict::Accept`. Observed in `--release`:
///
/// ```text
/// assertion `left == right` failed
///   left: Accept
///  right: OutsideConsumerBracket
/// ```
///
/// Restored.
#[test]
fn the_acceptance_check_publishes_only_a_build_bracketed_by_its_own_consumer() {
    let (consumer, opening, closing) = clean_pair();
    assert_eq!(
        shared_map_topology_verdict(opening, closing, Some(consumer), label_hour()),
        SharedMapTopologyVerdict::Accept,
        "a build that began inside the consumer's bracket, in the frame's own \
         hour, is exactly what the owner approved sharing"
    );

    // The build started BEFORE this consumer's bracket opened. The consumer
    // cannot claim the topology was observed inside its own read window, even
    // though every hour involved is the label's.
    let late_consumer = clock_obs(label_hour(), opening.ordinal + 1);
    assert_eq!(
        shared_map_topology_verdict(opening, closing, Some(late_consumer), label_hour()),
        SharedMapTopologyVerdict::OutsideConsumerBracket
    );

    // A consumer that holds no bracket at all has nothing to compare against.
    assert_eq!(
        shared_map_topology_verdict(opening, closing, None, label_hour()),
        SharedMapTopologyVerdict::NoConsumerBracket
    );

    // The hour changed while the shared walk ran, so its rows mix two hours --
    // the same defect `BracketHourVerdict::Straddled` names for a domain read.
    assert_eq!(
        shared_map_topology_verdict(
            opening,
            clock_obs(label_hour() + 1, closing.ordinal),
            Some(consumer),
            label_hour()
        ),
        SharedMapTopologyVerdict::BuildStraddledHour
    );
}

/// The label half of the rule, in both directions.
///
/// MUTATION RUN: the `consumer_opening.hour != game_hour_raw` refusal deleted
/// from `shared_map_topology_prebuild_admission`. Observed in `--release`:
///
/// ```text
/// assertion `left == right` failed: a consumer that opened in another hour must not inherit a clean build
///   left: Accept
///  right: HourLabelMismatch
/// ```
///
/// The same mutation ALSO reddens
/// `an_hour_label_mismatch_refuses_the_share_before_the_walk_is_paid_for`, on
/// the pre-build half (`left: 1, right: 0` builds). Restored.
#[test]
fn a_build_or_a_consumer_outside_the_frames_labelled_hour_never_publishes() {
    let (consumer, opening, closing) = clean_pair();

    // The walk ran wholly inside the NEXT hour. A bracket alone cannot see
    // this -- opening == closing holds just as firmly -- and only the frame's
    // label tells the two apart.
    assert_eq!(
        shared_map_topology_verdict(
            clock_obs(label_hour() + 1, opening.ordinal),
            clock_obs(label_hour() + 1, closing.ordinal),
            Some(clock_obs(label_hour(), consumer.ordinal)),
            label_hour()
        ),
        SharedMapTopologyVerdict::HourLabelMismatch,
        "a build observed wholly in H+1 must not be published under H"
    );

    // ... and the consumer's own end of the interval. Its rows will be
    // published over the window from ITS opening to ITS closing.
    assert_eq!(
        shared_map_topology_verdict(
            opening,
            closing,
            Some(clock_obs(label_hour() - 1, consumer.ordinal)),
            label_hour()
        ),
        SharedMapTopologyVerdict::HourLabelMismatch,
        "a consumer that opened in another hour must not inherit a clean build"
    );

    // The NO_CLOCK sentinel, and every other label the calendar refuses. The
    // rule cannot be applied, so it refuses rather than skipping: such a frame
    // is already failing on its own account.
    assert_eq!(
        shared_map_topology_verdict(
            clock_obs(0, opening.ordinal),
            clock_obs(0, closing.ordinal),
            Some(clock_obs(0, consumer.ordinal)),
            0
        ),
        SharedMapTopologyVerdict::FrameLabelUnusable
    );
}

/// WHO OWES WHICH PROOF.
///
/// The closing observation, the ordinal comparison and the array re-check all
/// bridge the gap between a walk and a consultation that did not perform it.
/// The consumer that DID perform it has no gap: its own guard brackets the walk
/// from both sides, and `HOUR_LABEL_MISMATCH_POLICY = Refuse` already decides
/// its hour from that bracket -- which is exactly the guarantee the pre-cache
/// code had. Demanding the extra proofs there is not caution; it forces a
/// second traversal that can fail where the first succeeded, losing the domain
/// its hour.
///
/// MUTATION RUN: `shared_map_topology_claim`'s `this_consumer_built` early
/// return deleted. Observed in `--release`:
///
/// ```text
/// assertion `left == right` failed: the consumer that performed the walk owes no proof it did not owe before the cache
///   left: Refused(BuildCloseUnreadable)
///  right: Builder
/// ```
///
/// Restored.
#[test]
fn only_a_consumer_that_did_not_perform_the_walk_owes_the_extra_proofs() {
    let (consumer, opening, closing) = clean_pair();

    // A build whose closing read failed authentication cannot say whether it
    // straddled, so nobody who did not perform it may take it...
    assert_eq!(
        shared_map_topology_claim(false, opening, None, Some(consumer), label_hour()),
        SharedMapTopologyClaim::Refused(SharedMapTopologyVerdict::BuildCloseUnreadable)
    );
    // ... and the consumer that DID perform it still may: it is holding the
    // result of the same call it would have made itself.
    assert_eq!(
        shared_map_topology_claim(true, opening, None, Some(consumer), label_hour()),
        SharedMapTopologyClaim::Builder,
        "the consumer that performed the walk owes no proof it did not owe \
         before the cache"
    );

    // The clean shared case is still only PENDING: the province array must
    // also still be the live one, and that is not a clock question.
    assert_eq!(
        shared_map_topology_claim(false, opening, Some(closing), Some(consumer), label_hour()),
        SharedMapTopologyClaim::SharedPendingArrayCheck
    );
    // Every clock refusal still refuses a non-performer.
    let late_consumer = clock_obs(label_hour(), opening.ordinal + 1);
    assert_eq!(
        shared_map_topology_claim(
            false,
            opening,
            Some(closing),
            Some(late_consumer),
            label_hour()
        ),
        SharedMapTopologyClaim::Refused(SharedMapTopologyVerdict::OutsideConsumerBracket)
    );
}

// ---------------------------------------------------------------------
// The staged map
// ---------------------------------------------------------------------

const MAP_GS: usize = 0x0000;
pub(super) const MAP_PROVINCE_ARRAY: usize = 0x1000;
const MAP_STATES: usize = 0x18000;
const MAP_PROVINCES: usize = 0xC0000;
const MAP_STATE_STRIDE: usize = 0xD0;
const MAP_PROVINCE_STRIDE: usize = 0x190;
/// Above `MIN_USABLE_PROVINCE_ROWS` (10,000) and `MIN_USABLE_STATE_ROWS`
/// (1,000): a smaller map is refused by the builder before the walk this lane
/// is about ever runs.
const MAP_PROVINCE_COUNT: u32 = 10_001;
const MAP_STATE_COUNT: u32 = 1_200;
const MAP_HEAP_LEN: usize =
    MAP_PROVINCES + (MAP_PROVINCE_COUNT as usize + 1) * MAP_PROVINCE_STRIDE + 0x1000;

/// A full-size `CProvince*` array, the `CState`s those provinces point at, and
/// the authenticated scheduler clock the brackets read.
///
/// The clock matters as much as the map here: without the CGameState singleton
/// slot and the `gs+0x468` word agreeing, `read_authenticated_scheduler_clock`
/// returns `None`, every bracket opens `ClockUnreadable`, and the cache refuses
/// every consumer for a reason that has nothing to do with what is being asked.
pub(super) fn staged_map_fixture() -> CollectorStateFixture {
    let mut fixture = CollectorStateFixture::new(
        &[compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA],
        0x1000,
        MAP_HEAP_LEN,
    );
    fixture.stage_scheduler_clock_at(MAP_GS);
    let module_base = fixture.module.module_base();

    let province_array_addr = fixture.heap.at(MAP_PROVINCE_ARRAY) as u64;
    fixture
        .heap
        .write_u64(MAP_GS + CGAMESTATE_CPROVINCE_ARRAY, province_array_addr);
    fixture
        .heap
        .write_u32(MAP_GS + CGAMESTATE_CPROVINCE_COUNT, MAP_PROVINCE_COUNT);

    let state_vtable = (module_base + STATE_VTABLE_RVA) as u64;
    for state_id in 1..=MAP_STATE_COUNT {
        let offset = MAP_STATES + state_id as usize * MAP_STATE_STRIDE;
        fixture.heap.write_u64(offset, state_vtable);
        fixture.heap.write_u32(offset + STATE_OFF_SID, state_id);
        // Owner and controller differ, and differ per state, so a reader that
        // borrowed one field's value for the other -- or one state's row for
        // another's -- cannot pass by coincidence.
        fixture
            .heap
            .write_u32(offset + STATE_OFF_OWNER, state_id % 97 + 1);
        fixture
            .heap
            .write_u32(offset + STATE_OFF_CTRL, state_id % 89 + 1);
    }

    let province_vtable = (module_base + CPROVINCE_VTABLE_RVA) as u64;
    for province_id in 1..MAP_PROVINCE_COUNT {
        let offset = MAP_PROVINCES + province_id as usize * MAP_PROVINCE_STRIDE;
        let state_id = (province_id - 1) % MAP_STATE_COUNT + 1;
        let state_addr = fixture
            .heap
            .at(MAP_STATES + state_id as usize * MAP_STATE_STRIDE) as u64;
        let province_addr = fixture.heap.at(offset) as u64;
        fixture.heap.write_u64(offset, province_vtable);
        fixture
            .heap
            .write_u32(offset + CPROVINCE_OFF_ID, province_id);
        fixture
            .heap
            .write_u64(offset + CPROVINCE_OFF_STATE, state_addr);
        fixture
            .heap
            .write_u32(offset + CPROVINCE_OFF_CONTROLLER, province_id % 83 + 1);
        fixture.heap.write_u64(
            MAP_PROVINCE_ARRAY + province_id as usize * 8,
            province_addr,
        );
    }
    fixture
}

/// A lane tracker whose clock roots are the fixture's, so a bracket opened
/// against it reads the same authenticated word the cache does.
pub(super) fn lanes_for(state: &CollectorState) -> FrameLaneTracker {
    FrameLaneTracker::new(
        Instant::now(),
        Instant::now(),
        state.base,
        state.date_ptr as usize + GS_CURRENT_DATE_OFFSET,
    )
}

pub(super) fn cache_for(state: &CollectorState, game_hour_raw: i32) -> CompactFrameMapTopologyCache {
    CompactFrameMapTopologyCache::from_clock_roots(
        state.base,
        state.date_ptr as usize + GS_CURRENT_DATE_OFFSET,
        Instant::now(),
        game_hour_raw,
    )
}

/// Where two row vectors first disagree, or `None` when they are identical.
///
/// A bare `assert_eq!` over 10,000 rows prints both vectors in full and buries
/// the one row that differs under a megabyte of the ones that do not. This
/// reports the length mismatch or the FIRST differing index and its two rows,
/// which is what a reader of the failure actually needs.
fn first_difference<T: Clone + PartialEq + std::fmt::Debug>(
    left: &[T],
    right: &[T],
) -> Option<String> {
    if left.len() != right.len() {
        return Some(format!(
            "lengths differ: {} vs {}",
            left.len(),
            right.len()
        ));
    }
    left.iter()
        .zip(right)
        .enumerate()
        .find(|(_, (l, r))| l != r)
        .map(|(index, (l, r))| format!("row {index} differs: {l:?} vs {r:?}"))
}

/// The comparable projection of a topology. `CompactMapControlTopology` carries
/// no `PartialEq`, and the interesting claim is about its CONTENT rather than
/// its identity.
fn topology_shape(topology: &CompactMapControlTopology) -> (usize, u32, Vec<(u32, usize)>, Vec<(u32, usize, u32)>) {
    (
        topology.province_array,
        topology.province_count,
        topology
            .states
            .iter()
            .map(|root| (root.state_id, root.state_ptr))
            .collect(),
        topology
            .provinces
            .iter()
            .map(|root| (root.province_id, root.province_ptr, root.state_id))
            .collect(),
    )
}

// ---------------------------------------------------------------------
// The wiring
// ---------------------------------------------------------------------

/// The build the fixture's clock can vouch for is byte-for-byte the build a
/// reader would have performed on its own.
///
/// This is the claim the whole design rests on: the fallback must not be a
/// DIFFERENT answer, or a refusal would change what the hour publishes rather
/// than only what it costs. Both sides here go through the production
/// `sample_compact_map_control_cached`, one accepting the share and one forced
/// to rebuild, and the rows are compared.
///
/// MUTATION RUN: `sample_compact_map_control_cached`'s shared arm changed to
/// `let province_rows = Vec::new();`. Observed in `--release`:
///
/// ```text
/// assertion `left == right` failed: the shared province rows are not the rows the local build produced
///   left: Some("lengths differ: 0 vs 10000")
///  right: None
/// ```
///
/// Restored.
#[test]
fn an_accepted_share_returns_the_rows_a_local_rebuild_would_have_returned() {
    page_cache_new_frame();
    let fixture = staged_map_fixture();
    let state = fixture.collector_state();
    let hour = fixture_game_hour_raw();
    let lanes = lanes_for(&state);

    // The forced-rebuild side FIRST, with no bracket held, so it is refused
    // before the cache ever builds and takes the pre-cache code path.
    let rebuild_cache = cache_for(&state, hour);
    let mut rebuild_tracker = CompactMapControlTracker::default();
    let (rebuilt_states, rebuilt_provinces, rebuilt_full) =
        sample_compact_map_control_cached(&state, &mut rebuild_tracker, &rebuild_cache)
            .expect("the staged map builds");
    let rebuild_counts = rebuild_cache.counts();
    assert_eq!(
        (rebuild_counts.accepted, rebuild_counts.rebuilt, rebuild_counts.builds),
        (0, 1, 0),
        "a consumer with no bracket must be refused BEFORE the frame mints a \
         build it could never accept"
    );
    assert_eq!(rebuild_counts.rejected_no_bracket, 1);

    // The shared side, under a real bracket.
    let share_cache = cache_for(&state, hour);
    let mut share_tracker = CompactMapControlTracker::default();
    let (shared_states, shared_provinces, shared_full) = {
        let _bracket = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_CONTROL);
        sample_compact_map_control_cached(&state, &mut share_tracker, &share_cache)
            .expect("the staged map builds")
    };
    let share_counts = share_cache.counts();
    assert_eq!(
        (
            share_counts.builder_kept_own,
            share_counts.accepted,
            share_counts.rebuilt,
            share_counts.builds
        ),
        (1, 0, 0, 1),
        "this consumer PERFORMED the walk, so it keeps its own build and the \
         share rate is not credited for it"
    );

    assert_eq!(
        first_difference(&shared_states, &rebuilt_states),
        None,
        "the shared state rows are not the rows the local build produced"
    );
    assert_eq!(
        first_difference(&shared_provinces, &rebuilt_provinces),
        None,
        "the shared province rows are not the rows the local build produced"
    );
    assert_eq!((shared_full, rebuilt_full), (true, true));
    assert_eq!(shared_states.len(), MAP_STATE_COUNT as usize);
    assert_eq!(
        shared_provinces.len(),
        MAP_PROVINCE_COUNT as usize - 1,
        "province 0 is not a row; every other slot is"
    );
    assert_eq!(
        topology_shape(share_tracker.topology.as_ref().expect("d18 caches geometry")),
        topology_shape(
            rebuild_tracker
                .topology
                .as_ref()
                .expect("d18 caches geometry")
        ),
        "an accepted share must leave the reader's own tracker holding the \
         same geometry a local build would have"
    );
}

/// A consumer whose bracket opened AFTER the shared build must rebuild.
///
/// This is the case the owner's condition exists for. Both consumers are in the
/// frame's labelled hour and neither straddles anything, so nothing but the
/// ordinal comparison can tell them apart -- which is precisely why an
/// implementation that dropped it would look healthy on every other test.
///
/// MUTATION RUN: `shared_map_topology_verdict`'s body replaced by
/// `SharedMapTopologyVerdict::Accept`. Observed in `--release`:
///
/// ```text
/// assertion `left == right` failed: a consumer whose bracket opened after the build cannot vouch for it
///   left: (1, 0)
///  right: (1, 1)
/// ```
///
/// Restored.
#[test]
fn a_consumer_whose_bracket_opened_after_the_build_rebuilds_instead() {
    page_cache_new_frame();
    let fixture = staged_map_fixture();
    let state = fixture.collector_state();
    let hour = fixture_game_hour_raw();
    let lanes = lanes_for(&state);
    let cache = cache_for(&state, hour);

    // First consumer: opens a bracket, mints the shared build inside it, and
    // accepts it.
    {
        let _bracket = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_CONTROL);
        assert!(
            matches!(
                cache.accepted_build(&state),
                SharedMapTopologyOffer::Accepted(_)
            ),
            "the consumer that performed the build is inside its own bracket"
        );
    }
    assert_eq!(cache.counts().builder_kept_own, 1);

    // Second consumer: its bracket opens AFTER the build returned, so the build
    // it is being offered happened outside its own read window.
    let mut states_seen = 0usize;
    {
        let _bracket = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_DETAIL);
        if let SharedMapTopologyOffer::Accepted(shared) = cache.accepted_build(&state) {
            states_seen = shared.topology.states.len();
        }
    }
    let counts = cache.counts();
    assert_eq!(
        (counts.builder_kept_own, counts.rejected_outside_bracket),
        (1, 1),
        "a consumer whose bracket opened after the build cannot vouch for it"
    );
    assert_eq!((counts.accepted, counts.rebuilt), (0, 1));
    assert_eq!(
        states_seen, 0,
        "the refused consumer must have been handed nothing at all"
    );
    assert_eq!(
        counts.builds, 1,
        "the refusal must not make the frame build a second shared copy"
    );
}

/// A frame whose label is not the hour its readers are observing shares
/// nothing, and does not even pay for the walk.
///
/// MUTATION RUN: the `consumer_opening.hour != game_hour_raw` refusal deleted
/// from `shared_map_topology_prebuild_admission`. Observed in `--release`:
///
/// ```text
/// assertion failed: matches!(cache.accepted_build(&state), SharedMapTopologyOffer::Rebuild)
/// ```
///
/// -- the consumer that could accept nothing is now handed the build it
/// performed, which is exactly the mislabelled read the rule exists to refuse.
/// Restored.
#[test]
fn an_hour_label_mismatch_refuses_the_share_before_the_walk_is_paid_for() {
    page_cache_new_frame();
    let fixture = staged_map_fixture();
    let state = fixture.collector_state();
    let lanes = lanes_for(&state);
    // The frame publishes under the NEXT hour while every bracket in it reads
    // the staged one -- the shape t17 measured 35 times and could not price,
    // because an accepted bracket recorded its hour nowhere.
    let cache = cache_for(&state, fixture_game_hour_raw() + 1);

    {
        let _bracket = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_CONTROL);
        assert!(matches!(
            cache.accepted_build(&state),
            SharedMapTopologyOffer::Rebuild
        ));
    }
    let counts = cache.counts();
    assert_eq!(
        (counts.accepted, counts.rejected_hour_label),
        (0, 1),
        "a bracket reading an hour other than the frame's label can accept \
         nothing"
    );
    assert_eq!(
        counts.builds, 0,
        "a consumer that could accept no build must not make the frame mint one"
    );

    // ... and the reader still gets its rows, from its own walk.
    let mut tracker = CompactMapControlTracker::default();
    let (states, provinces, full) = {
        let _bracket = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_CONTROL);
        sample_compact_map_control_cached(&state, &mut tracker, &cache)
            .expect("the fallback build is the code that ran before the cache")
    };
    assert!(full);
    assert_eq!(states.len(), MAP_STATE_COUNT as usize);
    assert_eq!(provinces.len(), MAP_PROVINCE_COUNT as usize - 1);
}

/// The cache is asked at most once per frame, however many readers consult it.
///
/// The point is the COST: six consumers sharing one walk is the whole change,
/// and a `get_or_init` that re-entered, or a consumer that took the shared
/// topology and then rebuilt anyway, would leave every other assertion in this
/// file green.
///
/// MUTATION RUN: `accepted_build`'s
/// `let outcome = self.built.get_or_init(|| self.perform_build(state));`
/// replaced by `let fresh = self.perform_build(state); let outcome =
/// self.built.get_or_init(|| fresh);`, so the walk runs on every consultation
/// while the shared value stays the first one. Observed in `--release`:
///
/// ```text
/// assertion `left == right` failed: the six converted readers must share ONE walk
///   left: 3
///  right: 1
/// ```
///
/// Restored.
#[test]
fn many_consumers_in_one_frame_share_a_single_walk() {
    page_cache_new_frame();
    let fixture = staged_map_fixture();
    let state = fixture.collector_state();
    let hour = fixture_game_hour_raw();
    let lanes = lanes_for(&state);
    let cache = cache_for(&state, hour);

    // One bracket held across all three consultations, which is the shape a
    // single worker reading several dependent domains has; the ordinal rule
    // admits them all because the build began after this bracket opened.
    let _bracket = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_CONTROL);
    let mut pointers = Vec::new();
    for _ in 0..3 {
        let SharedMapTopologyOffer::Accepted(shared) = cache.accepted_build(&state) else {
            panic!("the share is accepted");
        };
        pointers.push(shared.topology.province_array);
    }
    let counts = cache.counts();
    assert_eq!(
        counts.builds, 1,
        "the six converted readers must share ONE walk"
    );
    assert_eq!(
        (counts.builder_kept_own, counts.accepted, counts.rebuilt),
        (1, 2, 0),
        "one consumer performed the walk; the other two SHARED it"
    );
    assert_eq!(
        pointers.iter().collect::<BTreeSet<_>>().len(),
        1,
        "every consumer was handed the same build"
    );
}

/// The bracket a reader is running under is the reader's OWN, even when
/// brackets nest.
///
/// d86 and d90 hold one each across the same terminal drain, so nesting is not
/// hypothetical. The innermost is the strict choice: openings are minted from a
/// process-global monotonic counter, so requiring a build to begin after the
/// innermost opening implies the same for every enclosing bracket.
///
/// MUTATION RUN: `ReaderClockBracketGuard::drop` changed to clear the slot
/// (`slot.set(None)`) instead of restoring `self.restore`. Observed in
/// `--release`: this test failed with
/// `the enclosing bracket must be restored, not cleared: assertion failed:
///  active_reader_bracket_opening().is_some()`. Restored.
#[test]
fn a_nested_bracket_hands_the_thread_back_to_the_one_that_enclosed_it() {
    page_cache_new_frame();
    let fixture = staged_map_fixture();
    let state = fixture.collector_state();
    let lanes = lanes_for(&state);

    assert!(
        active_reader_bracket_opening().is_none(),
        "no bracket is held outside a reader"
    );
    let outer = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_CONTROL);
    let outer_opening = active_reader_bracket_opening().expect("the outer bracket is readable");
    {
        let _inner = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_DETAIL);
        let inner_opening = active_reader_bracket_opening().expect("the inner bracket is readable");
        assert!(
            inner_opening.ordinal > outer_opening.ordinal,
            "the innermost bracket carries the largest opening ordinal"
        );
    }
    assert!(
        active_reader_bracket_opening().is_some(),
        "the enclosing bracket must be restored, not cleared"
    );
    assert_eq!(
        active_reader_bracket_opening().map(|obs| obs.ordinal),
        Some(outer_opening.ordinal)
    );
    drop(outer);
    assert!(
        active_reader_bracket_opening().is_none(),
        "the outermost bracket's exit leaves the thread unbracketed"
    );
}

// ---------------------------------------------------------------------
// The two divergences the adversarial review found (Codex thread
// 019fcfc0-e5b9-74e3-920d-654811a3f489, 2026-08-05)
// ---------------------------------------------------------------------

/// Geometry that the live map has moved on from is refused, even though its
/// clock pair is impeccable.
///
/// The clock rules place the walk in TIME. They say nothing about whether the
/// engine has since replaced the province array, and it can do that without the
/// hour moving -- so a consumer consulting the cache later in the same hour
/// could be handed geometry naming objects the live map no longer does, and
/// publish rows a local build would have read differently or refused. That is
/// the fallback's whole promise broken, in the one way the clock cannot see.
///
/// MUTATION RUN: the `cprovince_array_still_current` guard removed from
/// `accepted_build`. Observed in `--release`:
///
/// ```text
/// assertion failed: matches!(cache.accepted_build(&state), SharedMapTopologyOffer::Rebuild)
/// ```
///
/// Restored.
#[test]
fn a_province_array_replaced_after_the_walk_refuses_the_share() {
    page_cache_new_frame();
    let mut fixture = staged_map_fixture();
    let state = fixture.collector_state();
    let hour = fixture_game_hour_raw();
    let lanes = lanes_for(&state);
    let cache = cache_for(&state, hour);

    // One bracket, held across both consultations, so the ordinal rule admits
    // the second consumer and ONLY the array test can refuse it.
    let _bracket = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_CONTROL);
    assert!(
        matches!(
            cache.accepted_build(&state),
            SharedMapTopologyOffer::Accepted(_)
        ),
        "the first consultation performed the walk and keeps it"
    );

    // The engine resizes the province array. The scheduler hour does not move,
    // so every clock rule still passes.
    fixture
        .heap
        .write_u32(MAP_GS + CGAMESTATE_CPROVINCE_COUNT, MAP_PROVINCE_COUNT - 1);
    page_cache_new_frame();

    assert!(matches!(
        cache.accepted_build(&state),
        SharedMapTopologyOffer::Rebuild
    ));
    let counts = cache.counts();
    assert_eq!(
        (
            counts.builder_kept_own,
            counts.accepted,
            counts.rejected_array_changed
        ),
        (1, 0, 1),
        "geometry the live map has replaced must be refused however clean its \
         clock is"
    );
    assert_eq!(
        counts.builds, 1,
        "the refusal is the consumer's to answer with its own walk, not the \
         cache's to answer with a second shared one"
    );
}

/// A shared walk that the engine refuses is ONE spent attempt, and it is spent
/// by the consumer that performed it.
///
/// Before the cache, a site like d48 ran `build_compact_map_control_topology`
/// exactly once and returned its error. If the failing shared walk were simply
/// reported as "rebuild", that consumer would get a SECOND traversal it never
/// had -- and a transient engine refusal that used to fail d48's hour would
/// become one d48 silently survives. So the performer inherits the failure and
/// every other consumer, which spent nothing, still takes its own attempt.
///
/// MUTATION RUN: the `this_consumer_built` arm in `accepted_build` deleted, so
/// `BuildFailed` always returns `SharedMapTopologyOffer::Rebuild`. Observed in
/// `--release`:
///
/// ```text
/// assertion `left == right` failed: the consumer whose own walk failed must inherit that failure, not be handed a second attempt
///   left: 0
///  right: 1
/// ```
///
/// Restored.
#[test]
fn the_consumer_whose_shared_walk_failed_inherits_the_failure_it_paid_for() {
    page_cache_new_frame();
    let mut fixture = staged_map_fixture();
    let hour = fixture_game_hour_raw();
    // A province slot holding a non-heap pointer: the walk reaches it and
    // returns `province_pointer_invalid`, which is a REAL refusal of the engine
    // walk rather than a synthetic one injected into the cache.
    fixture.heap.write_u64(MAP_PROVINCE_ARRAY + 8, 0x1234);
    let state = fixture.collector_state();
    let lanes = lanes_for(&state);
    let cache = cache_for(&state, hour);

    let offer = {
        let _bracket = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_CONTROL);
        match cache.accepted_build(&state) {
            SharedMapTopologyOffer::BuiltAndFailed(reason) => Some(reason),
            _ => None,
        }
    };
    let counts = cache.counts();
    assert_eq!(
        counts.failed_through, 1,
        "the consumer whose own walk failed must inherit that failure, not be \
         handed a second attempt"
    );
    assert_eq!(
        offer,
        Some("province_pointer_invalid"),
        "and it inherits the engine's own reason, which is the string its own \
         build would have returned"
    );
    assert_eq!((counts.accepted, counts.rebuilt, counts.builds), (0, 0, 1));

    // A DIFFERENT consumer spent nothing on that walk, so it still gets its own
    // attempt -- and takes the local-build path.
    {
        let _bracket = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_DETAIL);
        assert!(matches!(
            cache.accepted_build(&state),
            SharedMapTopologyOffer::Rebuild
        ));
    }
    let counts = cache.counts();
    assert_eq!((counts.failed_through, counts.rebuilt), (1, 1));
    assert_eq!(
        counts.builds, 1,
        "the cached failure must not make the frame walk the map again"
    );

    // And d48's reader returns exactly the error its own build would have.
    let mut tracker = CompactDomain48Tracker::default();
    let outcome = {
        let _bracket = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_CONTROL);
        domain48_topology(&state, &mut tracker, &cache)
    };
    assert_eq!(outcome.err(), Some("province_pointer_invalid"));
}

/// An accepted share stands in for the geometry BUILD, never for the row walk.
///
/// The two projecting readers wrap themselves in a three-attempt retry. If an
/// accepted share whose projection tore fell through to a local build and a
/// SECOND projection inside the same attempt, one attempt would buy two row
/// walks and the wrapper's budget would be a lie -- a torn map could then
/// publish where the pre-cache reader exhausted its retries and badged the hour
/// `SOURCE_UNAVAILABLE`. So the wrapper stops consulting the share after any
/// failed attempt, and the retry takes the pre-cache path with its own build.
///
/// MUTATION RUN: `consult_share.then_some(topology_cache)` in
/// `sample_compact_map_detail_with_retry` replaced by `Some(topology_cache)`,
/// so a failed attempt keeps consulting the share. Observed in `--release`:
///
/// ```text
/// assertion `left == right` failed: a failed attempt must stop consulting the share, so the retry takes the pre-cache path
///   left: 3
///  right: 1
/// ```
///
/// Restored.
#[test]
fn a_failed_attempt_stops_consulting_the_share_for_the_rest_of_the_retries() {
    page_cache_new_frame();
    let fixture = staged_map_fixture();
    let state = fixture.collector_state();
    let hour = fixture_game_hour_raw();
    let lanes = lanes_for(&state);
    let cache = cache_for(&state, hour);
    let building_levels = CompactFrameBuildingLevelsCache::default();

    // `include_resources` with no staged resource vectors: the topology builds
    // and the PROJECTION is what fails, which is exactly the shape the budget
    // rule is about.
    let mut tracker = CompactMapDetailTracker::default();
    let outcome = {
        let _bracket = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_DETAIL);
        sample_compact_map_detail_with_retry(
            &state,
            true,
            false,
            &mut tracker,
            &building_levels,
            &cache,
            3,
        )
    };
    assert!(
        outcome.is_err(),
        "the fixture stages no resource vectors, so every projection tears"
    );
    let counts = cache.counts();
    assert_eq!(
        counts.builder_kept_own + counts.accepted,
        1,
        "a failed attempt must stop consulting the share, so the retry takes \
         the pre-cache path"
    );
    assert_eq!(
        counts.shared_rows_unreadable, 1,
        "and the one attempt that did consult it reports the torn projection"
    );
    assert_eq!(
        counts.builds, 1,
        "the retries build locally; the shared walk is still performed once"
    );
}
