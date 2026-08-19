use super::state_fixture::{fixture_game_hour_raw, FixtureRegion};
use super::*;

fn d81_state(fixture: &super::state_fixture::CollectorStateFixture) -> CollectorState {
    let mut state = fixture.collector_state();
    state.daily_match = true;
    state.long_string_match = true;
    state
}

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

/// share is discarded and d81 refuses. Observed in `--release`:
#[test]
fn d81_publishes_the_frames_shared_walk_instead_of_performing_its_own() {
    page_cache_new_frame();
    let fixture = super::shared_map_topology::staged_map_fixture();
    let state = d81_state(&fixture);
    let hour = fixture_game_hour_raw();
    let lanes = super::shared_map_topology::lanes_for(&state);
    let cache = super::shared_map_topology::cache_for(&state, hour);

    let _d81_bracket = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE);
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

/// `Err("refused")`. Observed in `--release`:
#[test]
fn a_d81_that_cannot_vouch_for_the_share_builds_its_own_map() {
    page_cache_new_frame();
    let fixture = super::shared_map_topology::staged_map_fixture();
    let state = d81_state(&fixture);
    let hour = fixture_game_hour_raw();
    let cache = super::shared_map_topology::cache_for(&state, hour);

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

/// rebuild. Observed in `--release`:
#[test]
fn a_shared_walk_d81_performed_and_the_engine_refused_refuses_d81() {
    page_cache_new_frame();
    let mut fixture = super::shared_map_topology::staged_map_fixture();
    let hour = fixture_game_hour_raw();
    fixture
        .heap
        .write_u64(super::shared_map_topology::MAP_PROVINCE_ARRAY + 8, 0x1234);
    let state = d81_state(&fixture);
    let lanes = super::shared_map_topology::lanes_for(&state);
    let cache = super::shared_map_topology::cache_for(&state, hour);

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

const CONTROLLER_PROVINCE_STRIDE: usize = 0x200;
const CONTROLLER_PROVINCE_COUNT: usize = 8;

fn controller_heap() -> FixtureRegion {
    FixtureRegion::new(CONTROLLER_PROVINCE_STRIDE * (CONTROLLER_PROVINCE_COUNT + 1))
}

fn controller_slot(province_id: u32) -> usize {
    province_id as usize * CONTROLLER_PROVINCE_STRIDE
}

fn controller_authority() -> BTreeMap<u32, (u32, usize)> {
    (1u32..64).map(|raw| (raw, (raw + 1000, 0usize))).collect()
}

/// reads again. Observed in `--release`:
#[test]
fn one_controller_read_serves_every_pov_that_asks_for_that_province() {
    let mut heap = controller_heap();
    heap.write_u32(controller_slot(3) + 0x188, 7);
    heap.write_u32(controller_slot(5) + 0x188, 9);
    let authority = controller_authority();

    let mut frame_map = BTreeMap::new();
    let mut witnesses = Vec::new();
    let mut answers = Vec::new();
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
        vec![
            (7, 1007),
            (9, 1009),
            (7, 1007),
            (9, 1009),
            (7, 1007),
            (9, 1009)
        ],
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

/// *frame_witnesses;`. Observed in `--release`:
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

    heap.write_u32(controller_slot(3) + 0x188, 11);
    assert!(
        witnesses.iter().any(|witness| !witness.unchanged()),
        "a controller changed after the frame read it must still refuse the \
         frame; a dedup that dropped the witness would publish it"
    );
}

/// bracket that filled it. Observed in `--release`:
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

/// `--release`:
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

/// first `len` bytes. Observed in `--release`:
#[test]
fn the_shared_scratch_buffer_never_leaks_one_read_into_the_next() {
    let mut heap = FixtureRegion::new(0x400);
    heap.write_bytes(0x00, &[0xAAu8; 0x40]);
    heap.write_bytes(0x80, &[0xBBu8; 0x40]);

    let mut scratch = Vec::new();
    let long = d81_read_stable_bytes(
        heap.at(0x00),
        0x40,
        D81FailureStage::HubObject,
        &mut scratch,
    )
    .expect("the staged window is readable");
    assert_eq!(long, vec![0xAAu8; 0x40]);

    let short = d81_read_stable_bytes(
        heap.at(0x80),
        0x10,
        D81FailureStage::HubObject,
        &mut scratch,
    )
    .expect("the staged window is readable");
    assert_eq!(
        short,
        vec![0xBBu8; 0x10],
        "the second read must report the bytes at ITS address, not the bytes \
         the buffer still held from the first"
    );

    let long_again = d81_read_stable_bytes(
        heap.at(0x80),
        0x40,
        D81FailureStage::HubObject,
        &mut scratch,
    )
    .expect("the staged window is readable");
    assert_eq!(long_again, vec![0xBBu8; 0x40]);
}

/// Observed in `--release`:
#[test]
fn a_window_that_moved_between_the_two_copies_is_still_torn() {
    let mut scratch = Vec::new();
    let failure = d81_read_stable_bytes_with(
        0x4000,
        8,
        D81FailureStage::HubObject,
        &mut scratch,
        |_, len| Some(vec![0x11u8; len]),
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

/// Observed in `--release`:
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

/// staged before the read that may refuse. Observed in `--release`:
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

/// scratch.as_slice()`. Observed in `--release`:
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
        reader
            .matches("build_compact_map_control_topology(state)")
            .count(),
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

/// and this commit fixed. Observed in `--release`:
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
    let gate = hub.rsplit_once("|| controller_witnesses").expect(
        "the closing gate must re-read the deduplicated controller \
             witnesses; without that disjunct the dedup has no closing check \
             at all",
    );
    assert!(
        gate.0
            .contains("|| final_witnesses.iter().any(|witness| !witness.unchanged())"),
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

/// that ignores the scratch entirely. Observed in `--release`:
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

/// MEASURED, 9 interleaved trials, `--release`:
/// cargo test -p collector --release d81_topology_phase_cost_model -- --ignored --nocapture
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
    for _ in 0..TRIALS {
        let mark = Instant::now();
        let built = build_compact_map_control_topology(&state).expect("the staged map builds");
        rebuild_us.push(mark.elapsed().as_micros() as u64);
        assert_eq!(built.0.provinces.len(), 10_000);

        let cache = super::shared_map_topology::cache_for(&state, hour);
        let outer = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE);
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

const POV_COUNTRY_VTABLE: usize = 0x7ff7_0000_1000;
const POV_CCSS_VTABLE0: usize = 0x7ff7_0000_2000;
const POV_CCSS_VTABLE1: usize = 0x7ff7_0000_3000;
const POV_SUPPLY_SYSTEM: usize = 0x0000_1000_0000;
const POV_COUNTRY: usize = 0x0000_2000_0000;
const POV_COUNTRY_SYSTEM: usize = 0x0000_3000_0000;
const POV_COMPANION: usize = 0x0000_4000_0000;
const POV_RAW_HANDLE: u32 = 37;
const POV_CANONICAL: u32 = 12;

fn put_u64(window: &mut [u8], offset: usize, value: usize) {
    window[offset..offset + 8].copy_from_slice(&(value as u64).to_le_bytes());
}

fn pov_authority() -> D81CountryAuthority {
    let mut country_window = vec![0u8; D81_COUNTRY_IDENTITY_WINDOW_LEN];
    put_u64(&mut country_window, 0, POV_COUNTRY_VTABLE);
    country_window[OBJ_TAG_DB_ID..OBJ_TAG_DB_ID + 4].copy_from_slice(&POV_RAW_HANDLE.to_le_bytes());

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

#[test]
fn the_roster_loops_own_country_windows_are_admitted_by_the_pov_gate() {
    admit_pov(&pov_authority())
        .expect("the windows the roster loop authenticated must be admitted");
}

/// `--release`:
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
    authority.country_window = other.country_window;
    authority.country_system_window = other.country_system_window;
    let failure = admit_pov(&authority)
        .expect_err("a POV handed another country's windows must refuse the frame");
    assert_eq!(failure.reason, D81FailureReason::IdentityMismatch);
    assert_eq!(failure.subject_id, POV_CANONICAL);
}

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
        Some(POV_CANONICAL + 1),
    )
    .expect_err("a handle that no longer resolves to this POV must refuse");
    assert_eq!(failure.stage, D81FailureStage::HubRoot);
    assert_eq!(failure.reason, D81FailureReason::IdentityMismatch);
    assert_eq!(failure.subject_id, POV_CANONICAL);
    assert_eq!(failure.raw_value, u64::from(POV_RAW_HANDLE));
}

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

/// the retained windows are never closed. Observed in `--release`:
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

const SELECTOR_CPROVINCE_VTABLE: usize = 0x7ff7_99b1_8170;
const SELECTOR_PROVINCE_ID: u32 = 260;

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

/// deleted, so the slot is always read. Observed in `--release`:
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

/// any hint is honoured. Observed in `--release`:
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

/// for, and it is invisible to every behavioural test. Observed in `--release`:
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

/// MEASURED, `--release`, counted through the collector's own `rc_d219_reads`:
/// cargo test -p collector --release d81_elided_read_count_model -- --ignored --nocapture
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct D81ForeignValue {
    pub(crate) raw_handle: u32,
    pub(crate) equivalence_id: u32,
    pub(crate) value: i64,
}

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

fn cw_splitmix(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *seed;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn probe_tables_over(tables: &[(u32, u32, BTreeMap<u32, i64>)]) -> Vec<(u32, u32, D81RowTable)> {
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

    assert!(
        provinces_with_several > 0,
        "the shapes must produce provinces held by several related tables"
    );
}

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
        if !excluded_by_handle && !excluded_by_equivalence && entry.value > 0 && entry.value > best
        {
            best = entry.value;
        }
    }
    best
}

#[test]
fn the_query_time_probe_reports_the_same_foreign_maximum_for_every_query() {
    const PROVINCE_SPACE: u32 = 64;
    let mut seed = 0x51_9F_C0_DEu64;

    for trial in 0..200u32 {
        let table_count = 1 + (cw_splitmix(&mut seed) % 6) as usize;
        let tables: Vec<(u32, u32, BTreeMap<u32, i64>)> = (0..table_count)
            .map(|index| {
                let raw_handle = index as u32;
                let rows_len = 1 + (cw_splitmix(&mut seed) % 40) as usize;
                let rows = (0..rows_len)
                    .map(|_| {
                        let province = (cw_splitmix(&mut seed) % u64::from(PROVINCE_SPACE)) as u32;
                        let value = (cw_splitmix(&mut seed) % 41) as i64 - 20;
                        (province, value)
                    })
                    .collect::<BTreeMap<u32, i64>>();
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

struct ForeignFixture {
    #[allow(dead_code)]
    region: FixtureRegion,
    supply_system: usize,
    authority: BTreeMap<u32, (u32, usize)>,
    staged_rows: Vec<(u32, u32, BTreeMap<u32, i64>)>,
    expected_witness_addresses: [usize; 3],
}

fn stage_foreign_fixture(
    related_count: u32,
    rows_per_table: u32,
    province_space: u32,
    seed: &mut u64,
) -> ForeignFixture {
    const ROW_STRIDE: usize = 0x78;
    const COUNTRY_COUNT: u32 = 8;
    let mask: u32 = (rows_per_table * 4).next_power_of_two().max(16) - 1;
    let max_probe: u8 = 1;
    let slot_count = mask + u32::from(max_probe) + 2;
    let table_bytes = slot_count as usize * 0x18;

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
        region.write_u64(companion_off + 0x08, (base + buckets_off) as u64);
        region.write_u32(companion_off + 0x08 + 8, rows_per_table);
        region.write_u32(companion_off + 0x08 + 12, mask);
        region.write_u8(companion_off + 0x08 + 16, max_probe);

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
        region.write_u8(buckets_off + (slot_count as usize - 1) * 0x18 + 4, 0xff);

        authority.insert(raw_handle, (1000 + raw_handle, companion));
        staged_rows.push((raw_handle, 1000 + raw_handle, rows));
    }

    ForeignFixture {
        region,
        supply_system: base,
        authority,
        staged_rows,
        expected_witness_addresses: [
            base + vector_header_off,
            base + pov_row + 0x58,
            base + related_off,
        ],
    }
}

#[test]
fn the_staged_foreign_read_answers_like_the_unrestricted_index_and_mints_the_same_witnesses() {
    let mut seed = 0xF0_1E_16_4Eu64;
    for &(related_count, rows_per_table, province_space) in &[
        (4u32, 12u32, 64u32),
        (6, 200, 512),
        (3, 4, 32),
        (5, 64, 128),
    ] {
        let fixture =
            stage_foreign_fixture(related_count, rows_per_table, province_space, &mut seed);

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
        let source = d81_hub_foreign_source_for(
            &D81HubPovPlan {
                foreign_related: plan.related.clone(),
                ..D81HubPovPlan::default()
            },
            &table_cache,
        );

        let oracle = unrestricted_foreign_index(&fixture.staged_rows);
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
        assert_eq!(
            table_cache.len(),
            related_count as usize,
            "every related companion's table must still be decoded and cached"
        );

        drop(fixture);
    }
}

#[test]
fn the_foreign_read_offers_one_probe_target_per_authenticated_handle() {
    let mut seed = 0x0081_C0DEu64;
    let fixture = stage_foreign_fixture(6, 48, 96, &mut seed);

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

    let phases = D81PhaseAccum::default();
    for province_id in 1..=48u32 {
        source.maximum_excluding_controller(&phases, province_id, 0, 0);
    }
    let snapshot = phases.snapshot();
    assert_eq!(snapshot.hub_query_calls, 48);
    assert_eq!(snapshot.hub_query_probes, 288);
}

#[test]
fn the_probe_charges_for_queries_performed_not_for_routes_planned() {
    let phases = D81PhaseAccum::default();
    let empty: Vec<(u32, u32, D81RowTable)> = (0..9u32)
        .map(|handle| (handle + 1, handle + 1001, D81RowTable::default()))
        .collect();

    let wide = lazy_source_over(&empty[..5]);
    for _ in 0..2 {
        wide.maximum_excluding_controller(&phases, 1, 0, 0);
    }
    let narrow = lazy_source_over(&empty[..1]);
    for _ in 0..4 {
        narrow.maximum_excluding_controller(&phases, 1, 0, 0);
    }
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

    let rows_from = |maximum: &dyn Fn(u32, u32, u32) -> i64| -> Vec<RailHubSimulationState> {
        let mut rows = Vec::new();
        for (subtype, route_class) in [(0u8, 2u8), (1u8, 1u8)] {
            for &province_id in &queried {
                let controller_raw_handle = province_id % 4;
                let controller_country_id = 1000 + controller_raw_handle;
                rows.push(RailHubSimulationState {
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
    assert_eq!(
        probed_rows, index_rows,
        "the query-time probe must produce identical hub simulation rows"
    );
    assert!(
        probed_rows.iter().any(|row| row.foreign_max_raw > 0),
        "the fixture must produce at least one non-zero foreign maximum"
    );
}

pub(crate) mod t31 {
    pub(crate) const POV_COUNT: u64 = 439;
    pub(crate) const RELATED_VISITS: u64 = 2_005;
    pub(crate) const QUERY_CALLS: u64 = 11_586;
    pub(crate) const QUERY_PROBES: u64 = 345_704;
    pub(crate) const TABLE_ROWS_VISITED: u64 = 552_492;
    pub(crate) const PROBE_HITS: u64 = 281_693;
    pub(crate) const READ_US: u64 = 48_449;
    pub(crate) const HUB_US: u64 = 32_766;
    pub(crate) const HUB_FOREIGN_US: u64 = 19_740;
    pub(crate) const HUB_NODES_US: u64 = 2_014;
    pub(crate) const HUB_CLOSE_US: u64 = 1_388;
    pub(crate) const PROVINCES_VISITED: u32 = 13_413;
}

const T31_TIERS: [(usize, usize, usize); 3] = [(20, 48, 330), (89, 8, 38), (330, 1, 5)];

const T31_ROWS_PER_TABLE: usize = 276;

const T31_CORRIDOR: usize = 339;
const T31_GROUP: usize = 55;

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

pub(super) struct T31Frame {
    pub(super) tables: Vec<(u32, u32, D81RowTable)>,
    pub(super) related: Vec<Vec<usize>>,
    pub(super) queries: Vec<Vec<u32>>,
}

impl T31Frame {
    pub(super) fn build(seed: &mut u64) -> Self {
        let country_count = T31_GROUP * ((t31::POV_COUNT as usize).div_ceil(T31_GROUP));
        let group_count = country_count / T31_GROUP;

        let corridors: Vec<Vec<u32>> = (0..group_count)
            .map(|_| {
                let mut pool = BTreeSet::new();
                while pool.len() < T31_CORRIDOR {
                    pool.insert(1 + (cw_splitmix(seed) % u64::from(t31::PROVINCES_VISITED)) as u32);
                }
                pool.into_iter().collect()
            })
            .collect();

        let tables: Vec<(u32, u32, D81RowTable)> = (0..country_count)
            .map(|country| {
                let corridor = &corridors[country / T31_GROUP];
                let mut rows = BTreeMap::new();
                while rows.len() < T31_ROWS_PER_TABLE {
                    let province = corridor[(cw_splitmix(seed) % T31_CORRIDOR as u64) as usize];
                    rows.insert(province, (cw_splitmix(seed) % 4_000) as i64 - 1_000);
                }
                (
                    country as u32 + 1,
                    country as u32 / 2 + 1_000,
                    rows.into_iter().collect::<D81RowTable>(),
                )
            })
            .collect();

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

/// cargo test -p collector --release d81_foreign_fold_cost_model -- --ignored --nocapture
#[test]
#[ignore]
fn d81_foreign_fold_cost_model() {
    use std::time::Instant;

    const TRIALS: usize = 15;
    let mut seed = 0xD8_1D_A0_31u64;
    let frame = T31Frame::build(&mut seed);

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
    close_to(
        frame.related_visits(),
        t31::RELATED_VISITS,
        "hub_related_visits",
    );
    close_to(
        frame.table_rows_visited(),
        t31::TABLE_ROWS_VISITED,
        "hub_table_rows_visited",
    );
    close_to(frame.query_calls(), t31::QUERY_CALLS, "hub_query_calls");
    close_to(frame.query_probes(), t31::QUERY_PROBES, "hub_query_probes");

    let eager_phases = D81PhaseAccum::default();
    let eager_frame = || {
        let mut checksum = 0i64;
        for pov in 0..frame.related.len() {
            let queried: BTreeSet<u32> = frame.queries[pov].iter().copied().collect();
            let mut index: BTreeMap<u32, Vec<D81ForeignValue>> = BTreeMap::new();
            for &country in &frame.related[pov] {
                let (raw_handle, equivalence_id, rows) = &frame.tables[country];
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

    let snapshot = phases.snapshot();
    let per_frame = |total: u64| total / TRIALS as u64;
    let eager_snapshot = eager_phases.snapshot();
    assert_eq!(
        eager_snapshot.hub_query_probes, snapshot.hub_query_probes,
        "the eager and lazy arms must be charged the same probe total"
    );
    assert_eq!(eager_snapshot.hub_query_calls, snapshot.hub_query_calls);
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct D81Range {
    address: usize,
    len: usize,
}

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
/// acquired by a bounded set of copies planned before the bracket opens; a
/// cargo test -p collector --release d81_copy_only_manifest_census -- --ignored --nocapture
#[test]
#[ignore]
fn d81_copy_only_manifest_census() {
    use std::time::Instant;

    const TRIALS: usize = 51;
    const BUCKETS: usize = 512;
    const BUCKET_STRIDE: usize = 0x18;
    const TABLE_BLOB: usize = (BUCKETS - 1 + 1 + 2) * BUCKET_STRIDE;
    const TABLE_HEADER: usize = 0x11;
    const POV_HEADER: usize = 16;

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
        place(&mut manifest, &mut cursor, handles * 4);
    }
    assert!(
        cursor < region_len,
        "the staged manifest must fit its region: {cursor} >= {region_len}"
    );
    // fixture's own bounds check rather than by arithmetic alone.
    let _ = region.at(cursor - 1);

    let coalesced = coalesce(manifest.clone());
    let unique_bytes: usize = coalesced.iter().map(|range| range.len).sum();
    let moved_bytes = unique_bytes * 2;
    let pages: BTreeSet<usize> = coalesced
        .iter()
        .flat_map(|range| (range.address..range.address + range.len).step_by(4096))
        .map(|address| address / 4096)
        .collect();

    let mut destination = vec![0u8; moved_bytes + 4096];
    let mut evictor = vec![0u8; 64 << 20];

    let copy_pass = |destination: &mut [u8]| {
        let mut written = 0usize;
        for range in &coalesced {
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
    println!(
        "  optimistic scatter/gather, HOT   us  {}",
        tail(hot_ns.clone())
    );
    println!(
        "  optimistic scatter/gather, COLD  us  {}",
        tail(cold_ns.clone())
    );
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

///  * `key <= first` or `key >= last` in the bound check, which drops the two
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
            shuffled.swap(
                index,
                (cw_splitmix(&mut seed) % (index as u64 + 1)) as usize,
            );
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

#[test]
fn a_repeated_table_key_refuses_at_the_bucket_the_map_insert_refused_at() {
    let layouts: [&[u32]; 6] = [
        &[7, 9, 11, 13],
        &[7, 9, 7, 13],
        &[5, 9, 9, 5, 13],
        &[21, 4, 21, 4],
        &[3],
        &[7, 7, 0],
    ];
    let mut refusals = 0usize;
    let mut clean = 0usize;

    for (case, keys) in layouts.iter().enumerate() {
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

#[test]
fn a_low_duplicate_outranks_a_malformed_bucket_above_it() {
    let mut fixture = stage_single_table(&[7, 9, 9, 13, 21]);
    fixture.set_marker(5, 0xfe);
    let failure = d81_hash_table_rows(fixture.table, &mut Vec::new())
        .expect_err("the staged table is malformed twice over");
    assert_eq!(
        (failure.reason, failure.subject_id, failure.index),
        (D81FailureReason::Duplicate, 9, 2),
        "a duplicate below the malformed bucket must be the published refusal"
    );

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

struct SingleTableFixture {
    region: FixtureRegion,
    table: usize,
    buckets_off: usize,
}

impl SingleTableFixture {
    fn set_marker(&mut self, slot: usize, marker: u8) {
        let offset = self.buckets_off + slot * 0x18;
        self.region.write_u8(offset + 4, marker);
    }
}

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
