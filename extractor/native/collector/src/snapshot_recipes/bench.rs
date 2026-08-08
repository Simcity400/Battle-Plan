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

/// [`super::FxHasher`] with its finalising rotate removed, so the finaliser
/// can be PRICED rather than asserted.
///
/// This is the hasher the substrate had before: identical mixer, `finish`
/// returning the accumulator raw. It exists only in this bench.
#[derive(Clone, Copy, Debug, Default)]
struct UnfinalisedHasher {
    inner: super::FxHasher,
}

impl std::hash::Hasher for UnfinalisedHasher {
    fn write(&mut self, bytes: &[u8]) {
        self.inner.write(bytes)
    }
    fn write_u8(&mut self, value: u8) {
        self.inner.write_u8(value)
    }
    fn write_u16(&mut self, value: u16) {
        self.inner.write_u16(value)
    }
    fn write_u32(&mut self, value: u32) {
        self.inner.write_u32(value)
    }
    fn write_u64(&mut self, value: u64) {
        self.inner.write_u64(value)
    }
    fn write_usize(&mut self, value: usize) {
        self.inner.write_usize(value)
    }
    fn finish(&self) -> u64 {
        // The whole difference: `FxHasher::finish` rotates, this does not.
        self.inner.finish().rotate_right(20)
    }
}

