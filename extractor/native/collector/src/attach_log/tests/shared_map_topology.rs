use super::state_fixture::{fixture_game_hour_raw, CollectorStateFixture};
use super::*;

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

fn clean_pair() -> (ClockObservation, ClockObservation, ClockObservation) {
    let consumer = clock_obs(label_hour(), 10);
    let opening = clock_obs(label_hour(), 11);
    let closing = clock_obs(label_hour(), 12);
    (consumer, opening, closing)
}

/// `SharedMapTopologyVerdict::Accept`. Observed in `--release`:
#[test]
fn the_acceptance_check_publishes_only_a_build_bracketed_by_its_own_consumer() {
    let (consumer, opening, closing) = clean_pair();
    assert_eq!(
        shared_map_topology_verdict(opening, closing, Some(consumer), label_hour()),
        SharedMapTopologyVerdict::Accept,
        "a build that began inside the consumer's bracket, in the frame's own \
         hour, is exactly what the owner approved sharing"
    );

    let late_consumer = clock_obs(label_hour(), opening.ordinal + 1);
    assert_eq!(
        shared_map_topology_verdict(opening, closing, Some(late_consumer), label_hour()),
        SharedMapTopologyVerdict::OutsideConsumerBracket
    );

    assert_eq!(
        shared_map_topology_verdict(opening, closing, None, label_hour()),
        SharedMapTopologyVerdict::NoConsumerBracket
    );

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

/// from `shared_map_topology_prebuild_admission`. Observed in `--release`:
#[test]
fn a_build_or_a_consumer_outside_the_frames_labelled_hour_never_publishes() {
    let (consumer, opening, closing) = clean_pair();

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

/// return deleted. Observed in `--release`:
#[test]
fn only_a_consumer_that_did_not_perform_the_walk_owes_the_extra_proofs() {
    let (consumer, opening, closing) = clean_pair();

    assert_eq!(
        shared_map_topology_claim(false, opening, None, Some(consumer), label_hour()),
        SharedMapTopologyClaim::Refused(SharedMapTopologyVerdict::BuildCloseUnreadable)
    );
    assert_eq!(
        shared_map_topology_claim(true, opening, None, Some(consumer), label_hour()),
        SharedMapTopologyClaim::Builder,
        "the consumer that performed the walk owes no proof it did not owe \
         before the cache"
    );

    assert_eq!(
        shared_map_topology_claim(false, opening, Some(closing), Some(consumer), label_hour()),
        SharedMapTopologyClaim::SharedPendingArrayCheck
    );
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

const MAP_GS: usize = 0x0000;
pub(super) const MAP_PROVINCE_ARRAY: usize = 0x1000;
const MAP_STATES: usize = 0x18000;
const MAP_PROVINCES: usize = 0xC0000;
const MAP_STATE_STRIDE: usize = 0xD0;
const MAP_PROVINCE_STRIDE: usize = 0x190;
const MAP_PROVINCE_COUNT: u32 = 10_001;
const MAP_STATE_COUNT: u32 = 1_200;
const MAP_HEAP_LEN: usize =
    MAP_PROVINCES + (MAP_PROVINCE_COUNT as usize + 1) * MAP_PROVINCE_STRIDE + 0x1000;

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
        fixture
            .heap
            .write_u64(MAP_PROVINCE_ARRAY + province_id as usize * 8, province_addr);
    }
    fixture
}

#[test]
fn a_small_map_builds_and_a_map_that_authenticates_nothing_is_refused() {
    let mut fixture = staged_map_fixture();
    const SMALL_PROVINCES: u32 = 32;
    fixture
        .heap
        .write_u32(MAP_GS + CGAMESTATE_CPROVINCE_COUNT, SMALL_PROVINCES);
    let state = fixture.collector_state();
    let (topology, _, _) =
        build_compact_map_control_topology(&state).expect("a small map is an ordinary map");
    assert_eq!(
        topology.provinces.len(),
        SMALL_PROVINCES as usize - 1,
        "every province slot but the null one authenticates"
    );
    assert!(
        !topology.states.is_empty(),
        "and the states its provinces name come with it"
    );

    let mut zeroed = staged_map_fixture();
    zeroed
        .heap
        .write_u32(MAP_GS + CGAMESTATE_CPROVINCE_COUNT, SMALL_PROVINCES);
    for province_id in 1..SMALL_PROVINCES {
        zeroed
            .heap
            .write_u64(MAP_PROVINCE_ARRAY + province_id as usize * 8, 0);
    }
    let zeroed_state = zeroed.collector_state();
    assert_eq!(
        build_compact_map_control_topology(&zeroed_state).err(),
        Some("map_topology_authenticated_no_rows"),
        "a declared array that authenticates nothing is incoherent, not empty"
    );

    let mut minimal = staged_map_fixture();
    minimal
        .heap
        .write_u32(MAP_GS + CGAMESTATE_CPROVINCE_COUNT, 2);
    let province_offset = MAP_PROVINCES + MAP_PROVINCE_STRIDE;
    minimal
        .heap
        .write_u64(province_offset + CPROVINCE_OFF_STATE, 0);
    minimal
        .heap
        .write_u32(province_offset + CPROVINCE_OFF_CONTROLLER, 0);
    let minimal_state = minimal.collector_state();
    let (minimal_topology, minimal_state_rows, _) =
        build_compact_map_control_topology(&minimal_state)
            .expect("one authenticated province and no states is a legal map");
    assert_eq!(minimal_topology.provinces.len(), 1);
    assert!(
        minimal_topology.states.is_empty() && minimal_state_rows.is_empty(),
        "a province that links no state contributes none, and that is legal"
    );

    for degenerate in [0u32, 1] {
        let mut small = staged_map_fixture();
        small
            .heap
            .write_u32(MAP_GS + CGAMESTATE_CPROVINCE_COUNT, degenerate);
        let small_state = small.collector_state();
        assert_eq!(
            build_compact_map_control_topology(&small_state).err(),
            Some("province_array_unavailable"),
            "a province count of {degenerate} declares no addressable province"
        );
    }
}

pub(super) fn lanes_for(state: &CollectorState) -> FrameLaneTracker {
    FrameLaneTracker::new(
        Instant::now(),
        Instant::now(),
        state.base,
        state.date_ptr as usize + GS_CURRENT_DATE_OFFSET,
    )
}

pub(super) fn cache_for(
    state: &CollectorState,
    game_hour_raw: i32,
) -> CompactFrameMapTopologyCache {
    CompactFrameMapTopologyCache::from_clock_roots(
        state.base,
        state.date_ptr as usize + GS_CURRENT_DATE_OFFSET,
        Instant::now(),
        game_hour_raw,
    )
}

fn first_difference<T: Clone + PartialEq + std::fmt::Debug>(
    left: &[T],
    right: &[T],
) -> Option<String> {
    if left.len() != right.len() {
        return Some(format!("lengths differ: {} vs {}", left.len(), right.len()));
    }
    left.iter()
        .zip(right)
        .enumerate()
        .find(|(_, (l, r))| l != r)
        .map(|(index, (l, r))| format!("row {index} differs: {l:?} vs {r:?}"))
}

fn topology_shape(
    topology: &CompactMapControlTopology,
) -> (usize, u32, Vec<(u32, usize)>, Vec<(u32, usize, u32)>) {
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

/// `let province_rows = Vec::new();`. Observed in `--release`:
#[test]
fn an_accepted_share_returns_the_rows_a_local_rebuild_would_have_returned() {
    page_cache_new_frame();
    let fixture = staged_map_fixture();
    let state = fixture.collector_state();
    let hour = fixture_game_hour_raw();
    let lanes = lanes_for(&state);

    let rebuild_cache = cache_for(&state, hour);
    let mut rebuild_tracker = CompactMapControlTracker::default();
    let (rebuilt_states, rebuilt_provinces, rebuilt_full) =
        sample_compact_map_control_cached(&state, &mut rebuild_tracker, &rebuild_cache)
            .expect("the staged map builds");
    let rebuild_counts = rebuild_cache.counts();
    assert_eq!(
        (
            rebuild_counts.accepted,
            rebuild_counts.rebuilt,
            rebuild_counts.builds
        ),
        (0, 1, 0),
        "a consumer with no bracket must be refused BEFORE the frame mints a \
         build it could never accept"
    );
    assert_eq!(rebuild_counts.rejected_no_bracket, 1);

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
        topology_shape(
            share_tracker
                .topology
                .as_ref()
                .expect("d18 caches geometry")
        ),
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

/// `SharedMapTopologyVerdict::Accept`. Observed in `--release`:
#[test]
fn a_consumer_whose_bracket_opened_after_the_build_rebuilds_instead() {
    page_cache_new_frame();
    let fixture = staged_map_fixture();
    let state = fixture.collector_state();
    let hour = fixture_game_hour_raw();
    let lanes = lanes_for(&state);
    let cache = cache_for(&state, hour);

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

/// from `shared_map_topology_prebuild_admission`. Observed in `--release`:
#[test]
fn an_hour_label_mismatch_refuses_the_share_before_the_walk_is_paid_for() {
    page_cache_new_frame();
    let fixture = staged_map_fixture();
    let state = fixture.collector_state();
    let lanes = lanes_for(&state);
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

/// while the shared value stays the first one. Observed in `--release`:
#[test]
fn many_consumers_in_one_frame_share_a_single_walk() {
    page_cache_new_frame();
    let fixture = staged_map_fixture();
    let state = fixture.collector_state();
    let hour = fixture_game_hour_raw();
    let lanes = lanes_for(&state);
    let cache = cache_for(&state, hour);

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

/// `--release`: this test failed with
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

/// `accepted_build`. Observed in `--release`:
#[test]
fn a_province_array_replaced_after_the_walk_refuses_the_share() {
    page_cache_new_frame();
    let mut fixture = staged_map_fixture();
    let state = fixture.collector_state();
    let hour = fixture_game_hour_raw();
    let lanes = lanes_for(&state);
    let cache = cache_for(&state, hour);

    let _bracket = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_CONTROL);
    assert!(
        matches!(
            cache.accepted_build(&state),
            SharedMapTopologyOffer::Accepted(_)
        ),
        "the first consultation performed the walk and keeps it"
    );

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

/// `--release`:
#[test]
fn the_consumer_whose_shared_walk_failed_inherits_the_failure_it_paid_for() {
    page_cache_new_frame();
    let mut fixture = staged_map_fixture();
    let hour = fixture_game_hour_raw();
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

    let mut tracker = CompactDomain48Tracker::default();
    let outcome = {
        let _bracket = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_MAP_CONTROL);
        domain48_topology(&state, &mut tracker, &cache)
    };
    assert_eq!(outcome.err(), Some("province_pointer_invalid"));
}

/// so a failed attempt keeps consulting the share. Observed in `--release`:
#[test]
fn a_failed_attempt_stops_consulting_the_share_for_the_rest_of_the_retries() {
    page_cache_new_frame();
    let fixture = staged_map_fixture();
    let state = fixture.collector_state();
    let hour = fixture_game_hour_raw();
    let lanes = lanes_for(&state);
    let cache = cache_for(&state, hour);
    let building_levels = CompactFrameBuildingLevelsCache::default();

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
