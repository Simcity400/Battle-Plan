//! Executor overhead instrument for the snapshot-recipe substrate.
//!
//! WHAT THIS MEASURES. The substrate's OWN cost per copy transaction: recipe
//! expansion, per-wave coalescing, global interval ownership and dedup
//! lookups, arena allocation, dependency and edge bookkeeping, freezing, and
//! the closing-witness wave. The bytes move through
//! [`super::synthetic::SyntheticMemory`], which is a bounds check plus a
//! `memcpy` out of a `Vec`.
//!
//! WHAT THIS DOES NOT MEASURE, and must not be read as measuring:
//!
//!  * The cost of a REAL copy. A live observation copies through the
//!    SEH-guarded primitive against another process's pages, with region
//!    validation and cache misses this fixture does not have. The floor's
//!    40–140 ns/range scatter cost is that number; this bench deliberately
//!    excludes it so the substrate's own overhead is visible on its own.
//!  * Any wall-clock claim about an hour observation. There is no game, no
//!    contention, no worker pool, and the fixture heap is one contiguous
//!    region, so ownership lookups hit a much friendlier BTree than a live
//!    504k-range image would.
//!
//! HOW TO READ IT AGAINST THE FLOOR. The re-priced floor budgets
//! **200k–360k copy transactions at 3–8 ms dev / 8–24 ms on the 4c/8GB floor
//! machine** for the whole movement, INCLUDING the real per-range scatter
//! cost. The `substrate ns/txn` figure below is what the recipe engine adds on
//! top of that per transaction; the `baseline` row is the same transactions
//! issued straight at the fixture with no substrate at all, so the difference
//! is the honest overhead.
//!
//! WHERE THE COST IS. `bench_component_attribution` breaks the per-transaction
//! cost down, one row per component, each running the SAME 100k operations the
//! flat observation runs.
//!
//! The first version of the substrate performed FIVE ordered-map traversals
//! per copy transaction — two range queries to plan the acquisition, one
//! insert, two more to attach dependents and edges — plus one more for the
//! decode-phase parent lookup and a `BTreeSet` insert for the discovery
//! visited set. At 100k owned intervals those tree walks, not the byte
//! movement and not the allocations, were three quarters of the number. Global
//! deduplication is a hard requirement (t35's per-bracket sum is nearly twice
//! its union), so what changed is the structure behind it, not the guarantee:
//!
//!  * the ownership index is address-sorted arrays swept by a [`Cursor`], and
//!    the executor's two per-wave passes are sweeps rather than lookups
//!    (`OwnershipMap: sweep plan + insert`, `sweep overlap + attach` below —
//!    the `bisecting` rows are the same work with a fresh cursor per query,
//!    i.e. what the old probe path cost shape looked like);
//!  * the decode-phase parent lookup is gone — it fed a field nothing read;
//!  * the visited set is hashed rather than ordered.
//!
//! `bench_index_fold_cost` prices what the sweep buys the fold with: the index
//! is re-merged once per wave, and that row is the cost of doing so.
//!
//! HOW TO READ A NUMBER OFF IT. One invocation is one sample, and this box is
//! shared with other work, so a single sample is load as much as it is
//! measurement: rows have been seen to move by more than 2x between runs on a
//! 70%-busy machine. Quote the PER-ROW MINIMUM over at least five invocations
//! with `--test-threads=1` (the default parallel harness runs the benches
//! against each other). The minimum is the least-contended sample of each row;
//! it flatters nothing, because the same rule has to be applied to both sides
//! of any comparison, and it must be stated alongside the number.
//!
//! AND WHICH ROW. `substrate: opening image only` runs `flat_recipe`, which is
//! the sweep's best case in every respect: one recipe, one wave, a hundred
//! thousand roots already in address order, folding into an empty index.
//! `bench_observation_over_many_waves_and_recipes` is the same instrument on
//! shapes that are not, and it is the row to quote for what an observation
//! costs.
//!
//! These are `#[ignore]`d: an instrument, not a gate. Run with
//! `cargo test -p collector --release --lib -- --ignored --nocapture snapshot_recipes::bench`.

use std::time::Instant;

use super::executor::{observe, ObservationConfig, Recipe, RecipeOp, RecipeOutcome, Root};
use super::policy::{plan_copies, CopyRequest, GapPolicy, Provenance};
use super::synthetic::SyntheticMemory;
use super::{ConsumerId, HourStamp, RecipeId, Span, SnapshotMemory};

