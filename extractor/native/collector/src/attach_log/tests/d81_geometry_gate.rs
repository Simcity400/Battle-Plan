//! Lane PW: the unchanged-geometry gate fires, and the rows it publishes are
//! the rows the walk would have produced.
//!
//! The gate's whole claim is an implication — equal inputs imply equal rows —
//! so these tests attack both halves of it: that it actually FIRES (a gate that
//! never hits is a cost with no saving), and that every input it does not
//! compare would break it (a gate that hits on inputs that differ publishes the
//! wrong network).
//!
//! `super::d81_authoritative` owns the synthetic world; nothing here builds its
//! own, because a fixture whose shape drifts from the one the walk tests use
//! would prove the gate agrees with a walk nobody runs.

use super::d81_authoritative::{
    map_province_of, stage_world, stage_world_in, walk_with, Layout, StagedWorld, WorldShape,
    ACTIVE_TABLE, CPROVINCE_VTABLE, GAMESTATE, IDLER, OWNER, RAIL_TABLE, SELECTOR_MASK,
    SELECTOR_MAX_DISTANCE, SELECTOR_TABLE, WORLD_HOUR,
};
use crate::attach_log::d81_copy_bracket::{CountingSnapshotMemory, D81PlanCounters, D81_CONSUMER};
use crate::attach_log::d81_geometry_gate::{
    D81GeometryGate, D81GeometryScalars, D81GeometryVerdict,
};
use crate::attach_log::{d81_observe_copy_plan_with, D81PhaseAccum, D81WalkRows};

/// The anchor word the tests edit to make one copied byte differ.
///
/// An anchor triplet is three opaque `f32` bit patterns copied straight through
/// to the row, so ANY value is a legal one — which is what makes it the right
/// field to perturb: the walk cannot refuse it, so a miss here is the gate
/// noticing the change and nothing else.
const ANCHOR_PRIMARY_OFF: u64 = 0x14E4;

/// The province whose anchor the tests edit. Inside the railed range, so it is
/// a province the walk actually publishes a row for.
const EDITED_PROVINCE: u32 = 1;

fn with_anchor(word: u32) -> StagedWorld {
    let mut world = stage_world(WorldShape::default());
    world
        .memory
        .write_u32(map_province_of(EDITED_PROVINCE) + ANCHOR_PRIMARY_OFF, word);
    world
}

/// The prefix windows the gate does not compare byte for byte. The fixture
/// stages its chain at fixed addresses, so these are the same four the
/// production reader hands in.
fn chain_windows(world: &StagedWorld) -> [(usize, u64); 4] {
    use crate::attach_log::{
        D81_CONTAINER_PROVINCE_COUNT_OFF, D81_GS_ANCHOR_SELECTOR_MAP_OFF, D81_IDLER_OWNER_OFF,
        D81_OWNER_CONTAINER_OFF,
    };
    [
        (GAMESTATE as usize, (D81_GS_ANCHOR_SELECTOR_MAP_OFF + 8) as u64),
        (IDLER as usize, (D81_IDLER_OWNER_OFF + 8) as u64),
        (OWNER as usize, (D81_OWNER_CONTAINER_OFF + 8) as u64),
        (
            world.container.container,
            (D81_CONTAINER_PROVINCE_COUNT_OFF + 4) as u64,
        ),
    ]
}

/// The plan's span list for a staged observation, in the shape production
/// hands the gate.
///
/// `bulk_spans` plus the two chain hops it omits - the same union
/// `d81_finish_rail_replay_state` builds, and built here rather than a
/// simplification of it, so a test cannot pass on a span list production never
/// produces.
fn plan_spans_of(
    observation: &crate::attach_log::D81PlanObservation,
    _world: &StagedWorld,
) -> Vec<(u64, u64)> {
    observation
        .bulk_spans
        .iter()
        .copied()
        // ONLY THE TWO bulk_spans OMITS, exactly as production does: adding a
        // span the list already holds makes every ordinary record count two
        // overlaps and disqualifies every exclusion.
        .chain(
            [IDLER, OWNER]
                .into_iter()
                .zip([
                    (crate::attach_log::D81_IDLER_OWNER_OFF + 8) as u64,
                    (crate::attach_log::D81_OWNER_CONTAINER_OFF + 8) as u64,
                ])
                .map(|(base, len)| (base, base.saturating_add(len))),
        )
        .collect()
}

fn scalars_of(world: &StagedWorld) -> D81GeometryScalars {
    D81GeometryScalars::capture(
        0,
        GAMESTATE as usize,
        RAIL_TABLE as usize,
        ACTIVE_TABLE as usize,
        SELECTOR_TABLE as usize,
        SELECTOR_MASK,
        SELECTOR_MAX_DISTANCE,
        CPROVINCE_VTABLE as usize,
        world.container,
        &world.topology,
    )
}

