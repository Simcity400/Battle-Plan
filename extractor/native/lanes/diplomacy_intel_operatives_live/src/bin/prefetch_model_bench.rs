//! Standalone synthetic model of the d53 relation-loop memory access pattern,
//! built to measure the software-prefetch distance curve OFFLINE (no live
//! hoi4.exe attach available this session). Lane: lane/w11-prefetch (W11).
//!
//! WHY A DEDICATED MODEL (benchmark-validity note — read before trusting a
//! number): the in-tree `FakeMemory` test harness stores the fake game bytes in
//! a `Vec<(base, bytes)>` chunk list and answers reads by LINEAR-SCANNING that
//! list, returning a *copy*. The "game address" it is keyed by (0x100_0000_0000+
//! …) is NOT the real process address the bytes live at, so a `_mm_prefetch` on a
//! game address warms a page that no read ever touches — FakeMemory structurally
//! cannot show any prefetch effect. To measure prefetch you MUST prefetch the
//! SAME real, mapped line the subsequent read dereferences. This bench therefore
//! models the pattern directly on a real heap buffer with identity addressing:
//!   * a >LLC pool of relation objects, each `ENTRY_STRIDE` bytes;
//!   * a `table` of entry indices in RANDOMISED order (pointer-chase: each
//!     iteration first reads a table slot to get the next entry, exactly like the
//!     live loop resolving `relation_slot_ptr` from `table_bytes` then reading the
//!     entry). Random order defeats the hardware stream prefetcher and the TLB,
//!     reproducing the DRAM-latency-bound scatter the live relations phase pays;
//!   * the per-iteration read copies the SAME 552-byte hot window the live loop
//!     reads (`[entry+0xDC, entry+0x304)` — SUBREL_COUNT..OPINION+4) and consumes
//!     the opinion i32 at +0x300, accumulating a checksum;
//!   * with prefetch ON it warms `future_entry+0x300` and `+0x288` `DIST` targets
//!     ahead — the exact two lines `extractor.rs` prefetches.
//!
//! WHAT TRANSFERS AND WHAT DOES NOT: absolute ns/iter are SYNTHETIC and will not
//! equal live d53 (live has the game's TLB pressure, allocator layout, cross-core
//! contention, and the real SEH copy primitive rather than a plain memcpy). What
//! transfers is the CURVE SHAPE: whether prefetch helps at all on a DRAM-bound
//! pointer-chase of this stride, and which distance saturates the benefit. The
//! checksum printed for every distance MUST be identical — that is the offline
//! byte-identity gate proving prefetch distance never changes the consumed value.
//!
//! Run: cargo run --release -p diplomacy_intel_operatives_live --bin prefetch_model_bench
//! Optional env: BENCH_POOL_MB, BENCH_ITERS, BENCH_TRIALS, BENCH_STRIDE.

use std::time::Instant;

const HOT_WINDOW_START: usize = 0xDC; // DOMAIN53_RELATION_SUBREL_COUNT
const HOT_WINDOW_END: usize = 0x300 + 4; // DOMAIN53_RELATION_OPINION + size_of::<i32>()
const HOT_WINDOW_LEN: usize = HOT_WINDOW_END - HOT_WINDOW_START; // 552
const OPINION_OFF: usize = 0x300; // absolute within entry
const SUBJECT_OFF: usize = 0x288; // absolute within entry

