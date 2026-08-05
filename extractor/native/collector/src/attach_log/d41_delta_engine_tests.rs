//! `d41_delta_engine_tests`: the shared diff engine's output is a wire
//! contract, and this is what holds it still while the data structure under it
//! changes.
//!
//! WHY THE ENGINE WAS REWRITTEN. `compact_projects_delta_rows` is not d41's
//! alone — it is the generic keyed-diff every candidate delta path in the
//! producer is priced against. It used to hold both the pinned baseline and the
//! frame's rows in `BTreeMap`s, paying a node allocation and a rebalance for
//! every row of every frame. On this machine that cost 173-260 ns/row, and that
//! is not a rounding error against the alternative: a domain that re-sends a
//! whole payload spends nothing on a diff at all, so a slow diff is exactly the
//! term that decides whether a domain's delta path is a net win. d53's 389,527
//! rows alone spend 46-68 ms per frame in it, which is the whole of that
//! domain's measured delta regression.
//!
//! WHAT MAY NOT CHANGE. The emitted `Vec<R>` is decoded positionally by the
//! fold, so the engine's output is a wire contract in two halves:
//!   * the observed rows, in WALK order, filtered to those that differ from the
//!     pinned baseline (all of them on a keyframe);
//!   * one tombstone per baseline key the frame did not observe, in ASCENDING
//!     KEY order.
//! and the baseline it leaves behind must hold, for every key, the row a
//! repeated `BTreeMap::insert` over the frame's rows would have left there —
//! the LAST occurrence, because a section may legitimately emit a key twice in
//! one walk and the next frame's verdict must not depend on which copy was
//! kept.
//!
//! HOW THAT IS ENFORCED. `reference_delta_rows` below is a frozen, character-
//! for-character transcription of the `BTreeMap` engine as it stood at
//! 594bb829. The differential tests run it and the production engine side by
//! side over the same randomized frame sequences, each keeping its own
//! baseline, and compare the emitted rows element for element, the
//! `changed_rows` counter, and the two baselines' contents after every frame.
//! A rewrite that reorders, dedupes or drops anything fails here rather than in
//! a capture.
//!
//! `d41_delta_engine_bench` is an instrument, not a gate: it is `#[ignore]`d
//! and runs both engines over the same row-count ladder so the before/after
//! curve comes out of one process. Run it with
//! `cargo test -p collector --release -- --ignored --nocapture d41_delta_engine_bench`.

use super::*;
use std::collections::BTreeMap;

/// FROZEN. The `BTreeMap` engine exactly as `compact_projects_delta_rows` stood
/// before the sorted-vector rewrite (594bb829,
/// `collector/src/attach_log.rs:79558`). This is the differential tests' oracle
/// and it must not be "improved", refactored or shared with production code —
/// the moment it stops being an independent second implementation, the
/// comparison below stops proving anything.
fn reference_delta_rows<K, R, FKey, FDelete>(
    rows: &[R],
    previous: &BTreeMap<K, R>,
    current: &mut BTreeMap<K, R>,
    baseline: bool,
    key_fn: FKey,
    deleted_row_fn: FDelete,
    changed_rows: &mut u32,
) -> Vec<R>
where
    K: Copy + Ord,
    R: Clone + PartialEq,
    FKey: Fn(&R) -> K,
    FDelete: Fn(K) -> R,
{
    let mut emitted = Vec::new();
    for row in rows {
        let key = key_fn(row);
        let changed = previous.get(&key).map(|old| old != row).unwrap_or(true);
        current.insert(key, row.clone());
        if baseline || changed {
            emitted.push(row.clone());
            if !baseline {
                *changed_rows = changed_rows.saturating_add(1);
            }
        }
    }
    if !baseline {
        for key in previous.keys().copied() {
            if !current.contains_key(&key) {
                emitted.push(deleted_row_fn(key));
                *changed_rows = changed_rows.saturating_add(1);
            }
        }
    }
    emitted
}

/// xorshift64*, so a failing seed is reproducible from the test name alone and
/// the generator never becomes a dependency.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        // 0 is the fixed point of xorshift; no seed may reach it.
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: u32) -> u32 {
        if bound == 0 {
            return 0;
        }
        (self.next_u64() % bound as u64) as u32
    }

    fn chance(&mut self, one_in: u32) -> bool {
        self.below(one_in.max(1)) == 0
    }
}