/// Plan, walk, and hand the result to the gate to remember — the exact sequence
/// `d81_finish_rail_replay_state` runs on a walked frame.
fn observe_and_remember(gate: &mut D81GeometryGate, world: &StagedWorld) -> D81WalkRows {
    let mut plan = D81PlanCounters::default();
    let memory = CountingSnapshotMemory::new(&world.memory);
    let observation = d81_observe_copy_plan_with(&memory, &world.roots, WORLD_HOUR, &mut plan)
        .expect("custody admits a fixture this size");
    let source = observation
        .source_for(WORLD_HOUR)
        .expect("the plan captured, so its consumer has a view");
    let phases = D81PhaseAccum::default();
    let rows =
        walk_with(world, &source, &phases).expect("the default fixture walks clean");
    plan.misses = source.misses();
    let view = observation
        .closed
        .snapshot
        .view_for(D81_CONSUMER, WORLD_HOUR)
        .expect("view issues for the plan's own hour");
    gate.remember(
        &observation.closed.snapshot,
        &view,
        observation.vector.as_ref(),
        scalars_of(world),
        &chain_windows(world),
        &plan_spans_of(&observation, world),
        &rows,
        crate::attach_log::d81_geometry_gate::D81WalkCounters::capture(
            &phases,
            &std::cell::Cell::new(0),
            &std::cell::Cell::new(0),
        ),
        plan.consumer_gapped,
        plan.misses.total(),
    );
    rows
}

/// `observe_and_remember` with scalars the caller chooses.
///
/// Used to make the CHAIN-ROOT RE-PIN disagree with the memo without also
/// moving the copy: the memo is filled with scalars that name a table the copy
/// does not point at, so the scalar compare passes on the next frame and the
/// re-pin is the only thing left that can refuse.
fn remember_with_scalars(
    gate: &mut D81GeometryGate,
    world: &StagedWorld,
    scalars: D81GeometryScalars,
) {
    let mut plan = D81PlanCounters::default();
    let memory = CountingSnapshotMemory::new(&world.memory);
    let observation = d81_observe_copy_plan_with(&memory, &world.roots, WORLD_HOUR, &mut plan)
        .expect("custody admits a fixture this size");
    let source = observation
        .source_for(WORLD_HOUR)
        .expect("the plan captured, so its consumer has a view");
    let phases = D81PhaseAccum::default();
    let rows = walk_with(world, &source, &phases).expect("the default fixture walks clean");
    let view = observation
        .closed
        .snapshot
        .view_for(D81_CONSUMER, WORLD_HOUR)
        .expect("view issues for the plan's own hour");
    gate.remember(
        &observation.closed.snapshot,
        &view,
        observation.vector.as_ref(),
        scalars,
        &chain_windows(world),
        &plan_spans_of(&observation, world),
        &rows,
        crate::attach_log::d81_geometry_gate::D81WalkCounters::default(),
        plan.consumer_gapped,
        plan.misses.total(),
    );
}

/// Plan a world and ASK the gate, without walking.
fn ask(
    gate: &mut D81GeometryGate,
    world: &StagedWorld,
    phases: &D81PhaseAccum,
) -> D81GeometryVerdict {
    ask_with_scalars(gate, world, phases, scalars_of(world))
}

fn ask_with_scalars(
    gate: &mut D81GeometryGate,
    world: &StagedWorld,
    phases: &D81PhaseAccum,
    scalars: D81GeometryScalars,
) -> D81GeometryVerdict {
    let mut plan = D81PlanCounters::default();
    let memory = CountingSnapshotMemory::new(&world.memory);
    let observation = d81_observe_copy_plan_with(&memory, &world.roots, WORLD_HOUR, &mut plan)
        .expect("custody admits a fixture this size");
    let view = observation
        .closed
        .snapshot
        .view_for(D81_CONSUMER, WORLD_HOUR)
        .expect("view issues for the plan's own hour");
    gate.decide(
        &observation.closed.snapshot,
        &view,
        observation.vector.as_ref(),
        &scalars,
        &chain_windows(world),
        &plan_spans_of(&observation, world),
        plan.consumer_gapped,
        plan.misses.total(),
        phases,
    )
}