const BENCH_BASE: u64 = 0x2000_0000;
/// Distance between objects. Wider than every merge tier, so nothing
/// coalesces and the transaction count is exactly the object count.
const BENCH_STRIDE: u64 = 256;
const OBJECT_BYTES: u32 = 48;

fn bench_memory(objects: usize) -> SyntheticMemory {
    let mut memory = SyntheticMemory::new();
    let bytes = objects * BENCH_STRIDE as usize;
    let mut region = vec![0u8; bytes];
    for (index, byte) in region.iter_mut().enumerate() {
        *byte = (index % 251) as u8;
    }
    memory.map(BENCH_BASE, region);
    memory
}

/// One recipe with `objects` roots, each a flat `CopyHeader`. One wave, one
/// transaction per root: the widest-possible breadth at depth zero.
fn flat_recipe(objects: usize, witness: bool) -> Recipe {
    let ops = if witness {
        vec![
            RecipeOp::CopyHeader {
                size: OBJECT_BYTES,
                next: vec![1],
            },
            RecipeOp::WitnessClose { next: vec![2] },
            RecipeOp::Freeze,
        ]
    } else {
        vec![
            RecipeOp::CopyHeader {
                size: OBJECT_BYTES,
                next: vec![1],
            },
            RecipeOp::Freeze,
        ]
    };
    let roots = (0..objects)
        .map(|index| Root {
            op: 0,
            addr: BENCH_BASE + index as u64 * BENCH_STRIDE,
        })
        .collect();
    Recipe::new(RecipeId(1), ConsumerId(1), ops, roots)
}

fn report(label: &str, transactions: u64, bytes: u64, elapsed: std::time::Duration) {
    let millis = elapsed.as_secs_f64() * 1000.0;
    let per_txn = elapsed.as_nanos() as f64 / transactions.max(1) as f64;
    println!(
        "{label:<38} {transactions:>9} txn {:>9.3} MiB {millis:>9.3} ms {per_txn:>8.1} ns/txn",
        bytes as f64 / (1024.0 * 1024.0),
    );
}

#[test]
#[ignore = "instrument, not a gate: cargo test -p collector --release --lib -- --ignored --nocapture"]
fn bench_executor_overhead_at_one_hundred_thousand_ranges() {
    let objects = 100_000usize;
    let memory = bench_memory(objects);
    let bytes = objects as u64 * u64::from(OBJECT_BYTES);

    println!();
    println!("snapshot-recipe substrate, {objects} copy transactions of {OBJECT_BYTES} B");
    println!("  fixture: one contiguous region, {BENCH_STRIDE} B stride (no merge is possible)");

    // Baseline: the same transactions, issued straight at the fixture. This is
    // the floor under the substrate — everything above it is overhead.
    let mut sink = vec![0u8; OBJECT_BYTES as usize];
    let mut checksum = 0u64;
    let started = Instant::now();
    for index in 0..objects {
        memory
            .copy_into(BENCH_BASE + index as u64 * BENCH_STRIDE, &mut sink)
            .expect("fixture read");
        // CONSUMED, not merely written. A copy whose result is never read is
        // a copy the optimiser may delete, and a baseline that measures a
        // deleted copy makes every overhead figure above it too large.
        checksum = checksum.wrapping_add(u64::from(sink[index % sink.len()]));
    }
    let baseline = started.elapsed();
    assert_ne!(checksum, 0, "the fixture bytes were read");
    assert_eq!(memory.copies(), objects as u64, "one copy per transaction");
    report("baseline: bare copies, no substrate", objects as u64, bytes, baseline);

    let recipe = flat_recipe(objects, false);
    let started = Instant::now();
    let build = observe(&[recipe], &memory, ObservationConfig::new(HourStamp(1)))
        .expect("custody budget");
    let opening = started.elapsed();
    assert_eq!(
        build.report_for(RecipeId(1)).unwrap().outcome,
        RecipeOutcome::Captured
    );
    assert_eq!(build.snapshot.interval_count(), objects);
    assert_eq!(
        build.snapshot.custody_bytes(),
        bytes,
        "every transaction's bytes are in custody, so none of them was elided"
    );
    report("substrate: opening image only", objects as u64, bytes, opening);

    let recipe = flat_recipe(objects, true);
    let started = Instant::now();
    let build = observe(&[recipe], &memory, ObservationConfig::new(HourStamp(1)))
        .expect("custody budget");
    let with_witness = started.elapsed();
    assert_eq!(build.witnessed_intervals as usize, objects);
    report(
        "substrate: A/B, opening + closing",
        objects as u64 * 2,
        bytes * 2,
        with_witness,
    );

    let started = Instant::now();
    let closed = build.close_witnesses();
    let comparison = started.elapsed();
    assert!(closed.witness.changed.is_empty());
    report(
        "post-bracket: witness comparison",
        objects as u64,
        bytes * 2,
        comparison,
    );

    let overhead = opening.saturating_sub(baseline);
    println!(
        "  substrate overhead over bare copies: {:.3} ms total, {:.1} ns/txn",
        overhead.as_secs_f64() * 1000.0,
        overhead.as_nanos() as f64 / objects as f64,
    );
    println!(
        "  floor budget for 200k-360k copy transactions: 3-8 ms dev / 8-24 ms 4c (INCLUDES the"
    );
    println!("  real per-range scatter cost this fixture does not have)");
    println!();
}