// --- row shapes ---------------------------------------------------------
//
// Two real production row types, one per key shape the engine is instantiated
// with in `compact_projects_apply_emit_mode`:
//   * `ProjectRewardHistoryRow` — a `(u32, u32, u32)` key and a wide all-scalar
//     payload; this is the section that carries most of d41's bytes.
//   * `ProjectFacilityRow` — a `u32` key and a payload holding an owned
//     `Option<String>`, so a row whose clone is not a memcpy is covered too.

fn history_row(key: (u32, u32, u32), payload: u32) -> ProjectRewardHistoryRow {
    ProjectRewardHistoryRow {
        tag_db_id: key.0,
        local_id: key.1,
        history_index: key.2,
        prototype_reward_handle: payload,
        option_handle: payload ^ 0x5555_5555,
        province: payload as i32,
        progress_raw: payload.wrapping_mul(7),
        character_ref_type: payload & 3,
        character_ref_id: payload >> 2,
        country_ref: 0,
        scientist_ref: 0,
        state_ref: 0,
        date_raw: payload as i32 >> 1,
        country_present: payload & 1 == 1,
        scientist_present: payload & 2 == 2,
        state_present: payload & 4 == 4,
        deleted: false,
    }
}

fn facility_row(id: u32, payload: u32) -> ProjectFacilityRow {
    ProjectFacilityRow {
        id,
        province: payload,
        building: if payload & 1 == 0 {
            None
        } else {
            Some(format!("building_{payload}"))
        },
        active_project_handle: payload.wrapping_mul(3),
        active: payload & 2 == 2,
        scientist_char_id: if payload & 4 == 0 {
            None
        } else {
            Some(payload >> 3)
        },
        unread_prototype_rewards: payload % 11,
        deleted: false,
    }
}

fn history_key(row: &ProjectRewardHistoryRow) -> (u32, u32, u32) {
    (row.tag_db_id, row.local_id, row.history_index)
}

/// The two rows a caller compares must be equal field for field.
/// `ProjectFacilityRow` carries no `Debug`, so a mismatch is reported by
/// position and key rather than by value.
fn assert_facility_rows_equal(left: &[ProjectFacilityRow], right: &[ProjectFacilityRow], at: usize) {
    assert_eq!(
        left.len(),
        right.len(),
        "frame {at}: production emitted {} rows, the frozen BTreeMap engine emitted {}",
        left.len(),
        right.len()
    );
    for (index, (produced, expected)) in left.iter().zip(right.iter()).enumerate() {
        assert!(
            produced == expected,
            "frame {at}: emitted row {index} differs from the frozen BTreeMap engine \
             (production id={}, deleted={}; reference id={}, deleted={})",
            produced.id,
            produced.deleted,
            expected.id,
            expected.deleted
        );
    }
}

// --- differential drivers -----------------------------------------------