/// COVERAGE ARM (lane BA, 2026-08-05). The distance sweep above was settled on
/// live data (dist 8). What was never swept is WHICH LINES the hint block warms.
/// Production hints exactly two addresses — `entry+0x288` and `entry+0x300` —
/// but the read is a 552-byte `read_into` that STARTS at `entry+0xDC`, so the
/// copy's leading ~7 cache lines are never hinted and the copy stalls on its own
/// first line. These variants make that testable rather than argued; all of them
/// are pure `_mm_prefetch` hints, so the checksum gate below must hold across
/// every one of them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Coverage {
    /// Production today: the two FIELD lines only.
    Fields,
    /// The head of the copied window only (`entry+0xDC`).
    Head,
    /// Head plus the two field lines.
    HeadFields,
    /// Every 64-byte line the 552-byte copy actually touches.
    Span,
    /// EQUAL-TRIP-COUNT CONTROL for `Span` (Codex review finding D, 2026-08-05).
    /// `Fields` issues 2 hints and `Span` issues 10, so they differ in loop trip
    /// count, hint-instruction count, and how much lead time the FIRST hint of an
    /// iteration gets — not only in which lines are warmed. This arm issues
    /// exactly as many hints as `Span` but aims all of them at the two lines
    /// `Fields` already warms, so the only thing left between this arm and `Span`
    /// is COVERAGE. If `Span` beats this too, trip count is not the explanation.
    FieldsPadded,
}

impl Coverage {
    fn label(self) -> &'static str {
        match self {
            Coverage::Fields => "fields(0x288,0x300)",
            Coverage::Head => "head(0xDC)",
            Coverage::HeadFields => "head+fields",
            Coverage::Span => "span(all copied lines)",
            Coverage::FieldsPadded => "fields x N (trip ctrl)",
        }
    }
}

/// The byte offsets, within one relation entry, that a coverage variant hints.
/// `Span` walks the copied window `[HOT_WINDOW_START, HOT_WINDOW_END)` in
/// 64-byte steps and always includes the final line, so no touched line is
/// missed when the window length is not a multiple of the line size.
fn coverage_offsets(cov: Coverage, out: &mut Vec<usize>) {
    out.clear();
    match cov {
        Coverage::Fields => out.extend_from_slice(&[SUBJECT_OFF, OPINION_OFF]),
        Coverage::Head => out.push(HOT_WINDOW_START),
        Coverage::HeadFields => out.extend_from_slice(&[HOT_WINDOW_START, SUBJECT_OFF, OPINION_OFF]),
        Coverage::Span => {
            let mut off = HOT_WINDOW_START;
            while off < HOT_WINDOW_END - 1 {
                out.push(off);
                off += 64;
            }
            out.push(HOT_WINDOW_END - 1);
        }
        Coverage::FieldsPadded => {
            // Same hint COUNT as `Span`, aimed only at the two `Fields` lines,
            // alternating so the addresses (and therefore the address
            // computations) vary per hint exactly as they do in `Span`.
            let mut span = Vec::new();
            coverage_offsets(Coverage::Span, &mut span);
            for index in 0..span.len() {
                out.push(if index % 2 == 0 {
                    SUBJECT_OFF
                } else {
                    OPINION_OFF
                });
            }
        }
    }
}

#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[inline(always)]
fn prefetch(p: *const u8) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use core::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
        _mm_prefetch::<{ _MM_HINT_T0 }>(p as *const i8);
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = p;
    }
}

/// A dependent (latency-bound) compute chain seeded by the value just read, so
/// the optimizer cannot hoist it and it sits ON the critical path AFTER the read
/// — modelling the live per-pair work (opinion/state-event pushes, the subject
/// pointer hop, flag processing) that follows reading the opinion field. `work`
/// controls how much per-iteration compute the read latency must be hidden
/// behind; raising it shifts the prefetch-optimal distance DOWN, exactly as the
/// live loop (heavy per-pair work) differs from the bare-read model.
#[inline(always)]
fn dependent_work(seed: u64, work: u32) -> u64 {
    let mut x = seed | 1;
    for _ in 0..work {
        // multiply+xorshift dependent chain (each step needs the previous) —
        // latency-bound, ~a few cycles/step, not reorderable across the read.
        x = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        x ^= x >> 29;
    }
    x
}