#[test]
#[ignore = "instrument, not a gate"]
fn bench_coalescing_alone() {
    let requests: Vec<CopyRequest> = (0..100_000u64)
        .map(|index| CopyRequest {
            span: Span::new(BENCH_BASE + index * BENCH_STRIDE, 48),
            // Every request under one dereferenced pointer, so the policy has
            // to evaluate the generic tier at every boundary and reject it on
            // the gap rather than on the evidence.
            provenance: Provenance::from_pointer(BENCH_BASE),
            task: index as u32,
        })
        .collect();
    let policy = GapPolicy::default();
    let started = Instant::now();
    let planned = plan_copies(&requests, &policy);
    let elapsed = started.elapsed();
    assert_eq!(planned.len(), requests.len());
    println!();
    report(
        "policy: plan_copies, nothing merges",
        requests.len() as u64,
        requests.len() as u64 * 48,
        elapsed,
    );

    // The same count, all inside the generic tier: every boundary merges, so
    // this is the cost of the reduction rather than of the rejection.
    let dense: Vec<CopyRequest> = (0..100_000u64)
        .map(|index| CopyRequest {
            span: Span::new(BENCH_BASE + index * 40, 8),
            provenance: Provenance::from_pointer(BENCH_BASE),
            task: index as u32,
        })
        .collect();
    let started = Instant::now();
    let planned = plan_copies(&dense, &policy);
    let elapsed = started.elapsed();
    assert_eq!(planned.len(), 1, "32 B holes at the generic tier all merge");
    report(
        "policy: plan_copies, all merge",
        dense.len() as u64,
        dense.len() as u64 * 8,
        elapsed,
    );
    println!();
}