/// Drive both engines over the same frame sequence and compare everything
/// observable after every frame: the emitted rows, the `changed_rows` counter,
/// and the baseline each engine leaves behind.
fn run_history_differential(seed: u64, frames: usize, key_pool: usize, description: &str) {
    let mut rng = Rng::new(seed);
    let keys: Vec<(u32, u32, u32)> = (0..key_pool)
        .map(|index| {
            (
                (index as u32 % 7) + 1,
                (index as u32 / 7) + 1,
                index as u32 % 3,
            )
        })
        .collect();

    let mut production = SortedRowMap::<(u32, u32, u32), ProjectRewardHistoryRow>::default();
    let mut reference: BTreeMap<(u32, u32, u32), ProjectRewardHistoryRow> = BTreeMap::new();
    let mut scratch: Vec<((u32, u32, u32), usize)> = Vec::new();

    for frame in 0..frames {
        // Frame 0 is the keyframe, exactly as `compact_projects_apply_emit_mode`
        // seeds it; every later frame is a patch.
        let baseline = frame == 0;
        let mut rows: Vec<ProjectRewardHistoryRow> = Vec::new();
        for key in &keys {
            if rng.chance(4) {
                continue;
            }
            rows.push(history_row(*key, rng.below(6)));
            // A section may emit the same key twice in one walk; the engines
            // must agree on which copy the baseline keeps.
            if rng.chance(9) {
                rows.push(history_row(*key, rng.below(6)));
            }
        }
        // Walk order is not key order in the reader, so it must not be here.
        for index in (1..rows.len()).rev() {
            let swap = rng.below(index as u32 + 1) as usize;
            rows.swap(index, swap);
        }

        let mut production_changed = 0u32;
        let mut production_next = SortedRowMap::default();
        let produced = compact_projects_delta_rows(
            &rows,
            &production,
            &mut production_next,
            baseline,
            history_key,
            deleted_project_reward_history_row,
            &mut production_changed,
            &mut scratch,
        );

        let mut reference_changed = 0u32;
        let mut reference_next: BTreeMap<(u32, u32, u32), ProjectRewardHistoryRow> = BTreeMap::new();
        let expected = reference_delta_rows(
            &rows,
            &reference,
            &mut reference_next,
            baseline,
            history_key,
            deleted_project_reward_history_row,
            &mut reference_changed,
        );

        assert_eq!(
            produced, expected,
            "{description}: frame {frame} emitted a different row sequence than the \
             frozen BTreeMap engine"
        );
        assert_eq!(
            production_changed, reference_changed,
            "{description}: frame {frame} counted {production_changed} changed rows, \
             the frozen BTreeMap engine counted {reference_changed}"
        );
        assert_eq!(
            production_next.keys,
            reference_next.keys().copied().collect::<Vec<_>>(),
            "{description}: frame {frame} left a baseline with different keys than the \
             frozen BTreeMap engine"
        );
        assert_eq!(
            production_next.rows,
            reference_next.values().copied().collect::<Vec<_>>(),
            "{description}: frame {frame} left a baseline with different rows than the \
             frozen BTreeMap engine (last-write-wins on a repeated key)"
        );

        production = production_next;
        reference = reference_next;
    }
}

#[test]
fn d41_delta_engine_differential_matches_the_frozen_btreemap_engine() {
    // Several independent seeds, because one sequence is one sample of the
    // insert/skip/duplicate interleavings and the defect classes here
    // (ordering, dedup side, tombstone order) do not all show up in every one.
    for seed in [1u64, 0x9E37_79B9, 0xDEAD_BEEF, 12_345, 0xFFFF_FFFF_FFFF] {
        run_history_differential(seed, 40, 21, "randomized frames");
    }
}

#[test]
fn d41_delta_engine_differential_survives_a_key_pool_larger_than_one_frame() {
    // A wide pool with a high skip rate is where tombstoning dominates: most
    // baseline keys are absent from most frames, so the ascending-key tombstone
    // sweep carries the output.
    run_history_differential(0x5EED, 30, 200, "wide key pool");
}

#[test]
fn d41_delta_engine_differential_covers_an_owned_payload_row() {
    // `ProjectFacilityRow` owns an `Option<String>`, so its clone is not a
    // memcpy and its `PartialEq` is not a bitwise compare. The engine must not
    // depend on either.
    let mut rng = Rng::new(0xC0FFEE);
    let mut production = SortedRowMap::<u32, ProjectFacilityRow>::default();
    let mut reference: BTreeMap<u32, ProjectFacilityRow> = BTreeMap::new();
    let mut scratch: Vec<(u32, usize)> = Vec::new();

    for frame in 0..25usize {
        let baseline = frame == 0;
        let mut rows = Vec::new();
        for id in 1..18u32 {
            if rng.chance(3) {
                continue;
            }
            rows.push(facility_row(id, rng.below(5)));
        }
        for index in (1..rows.len()).rev() {
            let swap = rng.below(index as u32 + 1) as usize;
            rows.swap(index, swap);
        }

        let mut production_changed = 0u32;
        let mut production_next = SortedRowMap::default();
        let produced = compact_projects_delta_rows(
            &rows,
            &production,
            &mut production_next,
            baseline,
            |row| row.id,
            deleted_project_facility_row,
            &mut production_changed,
            &mut scratch,
        );

        let mut reference_changed = 0u32;
        let mut reference_next: BTreeMap<u32, ProjectFacilityRow> = BTreeMap::new();
        let expected = reference_delta_rows(
            &rows,
            &reference,
            &mut reference_next,
            baseline,
            |row| row.id,
            deleted_project_facility_row,
            &mut reference_changed,
        );

        assert_facility_rows_equal(&produced, &expected, frame);
        assert_eq!(
            production_changed, reference_changed,
            "frame {frame}: changed_rows disagreed with the frozen BTreeMap engine"
        );
        assert_eq!(
            production_next.keys,
            reference_next.keys().copied().collect::<Vec<_>>(),
            "frame {frame}: the baseline's keys disagreed with the frozen BTreeMap engine"
        );
        assert_facility_rows_equal(
            &production_next.rows,
            &reference_next.values().cloned().collect::<Vec<_>>(),
            frame,
        );

        production = production_next;
        reference = reference_next;
    }
}