/// One pass over the whole table. `dist == 0` => prefetch OFF (identical control
/// flow to the live "prefetch disabled" arm: no lookahead resolved, no hint).
/// Returns the opinion checksum, which MUST be independent of `dist` and `work`.
#[inline(never)]
fn pass(
    pool: &[u8],
    table: &[u32],
    entry_stride: usize,
    dist: usize,
    work: u32,
    hint_offsets: &[usize],
) -> u64 {
    let mut sum: u64 = 0;
    let mut carry: u64 = 0;
    let n = table.len();
    let base = pool.as_ptr();
    let mut buf = [0u8; HOT_WINDOW_LEN];
    for i in 0..n {
        // Pointer-chase lookahead (models the live block gated by
        // `prefetch_dist_relation != 0`): resolve the future entry from the table
        // (a near/sequential read, in cache) and warm the lines named by the
        // coverage arm under test.
        if dist != 0 {
            if let Some(&future) = table.get(i + dist) {
                let fe = future as usize * entry_stride;
                // Guard exactly like the live `is_heap_ptr(base+offset)` gate:
                // only prefetch addresses that lie inside the pool.
                for &off in hint_offsets {
                    if fe + off < pool.len() {
                        prefetch(unsafe { base.add(fe + off) });
                    }
                }
            }
        }
        let entry = table[i] as usize * entry_stride;
        // The scattered DRAM-bound read: copy the 552-byte hot window (models
        // `read_into(entry+0xDC, &mut buf)`).
        let src = entry + HOT_WINDOW_START;
        buf.copy_from_slice(&pool[src..src + HOT_WINDOW_LEN]);
        // CRITICAL (Codex refutation, finding 1): `black_box(&buf)` forces the
        // WHOLE 552-byte copy to be materialised. Without it the optimizer sees
        // that only the 4 opinion bytes are consumed and narrows the
        // `copy_from_slice` to a single 4-byte load at +0x300 — the exact line the
        // prefetch warms — which fabricates the prefetch benefit and models NOT the
        // live 552-byte `read_into` but a 4-byte scalar load. This escape hatch
        // makes the pointer to the full buffer observable, so LLVM must emit the
        // full copy that touches all 9 cache lines of the window (like live).
        // Pass the whole SLICE, not just the pointer value: `black_box` is
        // documented as best-effort, and handing it only `buf.as_ptr()` does not
        // reliably establish that all 552 copied bytes had to be materialised —
        // the checksum below consumes only the 4 opinion bytes, so an optimiser
        // that narrowed the copy to that one line would fabricate the prefetch
        // benefit and model a 4-byte load rather than the live 552-byte
        // `read_into` (Codex review finding D, 2026-08-05).
        std::hint::black_box(&buf[..]);
        // Consume the opinion i32 at entry+0x300 (== buf[0x300-0xDC]).
        let o = i32::from_le_bytes(
            buf[OPINION_OFF - HOT_WINDOW_START..OPINION_OFF - HOT_WINDOW_START + 4]
                .try_into()
                .unwrap(),
        );
        // Per-pair compute on the critical path AFTER the read (depends on the
        // opinion just read). It does NOT gate the next iteration's read address
        // (that comes from the precomputed table, independent — like live's
        // table_bytes), so out-of-order execution can overlap the next read with
        // this work; that overlap is precisely why heavier per-pair work lowers
        // the prefetch-optimal distance. `work == 0` reproduces the bare-read
        // model. The result feeds `sum`, so it is byte-live and cannot be elided,
        // yet it never changes the opinion checksum contribution (`o`).
        if work != 0 {
            carry = dependent_work(carry ^ (o as i64 as u64), work);
        }
        sum = sum.wrapping_add(o as i64 as u64);
    }
    // Force the (value-neutral) work carry to be materialised so the compiler
    // cannot dead-strip the per-pair work loop, WITHOUT folding it into the
    // returned checksum — so the printed checksum stays exactly the opinion sum
    // regardless of `work` (byte-identity across work levels too).
    std::hint::black_box(carry);
    sum
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default)
}