/// Where the executor's per-transaction cost actually goes.
///
/// Each row runs the SAME 100k operations the flat observation runs, in
/// isolation. The rows should roughly add up to the `substrate: opening image
/// only` figure; whatever dominates is what a future optimisation has to
/// address, and anything left over is dispatch and bookkeeping.
#[test]
#[ignore = "instrument, not a gate"]
fn bench_component_attribution() {
    use super::arena::Arena;
    use super::dedup::{AcquisitionPlan, OwnershipMap};
    use super::view::Snapshot;
    use super::{ConsumerId as C, DEFAULT_CUSTODY_BUDGET_BYTES};
    use std::collections::{BTreeMap, BTreeSet};

    let objects = 100_000usize;
    let spans: Vec<Span> = (0..objects)
        .map(|index| Span::new(BENCH_BASE + index as u64 * BENCH_STRIDE, 48))
        .collect();

    println!();
    println!("component attribution, {objects} operations each");

    // 1. Interval ownership as the executor does it: one cursor for the whole
    // batch, the acquisition plan swept against it, the fold at the wave end.
    let mut owners = OwnershipMap::new();
    let mut plan = AcquisitionPlan::default();
    let started = Instant::now();
    let mut cursor = owners.sweep();
    for (index, span) in spans.iter().enumerate() {
        owners.sweep_plan(&mut cursor, *span, &mut plan);
        owners.insert(*span, index * 48);
    }
    owners.end_wave();
    let ownership = started.elapsed();
    assert_eq!(owners.len(), objects);
    report(
        "  OwnershipMap: sweep plan + insert",
        objects as u64,
        0,
        ownership,
    );

    // 1b. The same work with a fresh cursor per query — the cost shape of a
    // per-transaction probe, kept so the sweep's saving is visible rather than
    // asserted.
    let mut bisecting = OwnershipMap::new();
    let started = Instant::now();
    for (index, span) in spans.iter().enumerate() {
        bisecting.plan_into(*span, &mut plan);
        bisecting.insert(*span, index * 48);
    }
    bisecting.end_wave();
    let ownership_bisecting = started.elapsed();
    report(
        "  OwnershipMap: plan + insert, bisecting",
        objects as u64,
        0,
        ownership_bisecting,
    );

    // 2. The dependency/edge attach pass, swept.
    let mut owned = Vec::new();
    let started = Instant::now();
    let mut cursor = owners.sweep();
    for span in &spans {
        owners.sweep_overlapping(&mut cursor, *span, &mut owned);
        for id in owned.iter() {
            owners.add_dependent(*id, C(1), *span);
        }
    }
    let attach = started.elapsed();
    report(
        "  OwnershipMap: sweep overlap + attach",
        objects as u64,
        0,
        attach,
    );

    // 2a. The sweep on its own, with the attach removed, so the two halves of
    // the row above are separable: the walk versus what the walk writes.
    let mut sweep_only = OwnershipMap::new();
    let mut cursor = sweep_only.sweep();
    for (index, span) in spans.iter().enumerate() {
        sweep_only.sweep_plan(&mut cursor, *span, &mut plan);
        sweep_only.insert(*span, index * 48);
    }
    sweep_only.end_wave();
    let started = Instant::now();
    let mut cursor = sweep_only.sweep();
    let mut hits = 0usize;
    for span in &spans {
        sweep_only.sweep_overlapping(&mut cursor, *span, &mut owned);
        hits += owned.len();
    }
    let sweep_alone = started.elapsed();
    assert_eq!(hits, objects);
    report("  OwnershipMap: sweep overlap only", objects as u64, 0, sweep_alone);

    // 2b. The same attach with a fresh cursor per request.
    let started = Instant::now();
    for span in &spans {
        bisecting.overlapping_into(*span, &mut owned);
        for id in owned.iter() {
            bisecting.add_dependent(*id, C(2), *span);
        }
    }
    let attach_bisecting = started.elapsed();
    report(
        "  OwnershipMap: overlap + attach, bisecting",
        objects as u64,
        0,
        attach_bisecting,
    );

    // 3. Point lookup. No longer on the per-task path — the decode phase's
    // parent now comes back with the field read — but still one per decode
    // site, so it is still priced.
    let started = Instant::now();
    let mut found = 0usize;
    for span in &spans {
        found += usize::from(owners.find(span.start).is_some());
    }
    let lookup = started.elapsed();
    assert_eq!(found, objects);
    report("  OwnershipMap: find", objects as u64, 0, lookup);

    // 4. Arena allocation.
    let mut arena = Arena::with_budget(DEFAULT_CUSTODY_BUDGET_BYTES);
    let started = Instant::now();
    for _ in 0..objects {
        arena.alloc_zeroed(48).expect("budget");
    }
    let allocation = started.elapsed();
    report("  Arena: alloc_zeroed", objects as u64, 0, allocation);

    // 5. The discovery visited set, hashed and ordered.
    let mut visited: super::executor::VisitedSet = Default::default();
    let started = Instant::now();
    for span in &spans {
        visited.insert((0, 0, span.start, 0));
    }
    let visit = started.elapsed();
    assert_eq!(visited.len(), objects);
    report("  visited set: insert, hashed", objects as u64, 0, visit);

    let mut ordered: BTreeSet<(u32, u16, u64, u64)> = BTreeSet::new();
    let started = Instant::now();
    for span in &spans {
        ordered.insert((0, 0, span.start, 0));
    }
    let visit_ordered = started.elapsed();
    report(
        "  visited set: insert, ordered",
        objects as u64,
        0,
        visit_ordered,
    );

    // THE TWO ROWS ABOVE FLATTER THE ORDERED SET, and the flattery is the
    // fixture's, not the structure's: this bench's addresses ascend, so every
    // ordered insert lands in the same rightmost leaf and never leaves L1,
    // while every hashed insert is a fresh random bucket. A discovered address
    // is a pointer the engine chose. These two rows are the same measurement
    // over addresses in no order, which is the shape the executor actually
    // sees.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let scattered: Vec<u64> = (0..objects)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state & 0x0000_7FFF_FFFF_FFC0
        })
        .collect();
    let mut hashed_scattered: super::executor::VisitedSet = Default::default();
    let started = Instant::now();
    for addr in &scattered {
        hashed_scattered.insert((0, 0, *addr, 0));
    }
    let visit_hashed_scattered = started.elapsed();
    report(
        "  visited set: scattered, hashed",
        objects as u64,
        0,
        visit_hashed_scattered,
    );

    let mut ordered_scattered: BTreeSet<(u32, u16, u64, u64)> = BTreeSet::new();
    let started = Instant::now();
    for addr in &scattered {
        ordered_scattered.insert((0, 0, *addr, 0));
    }
    let visit_ordered_scattered = started.elapsed();
    report(
        "  visited set: scattered, ordered",
        objects as u64,
        0,
        visit_ordered_scattered,
    );

    // 6. Freezing.
    let (records, links) = owners.into_frozen();
    let started = Instant::now();
    let snapshot = Snapshot::freeze(
        HourStamp(1),
        arena.into_bytes(),
        records,
        links,
        BTreeMap::new(),
    );
    let freeze = started.elapsed();
    assert_eq!(snapshot.interval_count(), objects);
    report("  Snapshot::freeze", objects as u64, 0, freeze);

    // NOT A DECOMPOSITION of the observation, and labelled so. `find` is in
    // it but off the flat recipe's path entirely (its only decode op is
    // `Freeze`, which reads no field); `into_frozen`'s ordering pass runs
    // before the freeze timer starts; and the coalescer, the synthetic
    // copies, the task queue and the decode dispatch are not here at all.
    // What the row is good for is comparing one component against itself
    // across a change, which is the only thing it is read for.
    let total = ownership + attach + lookup + allocation + visit + freeze;
    report("  SUM of the rows above (not a model)", objects as u64, 0, total);
    println!("  the rows are independent microbenchmarks: the sum is neither an upper");
    println!("  nor a lower bound on `substrate: opening image only`");
    println!();
}