/// THE GATE FIRES, AND THE ROWS IT PUBLISHES ARE THE WALK'S.
///
/// kills: deleting the `Reuse` arm (the gate never hits and `hit` stays 0);
/// returning a default/empty `D81WalkRows` from the memo instead of the
/// remembered one (the row comparison fails); remembering the rows BEFORE the
/// walk rather than after (the memo would hold rows nothing produced).
#[test]
fn an_identical_observation_reuses_the_rows_the_walk_produced() {
    let mut gate = D81GeometryGate::default();
    let first = stage_world(WorldShape::default());
    let walked = observe_and_remember(&mut gate, &first);
    // A SECOND, INDEPENDENTLY STAGED world of the same shape -- not the same
    // object re-planned. The gate must key on the BYTES, so two worlds built the
    // same way have to be indistinguishable to it.
    let again = stage_world(WorldShape::default());
    let phases = D81PhaseAccum::default();
    let reused = match ask(&mut gate, &again, &phases) {
        D81GeometryVerdict::Reuse(rows, _) => rows,
        D81GeometryVerdict::Walk => panic!(
            "an identical observation must reuse; the gate is all cost and no saving otherwise"
        ),
    };
    assert_eq!(
        phases.geometry_gate_hit.get(),
        1,
        "the hit must be counted, or a capture cannot say the gate did anything"
    );
    assert_eq!(
        reused.provinces, walked.provinces,
        "the reused rows must be the rows the walk produced, province for province"
    );
    assert_eq!(reused.observed_neighbors, walked.observed_neighbors);
    assert!(
        !reused.provinces.is_empty(),
        "a fixture publishing no rows would make the row comparison vacuous"
    );
}

/// ONE CHANGED WORD OF THE COPY IS A MISS.
///
/// Both worlds are staged identically and then given a DIFFERENT anchor word at
/// the same address, so exactly four copied bytes differ and nothing else does.
///
/// kills: comparing only the snapshot's interval framing and not its bytes;
/// comparing only the opening image; short-circuiting `image_matches` to true;
/// leaving the map-province windows out of what is fed.
#[test]
fn one_changed_word_of_the_copy_is_a_miss() {
    let mut gate = D81GeometryGate::default();
    let remembered = with_anchor(0x1111_1111);
    let walked = observe_and_remember(&mut gate, &remembered);
    let changed = with_anchor(0x2222_2222);
    let phases = D81PhaseAccum::default();
    match ask(&mut gate, &changed, &phases) {
        D81GeometryVerdict::Walk => {}
        D81GeometryVerdict::Reuse(..) => {
            panic!("a changed anchor must not reuse: that publishes the previous network")
        }
    }
    assert_eq!(phases.geometry_gate_miss.get(), 1);
    assert_eq!(phases.geometry_gate_hit.get(), 0);
    // AND THE CHANGE IS REAL: a walk over the edited world publishes different
    // rows, so the miss above was the gate protecting a genuine difference and
    // not a gate that simply never hits.
    let mut cold = D81GeometryGate::default();
    let rewalked = observe_and_remember(&mut cold, &changed);
    assert_ne!(
        rewalked.provinces, walked.provinces,
        "the edit must change the published rows, or this test proves nothing"
    );
}

/// THE SAME EDITED WORLD TWICE STILL HITS.
///
/// The companion to the test above: it pins that `with_anchor` itself does not
/// make two worlds differ, so the miss there is attributable to the WORD and not
/// to staging noise.
#[test]
fn the_same_edited_world_twice_is_a_hit() {
    let mut gate = D81GeometryGate::default();
    let first = with_anchor(0x3333_3333);
    let walked = observe_and_remember(&mut gate, &first);
    let second = with_anchor(0x3333_3333);
    let phases = D81PhaseAccum::default();
    match ask(&mut gate, &second, &phases) {
        D81GeometryVerdict::Reuse(rows, _) => assert_eq!(rows.provinces, walked.provinces),
        D81GeometryVerdict::Walk => panic!("two identically edited worlds must hit"),
    }
}

/// A CHANGED SCALAR IS A MISS EVEN THOUGH EVERY COPIED BYTE MATCHES.
///
/// `selector_mask` is not in the copy at all -- it is a header field the reader
/// decoded before the plan ran -- so a gate that compared only the snapshot
/// would hit here and publish rows resolved through a different selector table.
///
/// kills: dropping any field from `D81GeometryScalars`; comparing the scalars
/// with anything weaker than equality.
#[test]
fn a_changed_scalar_outside_the_copy_is_a_miss() {
    let mut gate = D81GeometryGate::default();
    let world = stage_world(WorldShape::default());
    let _walked = observe_and_remember(&mut gate, &world);
    let mut scalars = scalars_of(&world);
    scalars.selector_mask ^= 1;
    let phases = D81PhaseAccum::default();
    match ask_with_scalars(&mut gate, &world, &phases, scalars) {
        D81GeometryVerdict::Walk => {}
        D81GeometryVerdict::Reuse(..) => panic!(
            "a scalar the walk reads is part of its input; reusing across a change to it \
             publishes rows resolved through a table the frame did not use"
        ),
    }
    assert_eq!(phases.geometry_gate_miss.get(), 1);
}