#[test]
fn d41_delta_engine_emits_observed_rows_in_walk_order_and_tombstones_in_key_order() {
    // The differential tests prove agreement with the old engine; this one
    // states the contract in its own right, so a change that breaks BOTH
    // engines the same way still fails something.
    let mut baseline_map = SortedRowMap::<(u32, u32, u32), ProjectRewardHistoryRow>::default();
    let mut scratch = Vec::new();
    let mut changed = 0u32;
    let seed_rows = vec![
        history_row((9, 1, 0), 1),
        history_row((2, 1, 0), 1),
        history_row((5, 1, 0), 1),
        history_row((7, 1, 0), 1),
    ];
    let mut seeded = SortedRowMap::default();
    let _ = compact_projects_delta_rows(
        &seed_rows,
        &baseline_map,
        &mut seeded,
        true,
        history_key,
        deleted_project_reward_history_row,
        &mut changed,
        &mut scratch,
    );
    baseline_map = seeded;
    assert_eq!(
        baseline_map.keys,
        vec![(2, 1, 0), (5, 1, 0), (7, 1, 0), (9, 1, 0)],
        "a keyframe must leave an ascending baseline whatever the walk order"
    );

    // Frame 2 changes 9 and 2 (in that walk order), leaves 5 untouched and
    // drops 7 entirely.
    let rows = vec![
        history_row((9, 1, 0), 4),
        history_row((5, 1, 0), 1),
        history_row((2, 1, 0), 4),
    ];
    let mut next = SortedRowMap::default();
    let mut changed = 0u32;
    let emitted = compact_projects_delta_rows(
        &rows,
        &baseline_map,
        &mut next,
        false,
        history_key,
        deleted_project_reward_history_row,
        &mut changed,
        &mut scratch,
    );
    assert_eq!(
        emitted,
        vec![
            history_row((9, 1, 0), 4),
            history_row((2, 1, 0), 4),
            deleted_project_reward_history_row((7, 1, 0)),
        ],
        "changed rows come first in WALK order, then tombstones in ASCENDING KEY order"
    );
    assert_eq!(changed, 3, "two changed rows and one tombstone");
}

#[test]
fn d41_delta_engine_baseline_keeps_the_last_row_of_a_repeated_key() {
    // The map this replaced was filled with `insert`, so a repeated key left
    // the LAST row pinned. Both copies are still emitted on a keyframe.
    let empty = SortedRowMap::<(u32, u32, u32), ProjectRewardHistoryRow>::default();
    let mut next = SortedRowMap::default();
    let mut scratch = Vec::new();
    let mut changed = 0u32;
    let rows = vec![
        history_row((3, 1, 0), 1),
        history_row((3, 1, 0), 2),
        history_row((3, 1, 0), 3),
    ];
    let emitted = compact_projects_delta_rows(
        &rows,
        &empty,
        &mut next,
        true,
        history_key,
        deleted_project_reward_history_row,
        &mut changed,
        &mut scratch,
    );
    assert_eq!(emitted.len(), 3, "a keyframe emits every observed row");
    assert_eq!(next.keys, vec![(3, 1, 0)]);
    assert_eq!(
        next.rows,
        vec![history_row((3, 1, 0), 3)],
        "the LAST occurrence is the one the baseline keeps"
    );
}

