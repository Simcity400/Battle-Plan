//! expansion, per-wave coalescing, global interval ownership and dedup
//! [`super::synthetic::SyntheticMemory`], which is a bounds check plus a
//!    region, so ownership lookups hit a much friendlier BTree than a live
//!  * the ownership index is address-sorted arrays swept by a [`Cursor`], and
//!    (`OwnershipMap: sweep plan + insert`, `sweep overlap + attach` below —
//! `cargo test -p collector --release --lib -- --ignored --nocapture snapshot_recipes::bench`.

use std::time::Instant;

use super::executor::{observe, ObservationConfig, Recipe, RecipeOp, RecipeOutcome, Root};
use super::policy::{plan_copies, CopyRequest, GapPolicy, Provenance};
use super::synthetic::SyntheticMemory;
use super::{ConsumerId, HourStamp, RecipeId, SnapshotMemory, Span};

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
        self.inner.finish().rotate_right(20)
    }
}

type UnfinalisedSet<T> =
    std::collections::HashSet<T, std::hash::BuildHasherDefault<UnfinalisedHasher>>;

const BENCH_BASE: u64 = 0x2000_0000;
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

    let mut sink = vec![0u8; OBJECT_BYTES as usize];
    let mut checksum = 0u64;
    let started = Instant::now();
    for index in 0..objects {
        memory
            .copy_into(BENCH_BASE + index as u64 * BENCH_STRIDE, &mut sink)
            .expect("fixture read");
        checksum = checksum.wrapping_add(u64::from(sink[index % sink.len()]));
    }
    let baseline = started.elapsed();
    assert_ne!(checksum, 0, "the fixture bytes were read");
    assert_eq!(memory.copies(), objects as u64, "one copy per transaction");
    report(
        "baseline: bare copies, no substrate",
        objects as u64,
        bytes,
        baseline,
    );

    let recipe = flat_recipe(objects, false);
    let started = Instant::now();
    let build =
        observe(&[recipe], &memory, ObservationConfig::new(HourStamp(1))).expect("custody budget");
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
    report(
        "substrate: opening image only",
        objects as u64,
        bytes,
        opening,
    );

    let recipe = flat_recipe(objects, true);
    let started = Instant::now();
    let build =
        observe(&[recipe], &memory, ObservationConfig::new(HourStamp(1))).expect("custody budget");
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
    report(
        "  OwnershipMap: sweep overlap only",
        objects as u64,
        0,
        sweep_alone,
    );

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

    let started = Instant::now();
    let mut found = 0usize;
    for span in &spans {
        found += usize::from(owners.find(span.start).is_some());
    }
    let lookup = started.elapsed();
    assert_eq!(found, objects);
    report("  OwnershipMap: find", objects as u64, 0, lookup);

    let mut arena = Arena::with_budget(DEFAULT_CUSTODY_BUDGET_BYTES);
    let started = Instant::now();
    for _ in 0..objects {
        arena.alloc_zeroed(48).expect("budget");
    }
    let allocation = started.elapsed();
    report("  Arena: alloc_zeroed", objects as u64, 0, allocation);

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
    report(
        "  visited set: insert, wide key",
        objects as u64,
        0,
        visit_wide,
    );

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

    let ledger = arena.ledger();
    let (records, links) = owners.into_frozen().unwrap();
    let started = Instant::now();
    let snapshot = Snapshot::freeze(
        HourStamp(1),
        arena.into_bytes(),
        records,
        links,
        BTreeMap::new(),
        super::annot::Annotations::new(),
        Vec::new(),
        Vec::new(),
        &ledger,
    )
    .expect("custody budget");
    let freeze = started.elapsed();
    assert_eq!(snapshot.interval_count(), objects);
    report("  Snapshot::freeze", objects as u64, 0, freeze);

    let total = ownership + attach + lookup + allocation + visit + freeze;
    report(
        "  SUM of the rows above (not a model)",
        objects as u64,
        0,
        total,
    );
    println!("  the rows are independent microbenchmarks: the sum is neither an upper");
    println!("  nor a lower bound on `substrate: opening image only`");

    // inside the bracket, and removed it by reserving them. The ownership index
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

    let mut sink: Vec<super::executor::CopyTask> = Vec::new();
    let started = Instant::now();
    DispatchProbe::<SyntheticMemory>::construct_only(&recipes, &mut sink);
    let construct = started.elapsed();
    assert_eq!(sink.len(), objects, "every root became a task");
    let mut checksum = DispatchProbe::<SyntheticMemory>::checksum(&sink);
    assert_ne!(checksum, 0, "the constructed tasks were read back");
    report(
        "  dispatch: construct + queue push",
        objects as u64,
        0,
        construct,
    );

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
    let seeded = probe.take_wave(0);
    checksum ^= DispatchProbe::<SyntheticMemory>::checksum(&seeded);
    assert_ne!(checksum, 0, "the enqueued tasks were read back");
    drop(seeded);
    drop(probe);

    let mut probe = DispatchProbe::new(&recipes, &memory, ObservationConfig::new(HourStamp(1)))
        .expect("custody budget");
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

    let started = Instant::now();
    let tasks = probe.take_wave(0);
    let take = started.elapsed();
    assert_eq!(tasks.len(), objects);
    report("  dispatch: take wave", objects as u64, 0, take);

    let reserved_tasks = DispatchProbe::<SyntheticMemory>::checksum(&tasks);
    assert_eq!(
        reserved_tasks, checksum,
        "the reserved row queued the same tasks the unreserved row did",
    );

    let started = Instant::now();
    let grouped = probe.group_pass(&tasks);
    let group = started.elapsed();
    assert_eq!(grouped, objects, "every task landed in a group");
    checksum ^= probe.groups_checksum();
    report("  dispatch: group by recipe", objects as u64, 0, group);

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

    let started = Instant::now();
    probe.copy_pass(&tasks).expect("custody budget");
    let copy = started.elapsed();
    report("  (copy_wave, for context)", objects as u64, 0, copy);

    let started = Instant::now();
    probe.decode_pass(&tasks);
    let decode = started.elapsed();
    assert_eq!(
        probe.frozen_branches() as usize,
        objects,
        "every task's branch reached Freeze, so the dispatch really ran"
    );
    assert_eq!(probe.queued_at(1), 0, "a flat recipe discovers nothing");
    report(
        "  dispatch: decode_wave (Freeze only)",
        objects as u64,
        0,
        decode,
    );

    // phase, the coalescer, the ownership sweep and the freeze are not in
    let grown = enqueue + take + group + resolve + decode;
    let reserved = enqueue_reserved + take + group + resolve_reserved + decode;
    report(
        "  dispatch: SUM, buffers from empty",
        objects as u64,
        0,
        grown,
    );
    report(
        "  dispatch: SUM, buffers reserved",
        objects as u64,
        0,
        reserved,
    );
    println!("  `visited set: insert, hashed` above is inside both enqueue rows.");
    println!();
}

#[test]
#[ignore = "instrument, not a gate"]
fn bench_decode_dispatch_with_discovery() {
    use super::executor::DispatchProbe;

    const NODE: u64 = 64;
    const SLOT: u64 = 256;
    const LINK_BASE: u64 = 0x4000_0000;
    let roots = 100_000usize;

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
            RecipeOp::DecodePointer {
                offset: 0,
                guard: None,
                next: 0,
            },
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

#[test]
#[ignore = "instrument, not a gate"]
fn bench_observation_over_many_waves_and_recipes() {
    const NODE: u64 = 64;
    const SLOT: u64 = 256;
    const LINK_BASE: u64 = 0x4000_0000;

    for (recipes, roots, depth) in [(1usize, 20_000usize, 4u32), (8, 2_500, 4), (8, 1_000, 11)] {
        let per_recipe_nodes = roots * (depth as usize + 1);
        let total = recipes * per_recipe_nodes;

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
                        RecipeOp::DecodePointer {
                            offset: 0,
                            guard: None,
                            next: 0,
                        },
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
                build
                    .report_for(RecipeId(recipe as u32 + 1))
                    .unwrap()
                    .outcome,
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