/// A COLD GATE WALKS, and says so.
///
/// kills: seeding the memo with a default `D81WalkRows` -- the first frame would
/// publish an empty network as a hit.
#[test]
fn a_cold_gate_walks() {
    let mut gate = D81GeometryGate::default();
    let world = stage_world(WorldShape::default());
    let phases = D81PhaseAccum::default();
    match ask(&mut gate, &world, &phases) {
        D81GeometryVerdict::Walk => {}
        D81GeometryVerdict::Reuse(..) => panic!("an empty memo has no rows to reuse"),
    }
    assert_eq!(phases.geometry_gate_cold.get(), 1);
    assert_eq!(phases.geometry_gate_hit.get(), 0);
}

/// THE MEMO SERVES MORE THAN ONE FRAME. It is not consumed by the frame that
/// reads it, and it survives frames that never consulted it.
///
/// This is the property that stops the d47 death-spiral shape: the memo carries
/// no hour semantics, so a run of torn frames -- which never reach the compute
/// at all -- costs nothing beyond the walks those frames did not publish anyway.
///
/// kills: `Option::take`-ing the rows out of the memo on a hit instead of
/// cloning them (the second round reports `cold`); adding the memo to the
/// blanket tracker reset.
#[test]
fn the_memo_serves_more_than_one_frame() {
    let mut gate = D81GeometryGate::default();
    let first = stage_world(WorldShape::default());
    let walked = observe_and_remember(&mut gate, &first);
    for round in 0..3 {
        let world = stage_world(WorldShape::default());
        let phases = D81PhaseAccum::default();
        match ask(&mut gate, &world, &phases) {
            D81GeometryVerdict::Reuse(rows, _) => {
                assert_eq!(rows.provinces, walked.provinces, "round {round}");
            }
            D81GeometryVerdict::Walk => panic!("round {round} must still hit"),
        }
    }
}

/// A DIFFERENTLY SIZED WORLD IS A MISS -- the interval SET differs, not only its
/// bytes.
///
/// kills: omitting the interval count or the per-interval `(start, len)` framing
/// (a shorter observation could then compare equal to a prefix of a longer one);
/// dropping the trailing `cursor == image.len()` check.
#[test]
fn a_differently_sized_world_is_a_miss() {
    let mut gate = D81GeometryGate::default();
    let first = stage_world(WorldShape::default());
    let _walked = observe_and_remember(&mut gate, &first);
    // `sized` requires slots % 3 == 1 and >= 7; one triple fewer than the
    // default keeps both.
    let smaller = stage_world_in(
        WorldShape::default(),
        Layout::sized(super::d81_authoritative::PROVINCE_SLOTS - 3),
    );
    let phases = D81PhaseAccum::default();
    match ask(&mut gate, &smaller, &phases) {
        D81GeometryVerdict::Walk => {}
        D81GeometryVerdict::Reuse(..) => {
            panic!("a world with a different province count cannot reuse another's rows")
        }
    }
    assert_eq!(phases.geometry_gate_miss.get(), 1);
}

/// The gate's phase block is interpretable: the three eligible outcomes
/// partition `eligible`, a refusal is outside it, and a hit really did walk
/// bytes.
///
/// kills: bumping `eligible` on the refusal path, or forgetting to bump it on
/// one of the three eligible outcomes -- either makes the wire block
/// uninterpretable.
#[test]
fn the_phase_counters_partition_the_outcomes() {
    let mut gate = D81GeometryGate::default();
    let phases = D81PhaseAccum::default();
    // cold
    let _ = ask(&mut gate, &stage_world(WorldShape::default()), &phases);
    let first = stage_world(WorldShape::default());
    let _walked = observe_and_remember(&mut gate, &first);
    // hit
    let _ = ask(&mut gate, &stage_world(WorldShape::default()), &phases);
    // miss
    let _ = ask(&mut gate, &with_anchor(0x4444_4444), &phases);
    assert_eq!(phases.geometry_gate_hit.get(), 1);
    assert_eq!(phases.geometry_gate_cold.get(), 1);
    assert_eq!(phases.geometry_gate_miss.get(), 1);
    assert_eq!(
        phases.geometry_gate_eligible.get(),
        phases.geometry_gate_hit.get()
            + phases.geometry_gate_miss.get()
            + phases.geometry_gate_cold.get(),
        "the three eligible outcomes must partition `eligible`"
    );
    assert_eq!(phases.geometry_gate_refused.get(), 0);
    assert!(
        phases.geometry_gate_compared_bytes.get() > 0,
        "a hit walked the whole observation, so it compared bytes"
    );
}