type UnfinalisedSet<T> =
    std::collections::HashSet<T, std::hash::BuildHasherDefault<UnfinalisedHasher>>;

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
        owners.insert(*span, index * 48).unwrap();
    }
    owners.end_wave().unwrap();
    let ownership = started.elapsed();
    assert_eq!(owners.len(), objects);
    report(
        "  OwnershipMap: sweep plan + insert",
        objects as u64,
        0,
        ownership,
    );

    // 1a. The same sweep and the same inserts into a map whose record table
    // and index run were sized first.
    //
    // 1b. The same work with a fresh cursor per query — the cost shape of a
    // per-transaction probe, kept so the sweep's saving is visible rather than
    // asserted.
    let mut bisecting = OwnershipMap::new();
    let started = Instant::now();
    for (index, span) in spans.iter().enumerate() {
        bisecting.plan_into(*span, &mut plan);
        bisecting.insert(*span, index * 48).unwrap();
    }
    bisecting.end_wave().unwrap();
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
            owners.add_dependent(*id, C(1), *span).unwrap();
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
        sweep_only.insert(*span, index * 48).unwrap();
    }
    sweep_only.end_wave().unwrap();
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
            bisecting.add_dependent(*id, C(2), *span).unwrap();
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

    // 5. The discovery visited set.
    //
    // FOUR SHAPES, PRICED AGAINST EACH OTHER IN ONE RUN. The set was the
    // largest single component of the flat observation, and three things could
    // have been responsible: the container, the key width, or the hash. Each
    // row below changes exactly one of them against the row above it, and they
    // run back to back inside one invocation, so a busy box moves all four
    // together and the DIFFERENCES survive the load that the absolute numbers
    // do not.
    //
    //  * `insert, hashed` — what the executor now uses: two tables split by
    //    task kind, so a copy-op key is `(address, tag)` and hashes in two
    //    rounds instead of four.
    //  * `insert, wide key` — the same container and the same hash on the
    //    single 24-byte `(recipe, op, address, extent length)` key it
    //    replaced. The gap between the two rows is what the split bought.
    //  * `insert, unfinalised hash` — the wide key again, on `FxHash` WITHOUT
    //    the finalising rotate. `FxHash` propagates entropy upward only and
    //    `hashbrown` indexes buckets with the low bits, so this row is what a
    //    key set with constant low bits — every object-aligned engine address
    //    — costs when nothing folds the high half down.
    //  * `insert, ordered` — a `BTreeSet`, the structure the substrate started
    //    with.
    let mut visited = super::executor::VisitedSet::default();
    let started = Instant::now();
    for span in &spans {
        visited.insert_op(span.start, 0);
    }
    let visit = started.elapsed();
    assert_eq!(visited.len(), objects);
    report("  visited set: insert, hashed", objects as u64, 0, visit);

    let mut wide: super::FxSet<(u32, u16, u64, u64)> = Default::default();
    let started = Instant::now();
    for span in &spans {
        wide.insert((0, 0, span.start, 0));
    }
    let visit_wide = started.elapsed();
    assert_eq!(wide.len(), objects);
    report("  visited set: insert, wide key", objects as u64, 0, visit_wide);

    let mut raw: UnfinalisedSet<(u32, u16, u64, u64)> = Default::default();
    let started = Instant::now();
    for span in &spans {
        raw.insert((0, 0, span.start, 0));
    }
    let visit_raw = started.elapsed();
    assert_eq!(raw.len(), objects);
    report(
        "  visited set: insert, unfinalised hash",
        objects as u64,
        0,
        visit_raw,
    );

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
    let mut hashed_scattered = super::executor::VisitedSet::default();
    let started = Instant::now();
    for addr in &scattered {
        hashed_scattered.insert_op(*addr, 0);
    }
    let visit_hashed_scattered = started.elapsed();
    report(
        "  visited set: scattered, hashed",
        objects as u64,
        0,
        visit_hashed_scattered,
    );

    let mut raw_scattered: UnfinalisedSet<(u32, u16, u64, u64)> = Default::default();
    let started = Instant::now();
    for addr in &scattered {
        raw_scattered.insert((0, 0, *addr, 0));
    }
    let visit_raw_scattered = started.elapsed();
    report(
        "  visited set: scattered, unfinalised hash",
        objects as u64,
        0,
        visit_raw_scattered,
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
    let ledger = arena.ledger();
    let (records, links) = owners.into_frozen().unwrap();
    let started = Instant::now();
    let snapshot = Snapshot::freeze(
        HourStamp(1),
        arena.into_bytes(),
        records,
        links,
        BTreeMap::new(),
        &ledger,
    )
    .expect("custody budget");
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

    // LAST, AND DELIBERATELY LAST. Every row above this point predates lane FK
    // and is what an A/B between two binaries reads as a canary: if they move
    // together across a change that did not touch them, the box moved and not
    // the code. A row inserted among them perturbs the ones after it — its
    // allocations change the heap the next row starts from — and the first
    // draft of this row sat at position 1a and moved `sweep overlap + attach`
    // by 32% on an otherwise quiet A/B. So it goes at the end, after every row
    // anything is compared against.
    //
    // WHY IT IS HERE AT ALL. Lane FK found that most of what the executor's
    // dispatch block cost was two of its own buffers doubling up from empty
    // inside the bracket, and removed it by reserving them. The ownership index
    // grows the same way — `charged_push` per record, `charged_reserve` per
    // index entry — and after that cut `sweep plan + insert` is the largest
    // single component left. This row says how much of it is the same
    // pathology, so whoever takes it next starts from a number and not from an
    // analogy.
    //
    // NOT A CUT, and `reserve_for` is test-only: the interval count is not
    // known before the copies are planned, so a production reservation has to
    // be sized from the planned range count per batch, which is a change to the
    // ownership index's contract and not to this bench.
    let mut sized = OwnershipMap::new();
    sized.reserve_for(objects).unwrap();
    let started = Instant::now();
    let mut cursor = sized.sweep();
    for (index, span) in spans.iter().enumerate() {
        sized.sweep_plan(&mut cursor, *span, &mut plan);
        sized.insert(*span, index * 48).unwrap();
    }
    sized.end_wave().unwrap();
    let ownership_sized = started.elapsed();
    assert_eq!(sized.len(), objects);
    report(
        "  OwnershipMap: sweep plan + insert, sized",
        objects as u64,
        0,
        ownership_sized,
    );
    println!("  compare against `OwnershipMap: sweep plan + insert` at the top: the gap is what");
    println!("  the record table and index run cost by growing element by element");
    println!();
}

