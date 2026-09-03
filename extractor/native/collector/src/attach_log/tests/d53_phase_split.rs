use super::*;

pub(super) fn item_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("{signature} must exist"));
    let end = source[start..]
        .find("\n}\n")
        .map(|at| start + at + 3)
        .unwrap_or_else(|| panic!("{signature} must close on a column-zero brace"));
    &source[start..end]
}

pub(super) fn code_only(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn d53_clock_bracket_calls_only_the_bounded_capture_function() {
    let source = super::collector_source_lf();
    let anchor = source
        .find("COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE,\n                move || {\n                    capture_compact_diplomacy_intel_operatives_live_snapshot(")
        .expect("d53 must put its snapshot capture in the bracket");
    let spawn = source[..anchor]
        .rfind("spawn_gated_finishing(")
        .expect("d53 must use the split finishing helper");
    let tail = &source[anchor..];
    let finish = tail
        .find("finish_compact_diplomacy_intel_operatives_live_snapshot,")
        .expect("d53 must name its snapshot decoder as the finish half");
    let next_spawn = tail[1..]
        .find("spawn_gated")
        .map(|at| at + 1)
        .unwrap_or(tail.len());
    assert!(spawn < anchor && finish < next_spawn);

    let capture = code_only(item_body(
        &source,
        "fn capture_compact_diplomacy_intel_operatives_live_snapshot<'t>(",
    ));
    assert!(capture.contains("plan.capture_with("));
    // The gate query and the batch copy are their own functions so the
    // benchmark measures the production path, and the query budget the
    // bracket runs under is the planner's OS-region cap.
    assert!(capture.contains("domain53_snapshot_os_region,"));
    assert!(capture.contains("domain53_batch_copy,"));
    assert!(capture.contains("DOMAIN53_SNAPSHOT_MAX_OS_REGIONS as u32,"));
    let gated = code_only(item_body(&source, "fn domain53_batch_copy("));
    assert!(gated.contains("seh_guarded_copy_specs("));
    // The gate's region predicate is the live reader's (`readable_region`).
    let region = code_only(item_body(&source, "fn domain53_snapshot_os_region("));
    assert!(region.contains("query_region(address as usize)?"));
    assert!(region.contains("region.state == MEM_COMMIT"));
    assert!(region.contains("(region.protect & (PAGE_NOACCESS | PAGE_GUARD)) == 0"));
    let capture = format!("{capture}
{gated}
{region}");
    for forbidden in [
        "Domain53InProcessMemory",
        "read_compact_diplomacy_intel_operatives_live_observe",
        "extract_rows_from_current_roots",
        "Vec::",
        "Box::",
        "Mutex",
        "Instant::",
        "read_into(",
        "read_bytes(",
    ] {
        assert!(
            !capture.contains(forbidden),
            "the bracket capture reached `{forbidden}`:\n{capture}"
        );
    }
}

#[test]
fn d53_allocation_and_walk_are_before_or_after_the_bracket() {
    let source = super::collector_source_lf();
    let arm = source
        .find("let snapshot_arm = arm_compact_diplomacy_intel_operatives_live_snapshot(")
        .expect("d53 must arm its preallocated plan");
    let spawn = source[arm..]
        .find("spawn_gated_finishing(")
        .map(|at| arm + at)
        .expect("d53 must spawn after arming");
    assert!(arm < spawn, "allocation and trace walk must precede the bracket");

    let finish = code_only(item_body(
        &source,
        "fn finish_compact_diplomacy_intel_operatives_live_snapshot(",
    ));
    assert!(finish.contains("let snapshot = plan.view();"));
    assert!(finish.contains("read_compact_diplomacy_intel_operatives_live_observe("));
    assert!(finish.contains("snapshot.first_miss().is_some()"));
    assert!(finish.contains("DOMAIN53_SNAPSHOT_REGION_MISS_CAUSE"));
    assert!(
        finish.contains("tracker.snapshot_plan = Some(plan);"),
        "a successful frame must reuse its preallocated plan instead of live-walking to arm every hour"
    );
    assert!(
        !finish.contains("Domain53InProcessMemory"),
        "the decoder must not regain a live engine-memory handle"
    );
}

/// Lane PF: the traced plan walk is the old live d53 read and costs what it
/// cost (P4: 66 ms of serial scheduler time per hour when it armed inline).
/// It runs only in the plan job's thread; the scheduler-thread arm may take a
/// plan, reap a job or spawn one, and reads nothing.
#[test]
fn d53_arm_reads_nothing_and_the_plan_walk_runs_on_its_own_thread() {
    let source = super::collector_source_lf();
    let arm = code_only(item_body(
        &source,
        "fn arm_compact_diplomacy_intel_operatives_live_snapshot(",
    ));
    for forbidden in [
        "Domain53TracingMemory",
        "from_trace(",
        "extract_rows_from_current_roots",
        "into_trace(",
    ] {
        assert!(
            !arm.contains(forbidden),
            "the scheduler-thread arm reached `{forbidden}`:\n{arm}"
        );
    }
    assert!(arm.contains("domain53_reap_plan_job(tracker, now);"));
    assert!(arm.contains("domain53_spawn_plan_job("));
    // The status-failure short circuit is the one observe call the arm keeps;
    // it returns before any read.
    assert_eq!(
        arm.matches("read_compact_diplomacy_intel_operatives_live_observe(").count(),
        1
    );
    assert!(arm.contains("if compact_contract::compact_status_is_failure(status_flags) {"));

    let job = code_only(item_body(&source, "fn domain53_plan_job_main("));
    assert!(job.contains("Domain53TracingMemory::new(&live)"));
    assert!(job.contains("read_compact_diplomacy_intel_operatives_live_observe("));
    assert!(job.contains("from_trace("));
    assert!(job.contains("apply_collector_worker_priority();"));
    // Codex round 1, F7: the walk's Ready stage must not close the production
    // warmup window; it observes through a private flag seeded from it.
    assert!(job.contains("AtomicBool::new(domain53_warmup_window_open())"));
    assert!(!job.contains("&DOMAIN53_WARMUP_WINDOW_OPEN"));
    // F8: a held failure is named on every frame until the retry spawns.
    assert!(arm.contains("match tracker.plan_job_failure {"));
    assert!(!arm.contains("plan_job_failure.take()"));

    let spawn = code_only(item_body(&source, "fn domain53_spawn_plan_job("));
    assert!(spawn.contains("thread::Builder::new()"));
    assert!(spawn.contains("domain53_plan_job_main("));
}

/// A refused page copy drops the plan so the next arm walks again; keeping it
/// would refuse the same unreadable page every hour for the rest of the run.
#[test]
fn d53_copy_refusal_drops_the_plan_and_the_miss_keeps_none() {
    let source = super::collector_source_lf();
    let finish = code_only(item_body(
        &source,
        "fn finish_compact_diplomacy_intel_operatives_live_snapshot(",
    ));
    let copy_refused = finish
        .find("if receipt.failure_region_ordinal.is_some() || receipt.gate_budget_exhausted {")
        .expect("the copy-refusal arm drops the plan on a refused data page and on an exhausted gate budget");
    let miss = finish
        .find("if snapshot.first_miss().is_some() {")
        .expect("the region-miss arm");
    let success = finish
        .rfind("tracker.snapshot_plan = Some(plan);")
        .expect("the success arm keeps the plan");
    assert!(copy_refused < miss && miss < success);
    assert!(
        finish[copy_refused..miss].contains("drop(plan);")
            && !finish[copy_refused..miss].contains("tracker.snapshot_plan = Some(plan);"),
        "a copy refusal must drop the plan"
    );
    assert!(
        !finish[miss..success].contains("tracker.snapshot_plan = Some(plan);"),
        "a region miss must not keep the plan"
    );
}

#[test]
fn d53_snapshot_receipt_is_emitted_in_the_existing_perf_channel() {
    let source = super::collector_source_lf();
    let units = code_only(item_body(&source, "fn domain53_diag_units("));
    for name in [
        "domain53_bulk_snapshot_regions",
        "domain53_bulk_snapshot_bytes",
        "domain53_bulk_snapshot_copied_regions",
        "domain53_bulk_snapshot_probe_regions",
        "domain53_bulk_snapshot_probe_unreadable",
        "domain53_bulk_snapshot_attempted_os_regions",
        "domain53_bulk_snapshot_gate_queries",
        "domain53_bulk_snapshot_gate_budget_exhausted",
        "domain53_bulk_snapshot_copied_bytes",
        "domain53_bulk_snapshot_copy_refused",
        "domain53_bulk_snapshot_region_miss",
    ] {
        assert!(units.contains(name), "missing d53 snapshot receipt unit {name}");
    }
}

#[test]
fn a30_reads_only_through_the_selected_domain53_memory() {
    let source = super::collector_source_lf();
    let observing = code_only(item_body(
        &source,
        "fn read_a30_domain53_engine_authority<",
    ));
    for read in [".read_i32", ".read_u64", ".read_u32"] {
        assert!(observing.contains(read), "A30 must take `{read}` from the snapshot");
    }
    for live in [
        "read_i32_stable",
        "read_u64_stable",
        "read_u32_stable",
        "Domain53InProcessMemory",
    ] {
        assert!(
            !observing.contains(live),
            "A30 may not bypass snapshot custody through `{live}`"
        );
    }
}

// ---------------------------------------------------------------------------
// The bracket copy measured as production runs it. Shared by the benchmark
// receipt and the residency law below.

const BENCH_PAGE_BYTES: usize = diplomacy53::snapshot_memory::DOMAIN53_SNAPSHOT_PAGE_BYTES;
const BENCH_MAX_REGIONS: usize = diplomacy53::snapshot_memory::DOMAIN53_SNAPSHOT_MAX_REGIONS;
const BENCH_MAX_BYTES: usize = diplomacy53::snapshot_memory::DOMAIN53_SNAPSHOT_MAX_BYTES;
const BENCH_MAX_OS_REGIONS: usize = diplomacy53::snapshot_memory::DOMAIN53_SNAPSHOT_MAX_OS_REGIONS;

#[link(name = "kernel32")]
extern "system" {
    fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
    fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
    fn K32QueryWorkingSetEx(process: *mut c_void, info: *mut c_void, bytes: u32) -> i32;
    fn K32EmptyWorkingSet(process: *mut c_void) -> i32;
}
const BENCH_MEM_COMMIT_RESERVE: u32 = 0x1000 | 0x2000;
const BENCH_MEM_RELEASE: u32 = 0x8000;
const BENCH_PAGE_READWRITE: u32 = 0x04;

#[repr(C)]
struct WorkingSetExInfo {
    address: *mut c_void,
    attributes: usize,
}

/// How many of `addresses` are on a page that is in the process working set
/// now (PSAPI_WORKING_SET_EX_BLOCK.Valid).
fn resident_pages_at(addresses: &[usize]) -> usize {
    if addresses.is_empty() {
        return 0;
    }
    let mut info: Vec<WorkingSetExInfo> = addresses
        .iter()
        .map(|&address| WorkingSetExInfo {
            address: address as *mut c_void,
            attributes: 0,
        })
        .collect();
    let size = (info.len() * core::mem::size_of::<WorkingSetExInfo>()) as u32;
    let ok = unsafe { K32QueryWorkingSetEx(GetCurrentProcess(), info.as_mut_ptr().cast(), size) };
    assert_ne!(ok, 0, "QueryWorkingSetEx");
    info.iter().filter(|entry| entry.attributes & 1 == 1).count()
}

/// Pages of `[base, base + bytes)` that are in the process working set now.
fn resident_pages(base: usize, bytes: usize) -> usize {
    let addresses: Vec<usize> = (0..bytes / BENCH_PAGE_BYTES)
        .map(|index| base + index * BENCH_PAGE_BYTES)
        .collect();
    resident_pages_at(&addresses)
}


/// A plan over `reads`, allowed up to `max_regions` ranges and `max_bytes`
/// bytes (so a shape ABOVE the production caps can be measured against them).
fn bench_plan_for(
    reads: Vec<(u64, usize)>,
    max_regions: usize,
    max_bytes: usize,
) -> diplomacy53::snapshot_memory::Domain53SnapshotPlan {
    let trace = diplomacy53::snapshot_memory::Domain53AccessTrace {
        reads: reads
            .into_iter()
            .map(|(address, len)| diplomacy53::snapshot_memory::Domain53ReadTrace {
                address,
                len,
                succeeded: true,
            })
            .collect(),
        ..diplomacy53::snapshot_memory::Domain53AccessTrace::default()
    };
    diplomacy53::snapshot_memory::Domain53SnapshotPlan::from_trace_with_limits(
        1,
        trace,
        max_regions,
        max_bytes,
    )
    .expect("a plan inside the given limits")
}

/// `count` reads of `len` bytes, `per_page` of them on each 4 KiB page from
/// `base` up: the SPATIAL shape a d53 plan actually has. Capture e2e_02's
/// worst hour put 393,117 merged ranges of ~68 bytes on 125,083 distinct
/// pages, so the copy walks a wide span and touches a fraction of it -- which
/// is what the cache and the TLB, not the byte total, charge for.
fn scattered_reads(base: u64, count: usize, len: usize, per_page: usize) -> Vec<(u64, usize)> {
    let stride = (BENCH_PAGE_BYTES / per_page) as u64;
    assert!(
        stride > diplomacy53::snapshot_memory::DOMAIN53_SNAPSHOT_MERGE_GAP_BYTES + len as u64,
        "the reads must not merge into one range"
    );
    (0..count)
        .map(|ordinal| {
            let page = (ordinal / per_page) as u64;
            let slot = (ordinal % per_page) as u64;
            (base + page * BENCH_PAGE_BYTES as u64 + slot * stride, len)
        })
        .collect()
}

fn pages_for(count: usize, per_page: usize) -> usize {
    count.div_ceil(per_page)
}

/// One committed read-write region of `pages` pages, filled with `fill`.
struct SourceRegion {
    base: *mut c_void,
    pages: usize,
}

impl SourceRegion {
    fn new(pages: usize, fill: u8) -> Self {
        let base = unsafe {
            VirtualAlloc(
                ptr::null_mut(),
                pages * BENCH_PAGE_BYTES,
                BENCH_MEM_COMMIT_RESERVE,
                BENCH_PAGE_READWRITE,
            )
        };
        assert!(!base.is_null(), "source region of {pages} pages");
        unsafe { ptr::write_bytes(base as *mut u8, fill, pages * BENCH_PAGE_BYTES) };
        Self { base, pages }
    }

    fn address(&self) -> u64 {
        self.base as u64
    }

    fn bytes(&self) -> usize {
        self.pages * BENCH_PAGE_BYTES
    }
}

impl Drop for SourceRegion {
    fn drop(&mut self) {
        assert_eq!(unsafe { VirtualFree(self.base, 0, BENCH_MEM_RELEASE) }, 1);
    }
}

/// Nearest-rank p95: the ceil(0.95 n)-th of n ascending samples.
fn p95_nearest_rank(sorted: &[u64]) -> u64 {
    let rank = (sorted.len() * 95).div_ceil(100);
    sorted[rank - 1]
}

/// Busy-spin for `ns` of wall time touching no memory; answers what it
/// actually took, so a lost scheduler quantum shows in the answer.
fn spin_for(ns: u64) -> u64 {
    let started = Instant::now();
    let mut spins = 0u64;
    while started.elapsed().as_nanos() < u128::from(ns) {
        spins = std::hint::black_box(spins.wrapping_add(1));
    }
    started.elapsed().as_nanos() as u64
}

/// One shape's receipt: the copy population, the PAIRED spin population
/// (a bracket-length spin taken right before every copy, under the same load
/// instance: what the machine charged a thread of that length in the same
/// moments), and the per-pair difference.
struct BenchShape {
    name: String,
    regions: usize,
    bytes: usize,
    queries: u32,
    load_threads: usize,
    arena_resident_min: usize,
    arena_resident_max: usize,
    copy_ns: Vec<u64>,
    spin_ns: Vec<u64>,
    /// Per-pair `copy - spin`, sorted.
    paired_delta_ns: Vec<i64>,
}

/// Samples more than 10 ms above the population's own median: on this
/// machine a lost scheduler quantum is 15-30 ms, so these are the samples
/// the scheduler took, in either population.
const QUANTUM_EVENT_EXCESS_NS: u64 = 10_000_000;
/// The service bar the receipt is read against.
const SERVICE_BAR_NS: u64 = 16_000_000;

fn over_bar(sorted: &[u64]) -> usize {
    sorted.iter().filter(|&&sample| sample > SERVICE_BAR_NS).count()
}

fn quantum_events(sorted: &[u64]) -> usize {
    let median = sorted[sorted.len() / 2];
    sorted.iter().filter(|&&sample| sample > median + QUANTUM_EVENT_EXCESS_NS).count()
}

impl BenchShape {
    fn print(&self) {
        let pairs = self.paired_delta_ns.len();
        eprintln!(
            "D53_GATED_COPY_BENCH shape={} regions={} bytes={} queries={} samples={} load_threads={} arena_resident_before_min={} arena_resident_before_max={} p50_ns={} p95_ns={} max_ns={} spin_p50_ns={} spin_p95_ns={} spin_max_ns={} paired_delta_median_ns={} paired_delta_p95_ns={} paired_delta_max_ns={} copy_quantum_events={} spin_quantum_events={} copy_over_bar={} spin_over_bar={}",
            self.name,
            self.regions,
            self.bytes,
            self.queries,
            self.copy_ns.len(),
            self.load_threads,
            self.arena_resident_min,
            self.arena_resident_max,
            self.copy_ns[self.copy_ns.len() / 2],
            p95_nearest_rank(&self.copy_ns),
            self.copy_ns[self.copy_ns.len() - 1],
            self.spin_ns[self.spin_ns.len() / 2],
            p95_nearest_rank(&self.spin_ns),
            self.spin_ns[self.spin_ns.len() - 1],
            self.paired_delta_ns[pairs / 2],
            self.paired_delta_ns[(pairs * 95).div_ceil(100) - 1],
            self.paired_delta_ns[pairs - 1],
            quantum_events(&self.copy_ns),
            quantum_events(&self.spin_ns),
            over_bar(&self.copy_ns),
            over_bar(&self.spin_ns),
        );
    }
}

/// One bracket exactly as production runs it: the plan's gate over
/// `domain53_snapshot_os_region`, the batched SEH copy, one query budget.
fn bench_capture(
    plan: &mut diplomacy53::snapshot_memory::Domain53SnapshotPlan,
    budget: u32,
) -> diplomacy53::snapshot_memory::Domain53SnapshotCaptureReceipt {
    plan.capture_with(domain53_snapshot_os_region, domain53_batch_copy, budget)
}

/// `samples` PAIRS of (spin of `spin_ns`, then one bracket of the production
/// copy over `plan`) under whatever load is running, so the spin and the copy
/// populations saw the same machine.
fn bench_measure(
    name: String,
    plan: &mut diplomacy53::snapshot_memory::Domain53SnapshotPlan,
    expected_regions: usize,
    expected_queries: u32,
    samples: usize,
    load_threads: usize,
    spin_ns: u64,
) -> BenchShape {
    let (arena_base, arena_bytes) = plan.arena_span();
    let expected_bytes = plan.byte_count();
    let mut copy_ns = Vec::with_capacity(samples);
    let mut spin_samples = Vec::with_capacity(samples);
    let mut deltas = Vec::with_capacity(samples);
    let (mut arena_min, mut arena_max) = (usize::MAX, 0usize);
    for _ in 0..samples {
        let spin = spin_for(spin_ns);
        let arena = resident_pages(arena_base, arena_bytes);
        arena_min = arena_min.min(arena);
        arena_max = arena_max.max(arena);
        let started = Instant::now();
        let receipt = bench_capture(plan, expected_queries.max(1));
        let copy = started.elapsed().as_nanos() as u64;
        assert_eq!(receipt.failure_region_ordinal, None);
        assert_eq!(receipt.copied_region_count as usize, expected_regions);
        assert!(!receipt.gate_budget_exhausted);
        assert_eq!(
            receipt.gate_queries,
            expected_queries.max(1),
            "{name}: one query per OS region"
        );
        copy_ns.push(copy);
        spin_samples.push(spin);
        deltas.push(copy as i64 - spin as i64);
    }
    copy_ns.sort_unstable();
    spin_samples.sort_unstable();
    deltas.sort_unstable();
    BenchShape {
        name,
        regions: expected_regions,
        bytes: expected_bytes,
        queries: expected_queries,
        load_threads,
        arena_resident_min: arena_min,
        arena_resident_max: arena_max,
        copy_ns,
        spin_ns: spin_samples,
        paired_delta_ns: deltas,
    }
}

/// Background load beside the bracket (owner ruling 2026-08-31: the bar is
/// met WITH background load; the lane fleet is the stand-in). Two kinds:
///   Compute    every thread runs a cache-resident hash loop over 256 KiB:
///              the fleet's compile jobs -- cores and SMT siblings busy, the
///              memory bus mostly free
///   Bandwidth  every thread copies 32 MiB back and forth: the memory bus
///              saturated, a stress bound no ordinary background produces
#[derive(Clone, Copy)]
enum LoadKind {
    Compute,
    Bandwidth,
}

struct BackgroundLoad {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl BackgroundLoad {
    fn start(threads: usize, kind: LoadKind) -> Self {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let workers = (0..threads)
            .map(|ordinal| {
                let stop = std::sync::Arc::clone(&stop);
                std::thread::spawn(move || match kind {
                    LoadKind::Bandwidth => {
                        let mut a = vec![1u8; 32 << 20];
                        let mut b = vec![2u8; 32 << 20];
                        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                            b.copy_from_slice(&a);
                            a.copy_from_slice(std::hint::black_box(&b));
                        }
                    }
                    LoadKind::Compute => {
                        let words: Vec<u64> = (0..(256 << 10) / 8)
                            .map(|i| (i as u64 + ordinal as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
                            .collect();
                        let mut acc = ordinal as u64;
                        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                            for word in &words {
                                acc = (acc ^ word).wrapping_mul(0x9E37_79B9_7F4A_7C15);
                                acc = (acc.rotate_left(23) ^ (acc >> 17)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                            }
                            std::hint::black_box(acc);
                        }
                    }
                })
            })
            .collect();
        // Let every worker reach its loop before anything is measured.
        std::thread::sleep(std::time::Duration::from_millis(100));
        Self { stop, workers }
    }
}

impl Drop for BackgroundLoad {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        for worker in self.workers.drain(..) {
            worker.join().expect("load thread");
        }
    }
}

/// The measured production demand: capture e2e_02's worst refused hour,
/// 393,117 merged ranges totalling 26,609,589 bytes over 125,083 pages.
const MEASURED_RANGES: usize = 393_117;
const MEASURED_RANGE_BYTES: usize = 68;
const MEASURED_RANGES_PER_PAGE: usize = 3;
/// What the page-granular planner turned that demand into, and refused.
const SUPERSEDED_PAGE_PLAN_PAGES: usize = 125_083;

/// The bracket copy measured as production runs it (the plan's VirtualQuery
/// gate plus `domain53_batch_copy`, one SEH frame per confirmed region, into
/// the plan's arena), PAIRED: every copy sample is preceded by a
/// bracket-length busy spin under the same load instance, so what the machine
/// charged a thread of that length in the same moments is on the same line.
/// Shapes:
///   measured_demand      capture e2e_02's worst hour: 393,117 ranges of 68
///                        bytes, 3 to a page, in ONE operating-system region
///   measured_demand_fragmented  the same ranges when the layout costs the
///                        whole query budget
///   superseded_page_plan the 125,083 pages / 512 MiB the page-granular
///                        planner asked for on that same hour and was refused
///   cap_worst_legal      the worst LEGAL plan at the caps: the region cap,
///                        the byte cap and the OS-region cap all met at once,
///                        at the measured spatial density
/// each in these conditions:
///   (none)               the fleet running as it was
///   _compute_loaded      available_parallelism - 1 cache-resident hash
///                        threads (the lane-fleet stand-in: the acceptance
///                        load)
///   _bandwidth_saturated the same thread count copying 32 MiB buffers (the
///                        memory bus saturated: a stress bound)
///
/// The COLD conditions the page-plan receipt carried are not repeated. The
/// sources are the game's own live heap, which the plan walk read end to end
/// on its own thread milliseconds earlier, and the arena is committed and
/// touched when the plan is built -- proven by
/// `d53_plan_arena_is_page_aligned_and_resident_when_built`, not by a timing.
///
/// Closing with D53_ZERO_WORK_PROBE: 50 us spins, 1,000 of them, under no
/// load and under each load. Its over-bar count and max are what the
/// machine does to a bracket of NO work; under CPU saturation they are the
/// floor no bracket can get under.
///
/// IGNORED in the suite: it saturates every core and holds over a gigabyte of
/// source pages, which perturbs every test running beside it (and they it).
/// Run it alone: `cargo test -p collector --lib -- --ignored
/// d53_gated_copy_bench_receipt --nocapture`.
#[test]
#[ignore]
fn d53_gated_copy_bench_receipt() {
    const UNLOADED_PAIRS: usize = 60;
    const LOADED_PAIRS: usize = 120;
    const WARMUP: usize = 5;
    const ZERO_WORK_PAIRS: usize = 1_000;

    let load_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .saturating_sub(1)
        .max(1);

    // The spin length of a shape: the median of a few unloaded copies.
    let spin_length = |plan: &mut diplomacy53::snapshot_memory::Domain53SnapshotPlan,
                       queries: u32| {
        let mut warm: Vec<u64> = (0..WARMUP)
            .map(|_| {
                let started = Instant::now();
                bench_capture(plan, queries.max(1));
                started.elapsed().as_nanos() as u64
            })
            .collect();
        warm.sort_unstable();
        warm[WARMUP / 2]
    };

    let run_conditions = |name: &str,
                          plan: &mut diplomacy53::snapshot_memory::Domain53SnapshotPlan,
                          regions: usize,
                          queries: u32| {
        let spin_ns = spin_length(plan, queries);
        bench_measure(name.to_string(), plan, regions, queries, UNLOADED_PAIRS, 0, spin_ns).print();
        for (suffix, kind) in [
            ("_compute_loaded", LoadKind::Compute),
            ("_bandwidth_saturated", LoadKind::Bandwidth),
        ] {
            let _load = BackgroundLoad::start(load_threads, kind);
            bench_measure(
                format!("{name}{suffix}"),
                plan,
                regions,
                queries,
                LOADED_PAIRS,
                load_threads,
                spin_ns,
            )
            .print();
        }
    };

    // measured_demand and superseded_page_plan share one source region: the
    // same hour, planned the two ways.
    {
        let pages = pages_for(MEASURED_RANGES, MEASURED_RANGES_PER_PAGE);
        let source = SourceRegion::new(pages, 0x5a);
        let mut plan = bench_plan_for(
            scattered_reads(
                source.address(),
                MEASURED_RANGES,
                MEASURED_RANGE_BYTES,
                MEASURED_RANGES_PER_PAGE,
            ),
            MEASURED_RANGES,
            MEASURED_RANGES * MEASURED_RANGE_BYTES,
        );
        assert_eq!(plan.region_count(), MEASURED_RANGES);
        assert_eq!(
            plan.count_os_regions(domain53_snapshot_os_region),
            1,
            "one reservation, one region"
        );
        run_conditions("measured_demand", &mut plan, MEASURED_RANGES, 1);
        drop(plan);

        let mut pages_plan = bench_plan_for(
            vec![(source.address(), SUPERSEDED_PAGE_PLAN_PAGES * BENCH_PAGE_BYTES)],
            1,
            SUPERSEDED_PAGE_PLAN_PAGES * BENCH_PAGE_BYTES,
        );
        run_conditions("superseded_page_plan", &mut pages_plan, 1, 1);
    }

    // measured_demand_fragmented: the same range count and bytes when the
    // first BENCH_MAX_OS_REGIONS - 1 ranges each sit in their own reservation,
    // so the bracket spends its whole query budget.
    {
        let singles: Vec<SourceRegion> = (0..BENCH_MAX_OS_REGIONS - 1)
            .map(|ordinal| SourceRegion::new(1, ordinal as u8))
            .collect();
        let rest = SourceRegion::new(
            pages_for(MEASURED_RANGES - singles.len(), MEASURED_RANGES_PER_PAGE),
            0xc3,
        );
        let mut reads: Vec<(u64, usize)> = singles
            .iter()
            .map(|page| (page.address(), MEASURED_RANGE_BYTES))
            .collect();
        reads.extend(scattered_reads(
            rest.address(),
            MEASURED_RANGES - singles.len(),
            MEASURED_RANGE_BYTES,
            MEASURED_RANGES_PER_PAGE,
        ));
        let mut plan = bench_plan_for(
            reads,
            MEASURED_RANGES,
            MEASURED_RANGES * MEASURED_RANGE_BYTES,
        );
        let queries = plan.count_os_regions(domain53_snapshot_os_region);
        assert_eq!(queries, BENCH_MAX_OS_REGIONS, "the whole query budget");
        run_conditions(
            "measured_demand_fragmented",
            &mut plan,
            MEASURED_RANGES,
            queries as u32,
        );
    }

    // cap_worst_legal: the region cap, the byte cap and the OS-region cap in
    // one plan, at the measured spatial density.
    {
        let range_bytes = BENCH_MAX_BYTES / BENCH_MAX_REGIONS;
        let singles: Vec<SourceRegion> = (0..BENCH_MAX_OS_REGIONS - 1)
            .map(|ordinal| SourceRegion::new(1, ordinal as u8))
            .collect();
        let scattered = BENCH_MAX_REGIONS - singles.len();
        let rest = SourceRegion::new(pages_for(scattered, MEASURED_RANGES_PER_PAGE), 0x3c);
        let mut reads: Vec<(u64, usize)> = singles
            .iter()
            .map(|page| (page.address(), range_bytes))
            .collect();
        reads.extend(scattered_reads(
            rest.address(),
            scattered,
            range_bytes,
            MEASURED_RANGES_PER_PAGE,
        ));
        let mut plan = bench_plan_for(reads, BENCH_MAX_REGIONS, BENCH_MAX_BYTES);
        assert_eq!(plan.region_count(), BENCH_MAX_REGIONS);
        assert!(plan.byte_count() <= BENCH_MAX_BYTES);
        let queries = plan.count_os_regions(domain53_snapshot_os_region);
        assert_eq!(queries, BENCH_MAX_OS_REGIONS);
        run_conditions("cap_worst_legal", &mut plan, BENCH_MAX_REGIONS, queries as u32);
    }

    // The zero-work probe: what the machine does to a bracket of no work.
    const ZERO_WORK_SPIN_NS: u64 = 50_000;
    let zero_work = |condition: &str, load_threads: usize| {
        let mut sorted: Vec<u64> = (0..ZERO_WORK_PAIRS)
            .map(|_| spin_for(ZERO_WORK_SPIN_NS))
            .collect();
        sorted.sort_unstable();
        eprintln!(
            "D53_ZERO_WORK_PROBE condition={condition} samples={} load_threads={load_threads} spin_ns={ZERO_WORK_SPIN_NS} p50_ns={} p95_ns={} max_ns={} over_bar={} quantum_events={}",
            sorted.len(),
            sorted[sorted.len() / 2],
            p95_nearest_rank(&sorted),
            sorted[sorted.len() - 1],
            over_bar(&sorted),
            quantum_events(&sorted),
        );
    };
    zero_work("unloaded", 0);
    for (condition, kind) in [
        ("compute_loaded", LoadKind::Compute),
        ("bandwidth_saturated", LoadKind::Bandwidth),
    ] {
        let _load = BackgroundLoad::start(load_threads, kind);
        zero_work(condition, load_threads);
    }
}

/// The plan's arena is one page-aligned allocation, every page of which is
/// committed and resident the moment the plan is built (on the plan-walk
/// thread), so the bracket's first copy faults nothing in. Proven with the
/// working-set query the kernel answers, not by inspection.
#[test]
fn d53_plan_arena_is_page_aligned_and_resident_when_built() {
    const RANGES: usize = 4_096;
    const RANGE_BYTES: usize = 68;
    let source = SourceRegion::new(pages_for(RANGES, MEASURED_RANGES_PER_PAGE), 0x11);
    let mut plan = bench_plan_for(
        scattered_reads(source.address(), RANGES, RANGE_BYTES, MEASURED_RANGES_PER_PAGE),
        RANGES,
        RANGES * RANGE_BYTES,
    );
    let (base, bytes) = plan.arena_span();
    assert_eq!(base % BENCH_PAGE_BYTES, 0, "page-aligned arena");
    assert_eq!(bytes, RANGES * RANGE_BYTES);
    assert_eq!(
        resident_pages(base, bytes),
        bytes / BENCH_PAGE_BYTES,
        "every destination page resident before the first bracket"
    );
    // The arena holds the ranges end to end, and the bracket fills all of it.
    let specs = plan.copy_specs();
    assert_eq!(specs[0].dst_offset, 0);
    let last = specs[specs.len() - 1];
    assert_eq!(last.dst_offset + last.len, bytes as u64);
    let receipt = bench_capture(&mut plan, BENCH_MAX_OS_REGIONS as u32);
    assert_eq!(receipt.copied_region_count as usize, RANGES);
    assert_eq!(receipt.copied_byte_count as usize, bytes);
    assert_eq!(receipt.gate_queries, 1, "one reservation, one query");
}