fn main() {
    let pool_mb = env_usize("BENCH_POOL_MB", 512);
    let iters = env_usize("BENCH_ITERS", 8_000_000);
    let trials = env_usize("BENCH_TRIALS", 7);
    let entry_stride = env_usize("BENCH_STRIDE", 0x340); // DOMAIN53_RELATION_SCAN_BYTES = 832

    let pool_bytes = pool_mb * 1024 * 1024;
    // Round entry count down so every entry (incl. its hot window) fits.
    let entries = (pool_bytes - HOT_WINDOW_END) / entry_stride;
    assert!(entries > 1, "pool too small for one entry");
    let real_pool = entries * entry_stride + HOT_WINDOW_END;

    println!("# prefetch_model_bench (SYNTHETIC — models d53 relation pointer-chase)");
    println!(
        "# pool={} MiB  entries={}  entry_stride=0x{:x}  hot_window={} B  iters={}  trials={}",
        pool_mb, entries, entry_stride, HOT_WINDOW_LEN, iters, trials
    );
    println!(
        "# WARNING: absolute ns are synthetic; the CURVE and the constant checksum are the signal."
    );

    // Real, mapped backing memory (a Vec is page-backed by the OS at a real
    // address; prefetch warms the exact lines the copy will read).
    let mut pool = vec![0u8; real_pool];
    let mut st = 0x1234_5678_9abc_def0u64;
    for b in pool.iter_mut() {
        *b = (splitmix64(&mut st) & 0xff) as u8;
    }
    // Randomised visit order over `iters` steps (pointer-chase scatter, with
    // revisits so the working set churns like a live per-src loop over shared
    // relation objects).
    let mut table = Vec::with_capacity(iters);
    for _ in 0..iters {
        table.push((splitmix64(&mut st) % entries as u64) as u32);
    }

    // Distances to sweep: 0 (OFF) + the requested {8,16,32,64} plus neighbours.
    let dists = [0usize, 4, 8, 12, 16, 24, 32, 48, 64, 96, 128];
    // Per-pair work levels (dependent-chain iters). 0 = bare-read model. Higher
    // levels approximate the heavier live per-pair compute; watch the optimal
    // distance move DOWN and the whole prefetch benefit shrink as work rises
    // (out-of-order execution overlaps more of the read on its own). A single
    // level can be forced with BENCH_WORK to focus one regime.
    let work_levels: Vec<u32> = match std::env::var("BENCH_WORK") {
        Ok(v) => vec![v.trim().parse().unwrap_or(0)],
        Err(_) => vec![0, 8, 32, 96],
    };

    // Warm the pages once (fault-in) so trial 1 is not dominated by first-touch.
    let warm = pass(&pool, &table, entry_stride, 0, 0, &[]);
    std::hint::black_box(warm);

    let mut global_checksum: Option<u64> = None;
    for &work in &work_levels {
        // INTERLEAVED measurement (Codex refutation, finding 5): rather than
        // measure all trials of dist=0, then all of dist=4, … (which lets thermal
        // / frequency / background drift alias onto distance), collect the trials
        // in a randomized interleaved schedule so every distance is sampled across
        // the whole measurement window. Medians are then taken per distance.
        let mut samples: Vec<Vec<f64>> = vec![Vec::with_capacity(trials); dists.len()];
        let mut order: Vec<usize> = (0..dists.len() * trials).map(|k| k % dists.len()).collect();
        // Fisher–Yates shuffle with the same SplitMix64 stream.
        for k in (1..order.len()).rev() {
            let j = (splitmix64(&mut st) % (k as u64 + 1)) as usize;
            order.swap(k, j);
        }
        let mut fields_hints: Vec<usize> = Vec::new();
        coverage_offsets(Coverage::Fields, &mut fields_hints);
        for &di in &order {
            let d = dists[di];
            let t = Instant::now();
            let sum = pass(&pool, &table, entry_stride, d, work, &fields_hints);
            let ns = t.elapsed().as_nanos() as f64 / iters as f64;
            samples[di].push(ns);
            std::hint::black_box(sum);
            // Byte-identity: the opinion checksum is invariant across every
            // (dist, work) cell. First observed value is the run reference.
            let ref_cs = *global_checksum.get_or_insert(sum);
            assert_eq!(
                sum, ref_cs,
                "byte-identity gate: opinion checksum changed with (dist={d}, work={work})"
            );
        }
        let mut baseline_ns = 0f64;
        println!(
            "\n=== per-pair work = {} dep-chain iters (interleaved, {} trials/cell) ===\n dist |  median ns/iter |   vs dist=0 |  checksum",
            work, trials
        );
        println!("------+-----------------+-------------+------------------");
        for (di, &d) in dists.iter().enumerate() {
            samples[di].sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median = samples[di][samples[di].len() / 2];
            let sum = global_checksum.unwrap();
            if d == 0 {
                baseline_ns = median;
            }
            let rel = if baseline_ns > 0.0 {
                (median / baseline_ns - 1.0) * 100.0
            } else {
                0.0
            };
            println!(
                " {:>4} | {:>15.3} | {:>+9.2} % | {:016x}",
                d, median, rel, sum
            );
        }
    }
    // ---- COVERAGE SWEEP (lane BA) -----------------------------------------
    // Distance is held at the live-settled default (8, `BENCH_COV_DIST` to
    // override) and the HINTED LINE SET is varied instead. Interleaved and
    // median-of-trials exactly like the distance sweep above, and reported as
    // paired deltas against the production arm (`fields`) rather than against
    // dist=0, because the question is "does moving the hints pay", not "does
    // prefetching pay" — that second question was already answered live.
    let cov_dist = env_usize("BENCH_COV_DIST", 8);
    let covs = [
        Coverage::Fields,
        Coverage::Head,
        Coverage::HeadFields,
        Coverage::FieldsPadded,
        Coverage::Span,
    ];
    let mut hint_sets: Vec<Vec<usize>> = Vec::with_capacity(covs.len());
    for &c in &covs {
        let mut v = Vec::new();
        coverage_offsets(c, &mut v);
        hint_sets.push(v);
    }
    for &work in &work_levels {
        let mut samples: Vec<Vec<f64>> = vec![Vec::with_capacity(trials); covs.len()];
        let mut order: Vec<usize> = (0..covs.len() * trials).map(|k| k % covs.len()).collect();
        for k in (1..order.len()).rev() {
            let j = (splitmix64(&mut st) % (k as u64 + 1)) as usize;
            order.swap(k, j);
        }
        for &ci in &order {
            let t = Instant::now();
            let sum = pass(
                &pool,
                &table,
                entry_stride,
                cov_dist,
                work,
                &hint_sets[ci],
            );
            let ns = t.elapsed().as_nanos() as f64 / iters as f64;
            samples[ci].push(ns);
            std::hint::black_box(sum);
            let ref_cs = *global_checksum.get_or_insert(sum);
            assert_eq!(
                sum, ref_cs,
                "byte-identity gate: opinion checksum changed with coverage {} (work={work})",
                covs[ci].label()
            );
        }
        println!(
            "\n=== COVERAGE sweep, dist={} , per-pair work = {} ({} trials/cell, interleaved) ===\n \
             coverage            | hints | median ns/iter | vs production",
            cov_dist, work, trials
        );
        println!("---------------------+-------+----------------+--------------");
        let mut production_ns = 0f64;
        for (ci, &c) in covs.iter().enumerate() {
            samples[ci].sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median = samples[ci][samples[ci].len() / 2];
            if c == Coverage::Fields {
                production_ns = median;
            }
            let rel = if production_ns > 0.0 {
                (median / production_ns - 1.0) * 100.0
            } else {
                0.0
            };
            println!(
                " {:<20}| {:>5} | {:>14.3} | {:>+11.2} %",
                c.label(),
                hint_sets[ci].len(),
                median,
                rel
            );
        }
    }

    println!("\n# checksum constant across all (distance, work, coverage) cells => prefetch placement is value-neutral (offline byte-identity gate PASSED).");
}