/// The executor's DISPATCH block, priced one phase at a time.
///
/// # Why this test exists
///
/// [`bench_component_attribution`] names six components and they sum to a
/// little over half of `substrate: opening image only`. Adding the two on-path
/// rows it does not contain — `policy: plan_copies, nothing merges` and
/// `baseline: bare copies, no substrate` — still leaves roughly a hundred
/// nanoseconds per transaction with no row against its name. That residual is
/// the dispatch block: materialising a [`CopyTask`], pushing it into a wave
/// queue and taking it back out, resolving it into [`CopyRequest`]s, and the
/// decode-side match that turns a copied header into the next wave's tasks.
///
/// A residual is not a measurement. These rows run the executor's own methods,
/// one phase per timer, through [`super::executor::DispatchProbe`].
///
/// # How to read it
///
/// The rows run back to back inside ONE invocation against ONE prepared
/// observation, in the order the executor runs them, so a busy box moves the
/// whole block together and the shares survive load the absolute numbers do
/// not. Quote the per-row minimum over at least five invocations with
/// `--test-threads=1`, the same rule as every other row in this file.
///
/// `construct + queue push` and `enqueue` are the SAME 100k task literals: the
/// first pushes them into a bare `Vec<CopyTask>` and the second runs the real
/// `enqueue`, so the gap between them is the depth check, the visited insert
/// and the copy-task limit — and the visited insert already has its own row
/// above, which is what makes that gap attributable rather than lumped.
///
/// WHAT THESE ROWS DO NOT SEPARATE, stated rather than implied.
///
/// `queue push` is inside `construct + queue push` and not on its own, because
/// a `CopyTask` that is constructed and not pushed is a construction the
/// optimiser is free to delete. The two are priced as the pair they are, and
/// the pair's width is printed alongside as `size_of::<CopyTask>()`.
///
/// The two `at capacity` rows re-run a loop into the buffer the row above them
/// just filled, so they have that buffer's PAGES warm as well as its capacity.
/// They are therefore an upper bound on what a reservation can recover, not a
/// measurement of it: a reservation maps fresh pages that are still cold on
/// first touch. The rows that price the reservations as production takes them
/// — `enqueue, root queue reserved` and `requests_of, reserved` — time the
/// reservation itself along with the work, and the honest whole-path figure is
/// `substrate: opening image only`, which contains everything either way.
///
/// None of these rows decomposes the observation. `find` is off the flat
/// recipe's path, the copy phase and the freeze are not here, and the sums at
/// the bottom are sums of the rows above them and nothing more.
#[test]
#[ignore = "instrument, not a gate"]
fn bench_dispatch_components() {
    use super::executor::DispatchProbe;

    let objects = 100_000usize;
    let memory = bench_memory(objects);
    let recipes = vec![flat_recipe(objects, false)];

    println!();
    println!("dispatch components, {objects} tasks each");
    let widths = DispatchProbe::<SyntheticMemory>::value_sizes();
    let sizes = widths
        .iter()
        .map(|(name, bytes)| format!("{name} {bytes} B"))
        .collect::<Vec<_>>()
        .join(", ");
    println!("  value widths: {sizes}");

    // 1. Construction and the queue push, with nothing else. The sink is a
    // `Vec<CopyTask>` — the same container a wave queue is — and it is read
    // afterwards, so neither the construction nor the push is dead.
    let mut sink: Vec<super::executor::CopyTask> = Vec::new();
    let started = Instant::now();
    DispatchProbe::<SyntheticMemory>::construct_only(&recipes, &mut sink);
    let construct = started.elapsed();
    assert_eq!(sink.len(), objects, "every root became a task");
    // CONSUMED, not merely counted. A length is something the optimiser can
    // keep while dropping the stores that filled the elements, and a row that
    // measured a loop with its stores removed would make every attribution
    // below it wrong.
    let mut checksum = DispatchProbe::<SyntheticMemory>::checksum(&sink);
    assert_ne!(checksum, 0, "the constructed tasks were read back");
    report("  dispatch: construct + queue push", objects as u64, 0, construct);

    // 1b. The SAME loop into the SAME vector, which is now at capacity and
    // whose pages are mapped. `construct_only` clears rather than reallocates,
    // so the second pass constructs and pushes exactly what the first did with
    // the doubling reallocations and the first-touch page faults taken out.
    //
    // This row exists to answer one question before anything is changed: is
    // the construct-and-push cost the STORE, or is it the queue growing from
    // nothing? A wave queue starts empty on every observation, so if the gap
    // between these two rows is most of the first one, the cut is a reservation
    // and not a narrower task.
    let started = Instant::now();
    DispatchProbe::<SyntheticMemory>::construct_only(&recipes, &mut sink);
    let construct_warm = started.elapsed();
    assert_eq!(sink.len(), objects);
    checksum ^= DispatchProbe::<SyntheticMemory>::checksum(&sink);
    report(
        "  dispatch: construct + push, at capacity",
        objects as u64,
        0,
        construct_warm,
    );
    assert_eq!(checksum, 0, "the two passes constructed identical tasks");
    drop(sink);

    // 2. The real `enqueue` over the same roots: construction and push again,
    // plus the depth check, the visited insert and the copy-task limit.
    let mut probe = DispatchProbe::new(&recipes, &memory, ObservationConfig::new(HourStamp(1)))
        .expect("custody budget");
    let started = Instant::now();
    let queued = probe.seed_roots();
    let enqueue = started.elapsed();
    assert_eq!(queued, objects, "every root was queued, none pruned");
    report(
        "  dispatch: enqueue, queue from empty",
        objects as u64,
        0,
        enqueue,
    );
    probe.charge_tables().expect("custody budget");
    // The tasks this row queued, read back before the observation is dropped.
    // Observing only the queue's LENGTH leaves the element stores available for
    // the optimiser to remove.
    let seeded = probe.take_wave(0);
    checksum ^= DispatchProbe::<SyntheticMemory>::checksum(&seeded);
    assert_ne!(checksum, 0, "the enqueued tasks were read back");
    drop(seeded);
    drop(probe);

    // 2b. The same `enqueue` over the same roots on a fresh observation that
    // called the production `reserve_root_queue` first. A whole separate
    // observation, so the visited set is empty again and every root is fresh:
    // the ONLY difference from the row above is the reservation, and it is the
    // one `run` takes, not a reservation this bench invented.
    let mut probe = DispatchProbe::new(&recipes, &memory, ObservationConfig::new(HourStamp(1)))
        .expect("custody budget");
    // The reservation is INSIDE the timer. It is not free — it charges the
    // ledger and it takes the pages — and a row that excluded it would be
    // pricing a production path that does not exist.
    let started = Instant::now();
    probe.reserve_root_queue().expect("custody budget");
    let queued = probe.seed_roots();
    let enqueue_reserved = started.elapsed();
    assert_eq!(queued, objects);
    report(
        "  dispatch: enqueue, root queue reserved",
        objects as u64,
        0,
        enqueue_reserved,
    );
    probe.charge_tables().expect("custody budget");

    // 3. Taking the wave back out of the queue.
    let started = Instant::now();
    let tasks = probe.take_wave(0);
    let take = started.elapsed();
    assert_eq!(tasks.len(), objects);
    report("  dispatch: take wave", objects as u64, 0, take);

    // The reserved row's tasks, read back — and required to be the SAME tasks
    // the unreserved row built. Length alone left the reserved row's element
    // stores available for removal even though the unreserved row's were
    // consumed, and the two rows are only comparable if they built the same
    // thing. The flat recipe's path never reads `inherited`, so without this
    // the twenty-four bytes it stores could be specialised away from one row
    // and not the other.
    let reserved_tasks = DispatchProbe::<SyntheticMemory>::checksum(&tasks);
    assert_eq!(
        reserved_tasks,
        checksum,
        "the reserved row queued the same tasks the unreserved row did",
    );

    // 4. `copy_wave`'s grouping pass: one push per task into its recipe's
    // group.
    let started = Instant::now();
    let grouped = probe.group_pass(&tasks);
    let group = started.elapsed();
    assert_eq!(grouped, objects, "every task landed in a group");
    // The indices, not just how many there are: a count survives an optimiser
    // that never wrote them.
    checksum ^= probe.groups_checksum();
    report("  dispatch: group by recipe", objects as u64, 0, group);

    // 5. `requests_of`: the op lookup, the provenance construction and the
    // `CopyRequest` push, per task.
    // The buffer starts empty on the observation's first batch, so this row is
    // the resolution plus the buffer doubling up to a hundred thousand
    // requests — the shape before `copy_wave` reserved.
    let started = Instant::now();
    let produced = probe.requests_pass(&tasks, false);
    let resolve = started.elapsed();
    assert_eq!(produced, objects, "one request per flat header");
    checksum ^= probe.requests_checksum();
    assert_ne!(checksum, 0, "the resolved requests were read back");
    report(
        "  dispatch: requests_of, buffer from empty",
        objects as u64,
        0,
        resolve,
    );

    // The same pass with `copy_wave`'s reservation in front of it, on a FRESH
    // observation whose request buffer has never been touched.
    //
    // The earlier draft re-ran into the buffer the row above had just filled,
    // so the reservation was a no-op against warm pages — an "at capacity"
    // row wearing the reserved row's label, and the sum at the bottom then
    // claimed it as the production shape. Production's first batch reserves an
    // EMPTY buffer and still first-touches every page of it, which is what this
    // measures.
    let mut fresh = DispatchProbe::new(&recipes, &memory, ObservationConfig::new(HourStamp(1)))
        .expect("custody budget");
    fresh.reserve_root_queue().expect("custody budget");
    fresh.seed_roots();
    fresh.charge_tables().expect("custody budget");
    let fresh_tasks = fresh.take_wave(0);
    let started = Instant::now();
    let produced = fresh.requests_pass(&fresh_tasks, true);
    let resolve_reserved = started.elapsed();
    assert_eq!(produced, objects);
    let fresh_checksum = fresh.requests_checksum();
    assert_eq!(
        fresh_checksum,
        probe.requests_checksum(),
        "the reserved pass resolved the same requests as the unreserved one, so the two \
         rows price the same work",
    );
    report(
        "  dispatch: requests_of, reserved",
        objects as u64,
        0,
        resolve_reserved,
    );
    drop(fresh_tasks);
    drop(fresh);

    // 6. The whole copy phase, for context and so the decode row below runs
    // against bytes that have actually landed. NOT a dispatch row: it contains
    // the coalescer, the acquisition sweep, the arena and the real copies.
    let started = Instant::now();
    probe.copy_pass(&tasks).expect("custody budget");
    let copy = started.elapsed();
    report("  (copy_wave, for context)", objects as u64, 0, copy);

    // 7. The decode dispatch: per task, match the body, read the op, push a
    // `DecodeSite` per branch, then `run_sites` drains the stack through
    // `run_site`'s op match. The flat recipe's only decode op is `Freeze`, so
    // this row is the DISPATCH with no field reads and no discoveries under
    // it — which is exactly what it is meant to isolate.
    let started = Instant::now();
    probe.decode_pass(&tasks);
    let decode = started.elapsed();
    assert_eq!(
        probe.frozen_branches() as usize,
        objects,
        "every task's branch reached Freeze, so the dispatch really ran"
    );
    assert_eq!(probe.queued_at(1), 0, "a flat recipe discovers nothing");
    report("  dispatch: decode_wave (Freeze only)", objects as u64, 0, decode);

    // TWO sums, because there are two shapes above and only one of them is
    // what the executor runs. The first is what the dispatch block cost when
    // both buffers grew from empty; the second is the same block with the two
    // reservations `run` and `copy_wave` now take, priced with the reservations
    // inside the timers.
    //
    // Neither is a decomposition of `substrate: opening image only` — the copy
    // phase, the coalescer, the ownership sweep and the freeze are not in
    // either. And the difference between them is an INDICATION of what the
    // reservations bought, not a measurement of it: `requests_of, reserved`
    // inherits warm pages from the row before it. The measurement is the
    // whole-observation row, taken against the previous binary, interleaved.
    let grown = enqueue + take + group + resolve + decode;
    let reserved = enqueue_reserved + take + group + resolve_reserved + decode;
    report("  dispatch: SUM, buffers from empty", objects as u64, 0, grown);
    report("  dispatch: SUM, buffers reserved", objects as u64, 0, reserved);
    println!("  `visited set: insert, hashed` above is inside both enqueue rows.");
    println!();
}