#[test]
fn d41_delta_engine_cursor_lookup_agrees_with_binary_search() {
    // `get_with_cursor` carries state ACROSS lookups, so unlike `get` it can be
    // wrong as a function of the order it is asked in rather than of any single
    // key. This drives it against `get` — the plain binary search — over the
    // orders that would break a hint that is trusted without proof: strictly
    // ascending, strictly descending, random, all-absent, and an alternating
    // walk that steps far forward and then far back.
    //
    // MUTATION: drop the `keys[*cursor - 1] < *key` guard and use the hint
    // unconditionally. The descending and alternating orders then return `None`
    // for keys the baseline holds, because the cursor is already past them.
    let mut baseline = SortedRowMap::<(u32, u32, u32), ProjectRewardHistoryRow>::default();
    let mut scratch = Vec::new();
    let mut changed = 0u32;
    // Deliberately gappy, so roughly half of every probe set is absent and the
    // "not found" cursor updates are exercised as hard as the "found" ones.
    let held: Vec<(u32, u32, u32)> = (0..300u32).map(|i| (i / 10, (i % 10) * 2, 0)).collect();
    let rows: Vec<ProjectRewardHistoryRow> =
        held.iter().map(|key| history_row(*key, 1)).collect();
    let mut seeded = SortedRowMap::default();
    let _ = compact_projects_delta_rows(
        &rows,
        &baseline,
        &mut seeded,
        true,
        history_key,
        deleted_project_reward_history_row,
        &mut changed,
        &mut scratch,
    );
    baseline = seeded;

    let mut probes: Vec<(u32, u32, u32)> = (0..300u32).map(|i| (i / 10, i % 10, 0)).collect();
    let ascending = probes.clone();
    let mut descending = probes.clone();
    descending.reverse();
    let mut random = probes.clone();
    let mut rng = Rng::new(0xA11CE);
    for index in (1..random.len()).rev() {
        let swap = rng.below(index as u32 + 1) as usize;
        random.swap(index, swap);
    }
    let mut alternating = Vec::new();
    for index in 0..probes.len() / 2 {
        alternating.push(probes[index]);
        alternating.push(probes[probes.len() - 1 - index]);
    }
    let absent: Vec<(u32, u32, u32)> = (0..300u32).map(|i| (9000 + i, 0, 0)).collect();
    probes.clear();

    for (order_name, order) in [
        ("ascending", &ascending),
        ("descending", &descending),
        ("random", &random),
        ("alternating", &alternating),
        ("all absent", &absent),
    ] {
        let mut cursor = 0usize;
        for (index, key) in order.iter().enumerate() {
            let hinted = baseline.get_with_cursor(&mut cursor, key).copied();
            let searched = baseline.get(key).copied();
            assert_eq!(
                hinted, searched,
                "{order_name} probe {index} for key {key:?}: the cursor lookup disagreed \
                 with the binary search"
            );
            assert!(
                cursor <= baseline.len(),
                "{order_name} probe {index}: the cursor left the array"
            );
        }
    }
}

#[test]
fn d41_delta_engine_scratch_reuse_does_not_leak_between_sections() {
    // Production hands ONE scratch buffer to every section that shares a key
    // shape. A rebuild that read stale scratch entries would corrupt the second
    // section's baseline, so drive two sections through one buffer and check
    // the second against a pristine buffer.
    let empty = SortedRowMap::<(u32, u32, u32), ProjectRewardHistoryRow>::default();
    let mut shared_scratch = Vec::new();
    let mut changed = 0u32;

    let wide: Vec<ProjectRewardHistoryRow> =
        (0..40u32).map(|i| history_row((i, 1, 0), i)).collect();
    let mut first = SortedRowMap::default();
    let _ = compact_projects_delta_rows(
        &wide,
        &empty,
        &mut first,
        true,
        history_key,
        deleted_project_reward_history_row,
        &mut changed,
        &mut shared_scratch,
    );

    let narrow = vec![history_row((99, 1, 0), 5), history_row((98, 1, 0), 5)];
    let mut second_shared = SortedRowMap::default();
    let _ = compact_projects_delta_rows(
        &narrow,
        &empty,
        &mut second_shared,
        true,
        history_key,
        deleted_project_reward_history_row,
        &mut changed,
        &mut shared_scratch,
    );
    let mut pristine_scratch = Vec::new();
    let mut second_pristine = SortedRowMap::default();
    let _ = compact_projects_delta_rows(
        &narrow,
        &empty,
        &mut second_pristine,
        true,
        history_key,
        deleted_project_reward_history_row,
        &mut changed,
        &mut pristine_scratch,
    );
    assert_eq!(
        second_shared.keys, second_pristine.keys,
        "a reused scratch buffer must leave the same baseline as a fresh one"
    );
    assert_eq!(second_shared.rows, second_pristine.rows);
}

