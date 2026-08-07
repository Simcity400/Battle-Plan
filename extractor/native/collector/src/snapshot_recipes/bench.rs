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
//! cost down. The dominant term is [`super::dedup::OwnershipMap`]: the
//! substrate performs FIVE ordered-map traversals per copy transaction — two
//! range queries to plan the acquisition, one insert, two more range queries to
//! attach dependents and edges — plus one more for the decode-phase parent
//! lookup and one `BTreeSet` insert for the discovery visited set. At 100k
//! owned intervals those tree walks, not the byte movement and not the
//! allocations, are the bulk of the number. Global deduplication is a hard
//! requirement (t35's per-bracket sum is nearly twice its union), so the
//! structure behind it is what a future optimisation has to change — a
//! per-wave sort-and-sweep against a sorted array rather than per-transaction
//! tree probes.
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
    let started = Instant::now();
    for index in 0..objects {
        memory
            .copy_into(BENCH_BASE + index as u64 * BENCH_STRIDE, &mut sink)
            .expect("fixture read");
    }
    let baseline = started.elapsed();
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

    // 1. Interval ownership: the dedup query plus the insert.
    let mut owners = OwnershipMap::new();
    let mut plan = AcquisitionPlan::default();
    let started = Instant::now();
    for (index, span) in spans.iter().enumerate() {
        owners.plan_into(*span, &mut plan);
        owners.insert(*span, index * 48);
    }
    let ownership = started.elapsed();
    report("  OwnershipMap: plan + insert", objects as u64, 0, ownership);

    // 2. The dependency/edge attach pass: one overlap query per request.
    let mut owned = Vec::new();
    let started = Instant::now();
    for span in &spans {
        owners.overlapping_into(*span, &mut owned);
        for id in owned.iter() {
            owners.add_dependent(*id, C(1), *span);
        }
    }
    let attach = started.elapsed();
    report("  OwnershipMap: overlap + attach", objects as u64, 0, attach);

    // 3. The decode-phase parent lookup.
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

    // 5. The discovery visited set.
    let mut visited: BTreeSet<(usize, u16, u64)> = BTreeSet::new();
    let started = Instant::now();
    for span in &spans {
        visited.insert((0, 0, span.start));
    }
    let visit = started.elapsed();
    report("  visited set: insert", objects as u64, 0, visit);

    // 6. Freezing.
    let records = owners.into_records();
    let started = Instant::now();
    let snapshot = Snapshot::freeze(HourStamp(1), arena.into_bytes(), records, BTreeMap::new());
    let freeze = started.elapsed();
    assert_eq!(snapshot.interval_count(), objects);
    report("  Snapshot::freeze", objects as u64, 0, freeze);

    let total = ownership + attach + lookup + allocation + visit + freeze;
    report("  SUM of components", objects as u64, 0, total);
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