/// A GATED FRAME PUBLISHES THE COUNTERS A WALKED FRAME WOULD.
///
/// `provinces_visited` and `provinces_railed` describe the NETWORK, not the
/// work, so a hit has to replay them. Left at zero they would deflate every
/// capture's province counts in proportion to the gate's hit rate — and since
/// the measured hit rate is 93-98%, that is very nearly all of them.
///
/// kills: deleting `D81WalkCounters::replay`; capturing the counters BEFORE the
/// walk instead of after (they would be replayed as zeros); adding rather than
/// assigning in `replay` (the counts would compound across hits).
#[test]
fn a_gated_frame_replays_the_walks_counters() {
    let mut gate = D81GeometryGate::default();
    let first = stage_world(WorldShape::default());
    // Walk once and record what the walk itself counted.
    let walked_phases = D81PhaseAccum::default();
    let mut plan = D81PlanCounters::default();
    let memory = CountingSnapshotMemory::new(&first.memory);
    let observation = d81_observe_copy_plan_with(&memory, &first.roots, WORLD_HOUR, &mut plan)
        .expect("custody admits a fixture this size");
    let source = observation
        .source_for(WORLD_HOUR)
        .expect("the plan captured, so its consumer has a view");
    let _ = walk_with(&first, &source, &walked_phases).expect("the fixture walks clean");
    let walked_visited = walked_phases.provinces_visited.get();
    let walked_railed = walked_phases.provinces_railed.get();
    assert!(
        walked_visited > 0 && walked_railed > 0,
        "a fixture that visits nothing would make this test vacuous"
    );

    let _ = observe_and_remember(&mut gate, &first);
    let gated_phases = D81PhaseAccum::default();
    let counters = match ask(&mut gate, &stage_world(WorldShape::default()), &gated_phases) {
        D81GeometryVerdict::Reuse(_, counters) => counters,
        D81GeometryVerdict::Walk => panic!("an identical observation must hit"),
    };
    counters.replay(
        &gated_phases,
        &std::cell::Cell::new(0),
        &std::cell::Cell::new(0),
    );
    assert_eq!(
        gated_phases.provinces_visited.get(),
        walked_visited,
        "a gated frame must publish the province count the walk would have"
    );
    assert_eq!(gated_phases.provinces_railed.get(), walked_railed);
    // AND THE TIMERS STAY ZERO: the gated frame did not spend that wall, and
    // replaying it would make the phase block claim time nobody took.
    assert_eq!(
        gated_phases.walk_ns.get(),
        0,
        "the walk's wall is not replayed; only its counts are"
    );
}

/// A CHANGED BYTE IN A SKIPPED PREFIX WINDOW STILL HITS.
///
/// This is the whole point of excluding the four pointer-chain prefixes, and
/// the offset is not arbitrary: `captures/pw_gate3` reported the first
/// differing window as exactly 888 bytes -- the OWNER hop -- with the first
/// differing byte at offset 0x318. That is the byte written here.
///
/// kills: dropping the `is_chain_window` skip (the gate goes back to 0 hits,
/// which is what the live capture measured before it existed).
#[test]
fn a_changed_byte_in_a_skipped_prefix_window_still_hits() {
    let mut gate = D81GeometryGate::default();
    let first = stage_world(WorldShape::default());
    let walked = observe_and_remember(&mut gate, &first);
    let mut churned = stage_world(WorldShape::default());
    // ALL FOUR PREFIXES, not just the one the capture happened to report first.
    // Each write lands below that hop's pointer offset, so no reader decodes
    // it -- and covering all four means a WRONG LENGTH in `chain_windows` (which
    // would silently stop the skip matching and take the gate back to 0 hits)
    // fails this test instead of passing quietly.
    //
    // 0x318 is the offset `captures/pw_gate3` reported as the first differing
    // byte, inside OWNER and below `D81_OWNER_CONTAINER_OFF` (0x370).
    churned.memory.write_u32(OWNER + 0x318, 0xDEAD_BEEF);
    churned.memory.write_u32(IDLER + 0x400, 0xCAFE_F00D);
    churned.memory.write_u32(GAMESTATE + 0x200, 0x0BAD_0BAD);
    churned
        .memory
        .write_u32(churned.container.container as u64 + 0x10, 0xFEED_FACE);
    let phases = D81PhaseAccum::default();
    match ask(&mut gate, &churned, &phases) {
        D81GeometryVerdict::Reuse(rows, _) => {
            assert_eq!(rows.provinces, walked.provinces);
        }
        D81GeometryVerdict::Walk => panic!(
            "a byte no reader decodes must not cost a walk; this is the miss the live \
             capture measured 253 times out of 253"
        ),
    }
    assert_eq!(phases.geometry_gate_hit.get(), 1);
}