// --- instrument ---------------------------------------------------------

/// The row-count ladder the `BTreeMap` engine was measured on, plus d53's own
/// per-frame row count, which is the number that decided that domain's delta
/// verdict.
const DELTA_ENGINE_LADDER: [usize; 5] = [42_728, 85_456, 170_912, 341_824, 389_527];

/// Best-of-5, because this is a shared machine and the interesting quantity is
/// the floor, not the mean of whatever else was running.
const DELTA_ENGINE_TRIALS: usize = 5;

#[test]
#[ignore = "instrument, not a gate; run with --ignored --nocapture"]
fn d41_delta_engine_bench() {
    println!("rows,btreemap_ms,btreemap_ns_per_row,sorted_ms,sorted_ns_per_row,speedup");
    for row_count in DELTA_ENGINE_LADDER {
        // Steady state, which is the production shape: the baseline is already
        // pinned and the frame changes a small minority of rows.
        let seed_rows: Vec<ProjectRewardHistoryRow> = (0..row_count as u32)
            .map(|index| history_row((index / 64, index % 64, index % 5), index))
            .collect();
        let frame_rows: Vec<ProjectRewardHistoryRow> = seed_rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                if index % 100 == 0 {
                    history_row(history_key(row), index as u32 + 1)
                } else {
                    *row
                }
            })
            .collect();

        let mut reference_baseline: BTreeMap<(u32, u32, u32), ProjectRewardHistoryRow> =
            BTreeMap::new();
        let mut changed = 0u32;
        let mut seeded = BTreeMap::new();
        let _ = reference_delta_rows(
            &seed_rows,
            &reference_baseline,
            &mut seeded,
            true,
            history_key,
            deleted_project_reward_history_row,
            &mut changed,
        );
        reference_baseline = seeded;

        let mut production_baseline = SortedRowMap::default();
        let mut scratch = Vec::new();
        let mut seeded = SortedRowMap::default();
        let _ = compact_projects_delta_rows(
            &seed_rows,
            &production_baseline,
            &mut seeded,
            true,
            history_key,
            deleted_project_reward_history_row,
            &mut changed,
            &mut scratch,
        );
        production_baseline = seeded;

        let mut btreemap_best = Duration::from_secs(3600);
        let mut sorted_best = Duration::from_secs(3600);
        for _ in 0..DELTA_ENGINE_TRIALS {
            let mut next = BTreeMap::new();
            let mut changed = 0u32;
            let started = Instant::now();
            let emitted = reference_delta_rows(
                &frame_rows,
                &reference_baseline,
                &mut next,
                false,
                history_key,
                deleted_project_reward_history_row,
                &mut changed,
            );
            let elapsed = started.elapsed();
            // Consume the output so the optimizer cannot delete the work.
            assert!(!emitted.is_empty());
            btreemap_best = btreemap_best.min(elapsed);

            let mut next = SortedRowMap::default();
            let mut changed = 0u32;
            let started = Instant::now();
            let emitted = compact_projects_delta_rows(
                &frame_rows,
                &production_baseline,
                &mut next,
                false,
                history_key,
                deleted_project_reward_history_row,
                &mut changed,
                &mut scratch,
            );
            let elapsed = started.elapsed();
            assert!(!emitted.is_empty());
            sorted_best = sorted_best.min(elapsed);
        }

        let btreemap_ns = btreemap_best.as_nanos() as f64 / row_count as f64;
        let sorted_ns = sorted_best.as_nanos() as f64 / row_count as f64;
        println!(
            "{row_count},{:.3},{:.1},{:.3},{:.1},{:.2}x",
            btreemap_best.as_secs_f64() * 1000.0,
            btreemap_ns,
            sorted_best.as_secs_f64() * 1000.0,
            sorted_ns,
            btreemap_ns / sorted_ns
        );
    }
}