/// The same instrument on a shape the sweep does NOT flatter.
///
/// `flat_recipe` is one recipe, one wave, a hundred thousand roots already in
/// address order: the batch sweep's best case, and its single `end_wave` folds
/// into an empty index body. A real observation is not that. This one spreads
/// the same transaction count over several recipes and several discovery
/// waves, so every wave's batches fold into an index that is already large,
/// and the addresses a wave discovers are interleaved with the ones earlier
/// waves already own.
///
/// Read the two together. The gap between them is what the shape costs, and
/// it is the honest correction to apply to
/// `bench_executor_overhead_at_one_hundred_thousand_ranges`.
#[test]
#[ignore = "instrument, not a gate"]
fn bench_observation_over_many_waves_and_recipes() {
    // A chain of `depth` links per root: CopyHeader -> DecodePointer -> the
    // same CopyHeader, so one root produces `depth + 1` transactions spread
    // over `depth + 1` waves.
    const NODE: u64 = 64;
    /// Slot pitch. Wider than every merge tier, so touching objects cannot
    /// coalesce and the transaction count stays the node count — the same
    /// property `BENCH_STRIDE` gives the flat recipe.
    const SLOT: u64 = 256;
    const LINK_BASE: u64 = 0x4000_0000;

    for (recipes, roots, depth) in [(1usize, 20_000usize, 4u32), (8, 2_500, 4), (8, 1_000, 11)] {
        let per_recipe_nodes = roots * (depth as usize + 1);
        let total = recipes * per_recipe_nodes;

        // Every node of every recipe gets its own slot, and a recipe's chain
        // hops by a stride that interleaves it with the other recipes' nodes:
        // no wave owns a contiguous region.
        let mut memory = SyntheticMemory::new();
        let mut region = vec![0u8; total * SLOT as usize];
        let slot = |recipe: usize, root: usize, link: usize| -> usize {
            (link * recipes * roots) + (root * recipes) + recipe
        };
        for recipe in 0..recipes {
            for root in 0..roots {
                for link in 0..depth as usize {
                    let here = slot(recipe, root, link);
                    let next = slot(recipe, root, link + 1);
                    let addr = LINK_BASE + next as u64 * SLOT;
                    let at = here * SLOT as usize;
                    region[at..at + 8].copy_from_slice(&addr.to_le_bytes());
                }
            }
        }
        memory.map(LINK_BASE, region);

        let programs: Vec<Recipe> = (0..recipes)
            .map(|recipe| {
                Recipe::new(
                    RecipeId(recipe as u32 + 1),
                    ConsumerId(recipe as u16 + 1),
                    vec![
                        RecipeOp::CopyHeader {
                            size: NODE as u32,
                            next: vec![1, 2],
                        },
                        RecipeOp::DecodePointer { offset: 0, next: 0 },
                        RecipeOp::Freeze,
                    ],
                    (0..roots)
                        .map(|root| Root {
                            op: 0,
                            addr: LINK_BASE + slot(recipe, root, 0) as u64 * SLOT,
                        })
                        .collect(),
                )
            })
            .collect();

        let started = Instant::now();
        let build = observe(&programs, &memory, ObservationConfig::new(HourStamp(1)))
            .expect("custody budget");
        let elapsed = started.elapsed();
        for recipe in 0..recipes {
            assert_eq!(
                build.report_for(RecipeId(recipe as u32 + 1)).unwrap().outcome,
                RecipeOutcome::Captured
            );
        }
        assert_eq!(build.snapshot.interval_count(), total);
        assert_eq!(build.forward_waves_run, depth + 1);
        report(
            &format!("observation: {recipes} recipes x {} waves", depth + 1),
            total as u64,
            total as u64 * NODE,
            elapsed,
        );
    }
    println!();
}