/// BUT THE SHAPE OF A SKIPPED WINDOW IS STILL COMPARED.
///
/// Skipping a prefix's CONTENTS is not skipping its existence. A chain hop that
/// moved changes the window's `start`, and that is framing, which is always fed.
///
/// kills: moving the `is_chain_window` check above the framing sink -- the copy
/// could then be rooted somewhere else entirely and still hit.
#[test]
fn a_moved_chain_hop_is_still_a_miss() {
    let mut gate = D81GeometryGate::default();
    let first = stage_world(WorldShape::default());
    let _walked = observe_and_remember(&mut gate, &first);
    // Ask with the chain windows named at DIFFERENT addresses: the intervals
    // themselves are unchanged, so nothing is skipped that was skipped before,
    // and the prefixes now compare by bytes. They differ, so this misses --
    // which is the fail-closed direction.
    let phases = D81PhaseAccum::default();
    let world = stage_world(WorldShape::default());
    let mut plan = D81PlanCounters::default();
    let memory = CountingSnapshotMemory::new(&world.memory);
    let observation = d81_observe_copy_plan_with(&memory, &world.roots, WORLD_HOUR, &mut plan)
        .expect("custody admits a fixture this size");
    let view = observation
        .closed
        .snapshot
        .view_for(D81_CONSUMER, WORLD_HOUR)
        .expect("view issues for the plan's own hour");
    let mut churned = stage_world(WorldShape::default());
    churned.memory.write_u32(OWNER + 0x318, 0x1234_5678);
    let _ = churned;
    // Nothing skipped: an empty window list.
    let verdict = gate.decide(
        &observation.closed.snapshot,
        &view,
        observation.vector.as_ref(),
        &scalars_of(&world),
        &[],
        &plan_spans_of(&observation, &world),
        plan.consumer_gapped,
        plan.misses.total(),
        &phases,
    );
    // The memo was built while the four windows were SKIPPED, so an image built
    // without them cannot match one compared with them: the lengths differ.
    match verdict {
        D81GeometryVerdict::Walk => {}
        D81GeometryVerdict::Reuse(..) => {
            panic!("an image that covered different windows must not compare equal")
        }
    }
}

/// THE RE-PIN REFUSES WHEN THE COPY DOES NOT POINT WHERE THE SCALARS SAY.
///
/// The four prefixes are not compared byte for byte, so the six words the walk
/// decodes through them are checked by calling `d81_copied_roots_agree` on the
/// copy. Here the memo is filled with scalars naming a rail table the copy does
/// not point at: the scalar compare passes on the next frame (same scalars),
/// every compared byte matches, and the re-pin is the only thing that can
/// refuse -- so it must.
///
/// kills: deleting the `d81_copied_roots_agree` call in `decide` -- without it
/// this frame publishes rows resolved against a table the copy never followed.
#[test]
fn the_chain_root_repin_refuses_a_copy_that_points_elsewhere() {
    let mut gate = D81GeometryGate::default();
    let world = stage_world(WorldShape::default());
    let mut doctored = scalars_of(&world);
    doctored.rail_table ^= 0x40;
    remember_with_scalars(&mut gate, &world, doctored.clone());
    let phases = D81PhaseAccum::default();
    match ask_with_scalars(&mut gate, &world, &phases, doctored) {
        D81GeometryVerdict::Walk => {}
        D81GeometryVerdict::Reuse(..) => panic!(
            "the copy follows the real rail table; reusing against scalars that name \
             another one publishes rows the walk would have refused"
        ),
    }
    assert_eq!(phases.geometry_gate_miss.get(), 1);
}

/// The address of province `p`'s active-table entry window in the fixture.
fn active_window_addr(province: u64) -> u64 {
    ACTIVE_TABLE
        + province * crate::attach_log::D81_ACTIVE_ENTRY_STRIDE as u64
        + crate::attach_log::D81_ACTIVE_WINDOW_OFFSET as u64
}

/// A MOVING COUNTER IN THE ACTIVE TABLE DOES NOT COST A WALK.
///
/// `d81_active_state_from_window` reduces three of the four fields to
/// zero/nonzero because "the producer applies zero/nonzero predicates" -- so a
/// counter that ticks from one nonzero value to another is, to every d81
/// reader, the same state. Byte comparison disagreed, and that single extent
/// was the last thing standing between the gate and a hit
/// (`captures/pw_gate4`: 203 of 203 misses differed in exactly one interval of
/// exactly 429,248 bytes, which is `13,414 * 0x20`).
///
/// kills: comparing the active extent by raw bytes; widening the fed window
/// past `[+0x12, +0x19)`; feeding the raw `active_u16` instead of its
/// zero/nonzero reduction.
#[test]
fn a_moving_counter_in_the_active_table_still_hits() {
    let mut gate = D81GeometryGate::default();
    let first = stage_world(WorldShape::default());
    let walked = observe_and_remember(&mut gate, &first);
    let mut ticked = stage_world(WorldShape::default());
    // PROVINCE 2, and the choice is load-bearing for its sibling test below.
    // `active_terminal` is `active_a || active_b || active_u16 || flags&1`, and
    // the fixture gives province p `flags = p % 2` and `active_a = (p % 5 != 0)`.
    // An EVEN province that is not a multiple of five therefore has exactly one
    // live term, so byte 2 alone decides its row. Here it moves 0x5A -> 0x33:
    // still nonzero, same predicate, different byte.
    ticked.memory.write(active_window_addr(2) + 2, &[0x33]);
    let phases = D81PhaseAccum::default();
    match ask(&mut gate, &ticked, &phases) {
        D81GeometryVerdict::Reuse(rows, _) => {
            assert_eq!(rows.provinces, walked.provinces);
        }
        D81GeometryVerdict::Walk => {
            panic!("a counter tick no reader can observe must not cost a walk")
        }
    }
    assert_eq!(phases.geometry_gate_hit.get(), 1);
}