/// The decode dispatch on a shape that DISCOVERS.
///
/// [`bench_dispatch_components`]'s decode row runs a recipe whose only decode
/// op is `Freeze`: it prices the match and the stack, and deliberately nothing
/// under them. A real decode reads a pointer field out of copied bytes and
/// enqueues the next wave, so this row is the same dispatch with `read_uint`
/// and the enqueue of a discovered task in it. The two together separate the
/// dispatch from what the dispatch does.
#[test]
#[ignore = "instrument, not a gate"]
fn bench_decode_dispatch_with_discovery() {
    use super::executor::DispatchProbe;

    const NODE: u64 = 64;
    const SLOT: u64 = 256;
    const LINK_BASE: u64 = 0x4000_0000;
    let roots = 100_000usize;

    // Two slots per root: the header the root copies, and the node its pointer
    // field names. Depth one is enough — the dispatch is per task, not per
    // depth.
    let mut memory = SyntheticMemory::new();
    let mut region = vec![0u8; roots * 2 * SLOT as usize];
    for root in 0..roots {
        let target = LINK_BASE + (roots + root) as u64 * SLOT;
        let at = root * SLOT as usize;
        region[at..at + 8].copy_from_slice(&target.to_le_bytes());
    }
    memory.map(LINK_BASE, region);

    let recipes = vec![Recipe::new(
        RecipeId(1),
        ConsumerId(1),
        vec![
            RecipeOp::CopyHeader {
                size: NODE as u32,
                next: vec![1],
            },
            RecipeOp::DecodePointer { offset: 0, next: 0 },
            RecipeOp::Freeze,
        ],
        (0..roots)
            .map(|root| Root {
                op: 0,
                addr: LINK_BASE + root as u64 * SLOT,
            })
            .collect(),
    )];

    let mut probe = DispatchProbe::new(&recipes, &memory, ObservationConfig::new(HourStamp(1)))
        .expect("custody budget");
    assert_eq!(probe.seed_roots(), roots);
    probe.charge_tables().expect("custody budget");
    let tasks = probe.take_wave(0);
    probe.copy_pass(&tasks).expect("custody budget");

    println!();
    let started = Instant::now();
    probe.decode_pass(&tasks);
    let decode = started.elapsed();
    assert_eq!(
        probe.queued_at(1),
        roots,
        "every pointer was read and enqueued, so the row priced discovery"
    );
    report(
        "  dispatch: decode_wave + read + enqueue",
        roots as u64,
        0,
        decode,
    );
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
                owners.insert(span, 0).unwrap();
            }
            owners.end_wave().unwrap();
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