/// What the sweep costs on the other side of the ledger.
///
/// The index is address-sorted arrays, so a batch's new intervals are folded
/// in by a linear merge rather than by an insert per interval. That merge runs
/// once per batch (one recipe's copies in one wave) and once per wave, so its
/// cost scales with the WAVE count and not with the transaction count. This
/// row is a whole observation's worth of folds at the shape the floor
/// describes: fourteen phases over an index that ends at 100k intervals.
#[test]
#[ignore = "instrument, not a gate"]
fn bench_index_fold_cost() {
    use super::dedup::{AcquisitionPlan, OwnershipMap};

    let objects = 100_000usize;
    for waves in [1usize, 14, 56] {
        let per_wave = objects / waves;
        let mut owners = OwnershipMap::new();
        let mut plan = AcquisitionPlan::default();
        let started = Instant::now();
        for wave in 0..waves {
            let mut cursor = owners.sweep();
            // Interleaved: each wave's intervals land BETWEEN the previous
            // waves', which is the case a fold cannot shortcut.
            for index in 0..per_wave {
                let addr = BENCH_BASE + (index * waves + wave) as u64 * BENCH_STRIDE;
                let span = Span::new(addr, 48);
                owners.sweep_plan(&mut cursor, span, &mut plan);
                assert_eq!(plan.holes.len(), 1);
                owners.insert(span, 0);
            }
            owners.end_wave();
        }
        let elapsed = started.elapsed();
        assert_eq!(owners.len(), per_wave * waves);
        report(
            &format!("index: {waves} folds, plan + insert"),
            (per_wave * waves) as u64,
            0,
            elapsed,
        );
    }
    println!();
}

#[test]
#[ignore = "instrument, not a gate"]
fn bench_view_translation() {
    let objects = 100_000usize;
    let memory = bench_memory(objects);
    let closed = observe(
        &[flat_recipe(objects, false)],
        &memory,
        ObservationConfig::new(HourStamp(1)),
    )
    .expect("custody budget")
    .close_witnesses();
    let view = closed
        .snapshot
        .view_for(ConsumerId(1), HourStamp(1))
        .expect("view");

    let started = Instant::now();
    let mut checksum = 0u64;
    for index in 0..objects {
        checksum ^= view
            .u64_le(BENCH_BASE + index as u64 * BENCH_STRIDE)
            .expect("captured");
    }
    let elapsed = started.elapsed();
    assert_ne!(checksum, u64::MAX);
    println!();
    report(
        "view: translate + read u64",
        objects as u64,
        objects as u64 * 8,
        elapsed,
    );
    println!(
        "  page index buckets: {}",
        closed.snapshot.index().buckets()
    );
    println!();
}