/// BUT A PREDICATE THAT FLIPS IS A MISS.
///
/// The companion, and the one that makes the test above safe: the reduction is
/// zero/nonzero, so nonzero -> ZERO is a different state and must refuse. Both
/// tests write the same byte; only the value differs.
///
/// kills: feeding a constant instead of the reduction; skipping the active
/// extent entirely rather than comparing its semantic state.
#[test]
fn an_active_table_predicate_that_flips_is_a_miss() {
    let mut gate = D81GeometryGate::default();
    let first = stage_world(WorldShape::default());
    let walked = observe_and_remember(&mut gate, &first);
    let mut flipped = stage_world(WorldShape::default());
    flipped.memory.write(active_window_addr(2) + 2, &[0x00]);
    let phases = D81PhaseAccum::default();
    match ask(&mut gate, &flipped, &phases) {
        D81GeometryVerdict::Walk => {}
        D81GeometryVerdict::Reuse(..) => {
            panic!("nonzero -> zero is a state change the walk publishes; it must not reuse")
        }
    }
    assert_eq!(phases.geometry_gate_miss.get(), 1);
    // AND THE FLIP IS REAL: a walk over the flipped world publishes different
    // rows, so the miss above protected an observable difference.
    let mut cold = D81GeometryGate::default();
    let rewalked = observe_and_remember(&mut cold, &flipped);
    assert_ne!(
        rewalked.provinces, walked.provinces,
        "the predicate flip must change the published rows, or this proves nothing"
    );
}


/// THE IMAGE COVERS EVERY SAMPLE IMAGE, not just an opening/closing pair
/// (Codex r1, finding 1).
///
/// The recipe takes ADJACENT SAMPLES - two per ordinary interval, three across
/// the active table - and the walk reads through `u*_stable_at`, which compares
/// an image against its PAIRED sample. Serializing only `bytes` and
/// `closing_bytes` left those samples out of the key, so two observations whose
/// opening and closing agreed could differ in a sample: the gate would hit
/// while a full walk saw the disagreement and REFUSED.
/// `DisagreeAction::Record` means such a disagreement need not gap the
/// consumer, so `copy_is_clean` did not close it either.
///
/// This asserts coverage structurally, which is deterministic where scripting a
/// mid-copy edit into exactly one sample is not: the serialized image must be
/// at least the sum over records of every held image's length. Reverting to a
/// single pair drops the total below the bound.
///
/// kills: replacing the `0..opening_images` / `0..closing_images` loops with a
/// single `view.bytes` / `closing_bytes` pair.
#[test]
fn the_image_covers_every_sample_image() {
    let world = stage_world(WorldShape::default());
    let mut plan = D81PlanCounters::default();
    let memory = CountingSnapshotMemory::new(&world.memory);
    let observation = d81_observe_copy_plan_with(&memory, &world.roots, WORLD_HOUR, &mut plan)
        .expect("custody admits a fixture this size");
    let snapshot = &observation.closed.snapshot;
    let view = snapshot
        .view_for(D81_CONSUMER, WORLD_HOUR)
        .expect("view issues for the plan's own hour");

    let chain = chain_windows(&world);
    let mut expected_bytes = 0u64;
    let mut multi_sample_records = 0usize;
    for record in snapshot.intervals() {
        let start = record.start();
        let len = record.len();
        let opening = view.sample_count(start);
        let closing = if record.closing_offset().is_some() {
            snapshot
                .annotations()
                .closing_images(record.id())
                .map(|images| images.held())
                .unwrap_or(1)
        } else {
            0
        };
        if opening + closing > 2 {
            multi_sample_records += 1;
        }
        // A skipped chain prefix contributes no content, and the active table
        // contributes its reduction rather than its bytes; neither is counted
        // into the lower bound.
        let skipped = chain.iter().any(|(base, plen)| *base as u64 == start && *plen == len);
        let reduced = start == ACTIVE_TABLE;
        if !skipped && !reduced {
            expected_bytes += len * (opening + closing) as u64;
        }
    }
    assert!(
        multi_sample_records > 0,
        "the fixture must hold at least one record with more than one image, or this \
         bound is satisfied by an opening/closing pair and proves nothing"
    );

    let mut gate = D81GeometryGate::default();
    let _ = observe_and_remember(&mut gate, &world);
    assert!(
        gate.image_len() as u64 >= expected_bytes,
        "the memo image ({} bytes) must cover every held sample image ({} bytes of \
         non-skipped, non-reduced content)",
        gate.image_len(),
        expected_bytes
    );
}

/// A RECORD THAT ALSO SERVES ANOTHER ROLE IS NEVER SKIPPED OR REDUCED
/// (Codex r1 #2, corrected by r2 #1, corrected again by r3 #1).
///
/// Three forms of this guard were wrong before this one, and the test carries
/// all three shapes so none can come back:
///
///  * overlap against other RECORDS is a tautology - the ownership map emits
///    only non-overlapping records;
///  * an exact-duplicate request creates NO new record, so one record serves
///    two roles;
///  * an extent decoded later can CONTAIN an intact earlier record while
///    beginning before it, which a base-only test never sees.
///
/// `record_holds_one_role` answers the question the exclusions actually need,
/// against the plan's pre-dedup spans, and it is a pure function so these
/// geometries can be asked about directly.
///
/// kills: dropping the guard; testing `overlaps` with the wrong strictness;
/// accepting a record no span accounts for; forgetting that the role's own span
/// must actually appear in the list.
#[test]
fn a_record_that_also_serves_another_role_is_not_skipped() {
    use crate::attach_log::d81_geometry_gate::record_holds_one_role;
    const T: u64 = 0x2_0000_0000;

    // The clean case: a prefix the plan owns and nothing else touches.
    let clean = [(T, T + 888), (T + 0x1000, T + 0x2000)];
    assert!(record_holds_one_role(&clean, T, 888, (T, T + 888)));

    // r2's shape, and the one r4 caught the previous test NOT covering: an
    // EXACT duplicate. Two roles whose spans coincide exactly - the owner
    // prefix at 888 bytes and a selector extent of (31 + 4 + 2) * 0x18 = 888
    // over the same base. Identical bounds, so no bounds comparison can tell
    // them apart; only the overlap COUNT can.
    let exact_duplicate = [(T, T + 888), (T, T + 888)];
    assert!(
        !record_holds_one_role(&exact_duplicate, T, 888, (T, T + 888)),
        "two roles with identical bounds must disqualify the skip"
    );
    // A same-base overlap with different bounds, which the earlier test used.
    let same_base = [(T, T + 888), (T, T + 0x6C0)];
    assert!(!record_holds_one_role(&same_base, T, 888, (T, T + 888)));

    // r3's shape: a CONTAINING extent that begins BEFORE the record.
    let containing = [(T, T + 0x6A0), (T - 0x18, T - 0x18 + 0x6C0)];
    assert!(
        !record_holds_one_role(&containing, T, 0x6A0, (T, T + 0x6A0)),
        "an extent that begins before the record and contains it must be seen"
    );

    // A span that merely abuts is not an overlap.
    let abutting = [(T, T + 888), (T + 888, T + 1000)];
    assert!(record_holds_one_role(&abutting, T, 888, (T, T + 888)));

    // r5's shape: a ZERO-LENGTH role coincident with the record's start. Its
    // count came out zero from a copied header, but the operation is real and
    // the walk can still read through the record covering its base, because
    // snapshot ownership is consumer-level rather than per-operation. Compared
    // by the ordinary half-open rule it vanishes exactly here, which is where a
    // second role would hide.
    let zero_at_start = [(T, T + 64), (T, T)];
    assert!(
        !record_holds_one_role(&zero_at_start, T, 64, (T, T + 64)),
        "a zero-length role at the record start must still be counted"
    );
    // And inside it, and at its far edge (conservative: over-counting only
    // costs a walk).
    assert!(!record_holds_one_role(&[(T, T + 64), (T + 8, T + 8)], T, 64, (T, T + 64)));
    assert!(!record_holds_one_role(&[(T, T + 64), (T + 64, T + 64)], T, 64, (T, T + 64)));
    // A zero-length span well outside the record does not disqualify it.
    assert!(record_holds_one_role(&[(T, T + 64), (T + 0x9000, T + 0x9000)], T, 64, (T, T + 64)));

    // A record the plan does not claim at all is not proven to be anything.
    let unclaimed = [(T + 0x5000, T + 0x6000)];
    assert!(!record_holds_one_role(&unclaimed, T, 888, (T, T + 888)));
    assert!(!record_holds_one_role(&[], T, 888, (T, T + 888)));

    // And the list must hold each span ONCE: a caller that unions in a span
    // `bulk_spans` already carries turns every ordinary record into a
    // two-overlap refusal, which is safe but silently disables the gate. The
    // production call site is what keeps this true; this pins the consequence
    // so a change there fails here rather than in a capture.
    let doubled = [(T, T + 888), (T, T + 888)];
    assert!(!record_holds_one_role(&doubled, T, 888, (T, T + 888)));
}
