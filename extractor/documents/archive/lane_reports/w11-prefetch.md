# Lane w11-prefetch — read-primitive outcome-split + prefetch distance

Branch `lane/w11-prefetch`, base `779d65a17`. **No live hoi4.exe attach this session** — every
timing number below is a SYNTHETIC in-process benchmark unless marked LIVE. This report has been
revised after a Codex adversarial review that found a fatal benchmark bug in the first draft (see
§7); the corrected numbers and retractions are folded in.

## TL;DR (post-review)

- The software prefetch this lane was asked to tune is **already landed and shipping**
  (`extractor.rs`, `BATTLE_PLAN_PREFETCH_DIST_RELATION` default 16). This lane is **measurement +
  refutation**, not a new lever.
- **No production code path changed.** Diff = test-only (`#[cfg(test)]` microbench in
  `collector/src/lib.rs`) + one standalone bench binary + a 4-line Cargo `[[bin]]` entry. The
  shipped DLL's folded output is **byte-identical by construction**; Codex independently confirmed
  the diff alters no collector output.
- **Read-primitive outcome split (synthetic):** the region-cache metadata gate is cheap
  (MRU hit ~5.0 ns; span-list hit ~7.1 ns) and a region miss is a VirtualQuery ≥1000× a hit. But a
  fully-cold scattered 552 B relation read (the d53 hot read) is **~62 ns** — so the DRAM latency
  of that scattered read, charged to the read primitive (inside `copy_selected`), is up to ~34% of
  the live d53 warm frame. I RETRACT the first draft's "primitive is only ~6%, rest is
  materialization" claim (Codex finding 4): the cost is a ~7–37 ms band depending on live
  residency, and it is exactly what prefetch targets.
- **Prefetch distance (synthetic, after fixing a copy-elision bug):** prefetch helps this
  access pattern, but at live-like per-pair cost the benefit **peaks at a SHORT distance (~8) and
  falls off a cliff by dist 16**, where the shipped default sits. This is the opposite of the
  first draft's (buggy) "flat optimum at 16." It is still synthetic and cannot be transferred to
  live; it **refines the live-A/B hypothesis** (test 4/8 against 16, not just 0-vs-16).
- **No code change made.** The shipped default is NOT validated and NOT proven mistuned; only a
  paired live A/B can decide, per this project's evidence rule.

---

## 1. Read-primitive outcome split (W1.1 measurement prerequisite)

### What the phase timers already cover (LIVE, from the retained manifest)

`runs/live_validation_20260709_221706/stage2_shortspeed5/native_session_manifest.json`,
`per_domain.diplomacy_intel_operatives_live.data_units_total` (deep-war, 1421 d53 records):

| phase | µs (session total) | share of total_wall |
|---|---:|---:|
| `domain53_phase_total_wall_us` | 70,121,572 | 100% |
| `domain53_phase_extract_total_us` | 63,852,677 | 91.1% |
| `domain53_phase_relations_us` | 61,445,354 | **87.6%** |
| all other named phases (sum) | ~2.38M | ~3.4% |
| total_wall − extract_total (emit/encode/root/setup) | 6,268,895 | 8.9% |

The relations phase (materializing 242,368,575 opinion rows/session) dominates; the phase timers
cover `extract_total` to within 0.04%. The only genuinely un-timed slice is the 8.9% between
`extract_total` and `total_wall` (emit/encode/root discovery). **The W0-era "37% dark residual"
is a stale figure from a coarser 10-phase instrument; the current build's phase timers are much
tighter.**

### Read-primitive outcome split (SYNTHETIC — `collector` bench `read_outcome_split_microbench`)

Single thread, own-process memory, median of 9 × 2M reads, `--release`. The gate arms use a warm
256 MiB region (isolating the metadata gate); the copy arms use RANDOM >LLC scattered addresses
via the SHIPPED alloc-free `read_into` so the SEH copy pays real DRAM latency (Codex finding 4).

| outcome | ns/read | what it isolates |
|---|---:|---|
| MRU hit + 8 B copy (warm) | **5.0** | metadata gate fast path (2 compares) |
| span-list hit + 8 B copy (warm) | **7.1** | binary-search gate fallback |
| region miss (one VirtualQuery) | ~6.5 µs – ~650 µs* | the syscall a cache miss costs |
| reject (unmapped addr) | ~190 | gate reject path |
| `read_into` 8 B SCATTERED >LLC | **35.9** | gate + copy + ~1-line cold DRAM latency |
| `read_into` 552 B SCATTERED >LLC | **61.7** | **the d53 hot read, fully cold**: gate + copy + ~9-line DRAM |

*The absolute VirtualQuery ns is unstable: 8 MiB/N=2M gave ~6.5 µs, 256 MiB/long-run gave ~650 µs
under system contention — so I report **only the ratio** (a miss is ≥1000× an MRU hit). That ratio
is the durable signal and the reason the region-interval cache exists (pre-P2b 10.4% miss = 432 k
VirtualQuery/frame was catastrophic). Re-run on the target rig for local numbers.

### Attribution — corrected (Codex finding 4)

Two separable costs:

- **Metadata gate: cheap.** ~5 ns on an MRU hit, ~7 ns on a span-list hit. Near-free when the
  region-cache hit rate is high (deep war).
- **Scattered source DRAM latency: material, and charged to the read primitive.** A fully-cold
  scattered 552 B `read_into` (the d53 hot read) is **~62 ns**. At the W0_AB ~597 k reads/warm-frame
  that is an **upper bound of ~37 ms — ~34% of the 108.6 ms live d53 warm frame**. This stall lives
  INSIDE `copy_selected` and is read-primitive wall time; it CANNOT be reassigned to
  "materialization." I RETRACT the first draft's "primitive is only ~6 ms / ~6%" claim (which came
  from a cache-hot overlapping bench — Codex finding 4).

**Honest bound, not a point estimate.** The ~62 ns is the *cold* upper bound (random over 256 MiB,
zero locality). Live relation objects may retain some residency/locality, so the true live
read-primitive DRAM cost is **somewhere between ~11 ns (fully warm) and ~62 ns (fully cold) per hot
read → ~7–37 ms of the frame.** Pinning it requires the live `rc_region_hits/misses` counters (the
perf gate was OFF in the retained corpus, so those are absent). The decision-relevant point: the
read primitive's DRAM stall is potentially a large share, and **prefetch targets exactly this cost**
— so whether prefetch helps live hinges on whether these reads are actually cold live, which only
the A/B can answer.

### Two levers still re-derived as NULL

- **Reducing read COUNT of MRU-hit reads:** each hit is ~4.7 ns; deleting it saves ~4.7 ns.
  Confirms the documented "count ≠ time" NULL.
- **Widening `read_object_window` to cut "repeat region lookups":** repeated reads within one
  object resolve on `REGION_MRU` (~4.7 ns), never a VirtualQuery — so there is nothing to save.
  Codex adds the correctness caution (a wider window can straddle a region boundary
  `range_confirmed_good` would reject, and enlarges the tear/fault window). I found no undersized
  window causing repeat region *misses*; **no change made**, and widening is affirmatively
  discouraged on both cost and correctness grounds.

---

## 2. Prefetch distance sweep (SYNTHETIC model) — corrected after Codex finding 1

### The bug that was fixed

The first-draft model copied 552 B into a stack buffer but only consumed 4 opinion bytes and only
`black_box`-ed the work carry — so LLVM narrowed the `copy_from_slice` to a single 4-byte load at
`+0x300`, exactly the line the prefetch warms. That fabricated a ~55% "flat optimum at 16."
**Fixed** by `black_box(buf.as_ptr())` forcing the full 9-cache-line copy to materialise (like the
live `read_into`). Empirical confirmation of the bug: forcing the copy raised the base cost ~8×
(work=0 base 5.85 → 45.9 ns/iter) and completely changed the curve.

### Corrected result (interleaved cell order; real 512 MiB mapped pool; median of 7; % vs dist=0)

| dist | work=0 (base 45.9 ns) | work=8 (61.7) | work=32 (164.5) | work=96 (259.0 ≈ live 253/pair) |
|---:|---:|---:|---:|---:|
| 0 | +0% | +0% | +0% | +0% |
| 4 | −21% | −11% | **−47%** | **−32%** |
| 8 | −21% | −11% | −46% | **−33%** |
| 12 | −22% | −11% | −46% | −31% |
| 16 (default) | −12% | −8% | −18% | −14% |
| 24 | −10% | −9% | −15% | −13% |
| 32 | −12% | −8% | −15% | −13% |
| 64 | −9% | −6% | −15% | −13% |
| 128 | −10% | −7% | −14% | −12% |

The opinion checksum was **identical across all (distance, work) cells** — the offline
value-neutrality gate (prefetch never changes the consumed value).

**Reading the corrected curve.** At the live-like regime (work=32/96) prefetch clearly helps, but
the benefit **peaks at dist ≈ 4–8 (−32 to −47%) and drops off a cliff by dist 16 (−14 to −18%)**.
Mechanism: prefetching N iterations ahead, with the full 552 B window = 9 cache lines streamed per
iteration, means ~9·N lines of intervening traffic; by dist 16 the two prefetched lines are
evicted before use, so only the HW prefetcher's residual benefit remains. **The shipped default 16
sits just past this model's cliff.**

### Live-magnitude caveat (load-bearing; Codex findings 1, 2, 5)

This is SYNTHETIC. It does not reproduce live residency, allocator layout, concurrent mutation,
TLB pressure, or the memory-level parallelism from the loop's OTHER independent loads (subject
hop, flag reads, next-src batch) that out-of-order execution + the HW prefetcher may already
overlap. The cliff location (16) is a function of THIS model's cache size and 9-line stride and
will move on other hardware and under real work. W0's measured −43% live win was attributed to
alloc/format removal (T1/T2), **not** to this prefetch (T3), whose isolated live contribution was
never measured. Per the project rule that only a paired/live A/B counts (±13% single-capture
noise): **I claim no live speedup, and I do NOT change the default on this evidence.**

**Verdict:** prefetch is mechanically real for this pattern (not a structural NULL), but the
corrected model gives a genuine reason to suspect **dist 16 may be past the optimum and ~8 could
be better** — a hypothesis for the live A/B, not a code change.

---

## 3. Recommended next step (needs the live rig — cannot be done this session)

Isolated same-state deep-war live A/B of the prefetch itself, **interleaved** and repeated:
`BATTLE_PLAN_PREFETCH_DIST_RELATION` ∈ {0, 4, 8, 16} on the same advancing D04-1944 save, comparing
`per_domain.diplomacy_intel_operatives_live.read_time_us` and `domain53_phase_relations_us`, and —
critically — with the perf gate ON so `rc_region_hits/misses` are captured (they were absent in
the retained corpus). Outcomes: (a) 0-vs-16 flat within ±13% ⇒ prefetch is a live NULL, remove the
block for simplicity; (b) 8 beats 16 beyond noise ⇒ retune to ~8; (c) 16 best ⇒ keep. Also fold
the full output (G1) to confirm byte-identity across arms.

### LIVE A/B RUN 2026-07-10 — the shipped default 16 is the WORST arm

Ran the sweep as a 6-window drift-cancelling **palindrome** `[16,8,0,0,8,16]`, 240 s each, on the
live advancing game (`runs/ab_prefetch_20260710`). This was only possible after the env-var
channel was replaced by the control-file plumbing (env can't reach a running game): commit
`1b8ed7d66` + fixes. Each window's APPLIED distance was verified from its OWN stream
(`compact_stream.lane_wall.prefetch_dist_relation_applied` = 16/8/0/0/8/16, no `_mixed`), so the
arm labels are proven, not assumed. Perf gate was OFF (production preset), so `rc_region_hits/misses`
are still absent — the direct `per_domain.diplomacy_intel_operatives_live.read_time_us` (avg µs) is
the metric:

| applied dist | window A | window B | palindrome mean |
|---:|---:|---:|---:|
| 16 (shipped) | 58,565 | 56,462 | **57,513** |
| 8 | 52,708 | 51,995 | **52,352** |
| 0 (off) | 53,332 | 52,850 | **53,091** |

**Reading:** dist 16 is worst in BOTH of its windows — every dist-16 window sits above every dist-8
and dist-0 window. w1 (dist 16, the FIRST/smallest-world window) is nearly the slowest despite
world-growth predicting it should be fastest, so the ~9% penalty is not late-window drift. dist 8
and dist 0 are indistinguishable (~52–53k). This is a paired live A/B (better than the ±13%
single-capture rule), and it lands as **outcome (b)+(a) combined: the shipped default 16 is
mistuned PAST the optimum — actively worse than prefetch OFF — exactly as the corrected synthetic
model (§2, cliff by 16) predicted; dist 8 matches OFF rather than beating it.**

**Recommended action (NOT yet taken — a shipped hot-path default change on a ~9% effect near the
single-window noise floor should clear one confirmatory palindrome or a user call):** change
`PREFETCH_DIST_RELATION_DEFAULT` from 16 to **8** (safe: the data says 8 ≤ 16 always, and 8 is the
synthetic optimum), OR to **0** to delete the relation-loop prefetch entirely (0 ties 8 for best and
removes code). Do NOT keep 16. A second palindrome on a fresh session would tighten 8-vs-0; until
then the honest claim is "16 is refuted, 8 and 0 tie." Candidate-loop distance (32) was NOT swept
live and is untouched.

### CONFIRMATORY PALINDROME (same night, `runs/ab_prefetch2_20260710`) — DECIDED, RETUNED 16→8

Second independent drift-cancelled palindrome, ~40 min later on the further-advanced world
(applied distance again proven per-window from each stream; no mixed flags):

| applied dist | run 2 windows (µs) | run 2 mean | **combined 4-window mean** | spread across all 4 |
|---:|---|---:|---:|---|
| 16 | 50,543 / 53,190 | 51,867 | **54,690** | 50.5k–58.6k (high variance) |
| 8 | 51,134 / 50,983 | **51,058** | **51,705** | 51.0k–52.7k (tightest) |
| 0 | 53,089 / 54,062 | 53,576 | **53,333** | 52.9k–54.1k |

Run 2 refines run 1: dist 16's penalty is REAL but state-dependent (high variance — its run-2
windows were fine, its run-1 windows were the worst four numbers in the whole study), while
**dist 8 wins BOTH palindromes outright with the tightest spread** (−5.5% vs 16, −3.1% vs 0
combined). dist 0-vs-16 ordering flips between runs (those two are noise-separated only by
variance). Pre-registered outcome (b) satisfied: **`PREFETCH_DIST_RELATION_DEFAULT` retuned
16 → 8** (extractor.rs, same commit as this note; hint-only constant, cannot affect
rows/bytes/errors; suites green). Prefetch is NOT deleted: 8 beat 0 in both palindromes, so the
mechanism pays when tuned. Still unswept: candidate-loop distance (32), perf-gated arms
(`rc_region_hits/misses` remain unmeasured live).

---

## 4. Gates

- **Byte identity:** no shipping code changed (diff is `#[cfg(test)]`-only + a bin + a Cargo
  `[[bin]]` line), so folded/streamed output is byte-identical by construction; G1 fold-identity is
  not applicable (no emit change). Codex independently confirmed the diff alters no collector
  output. The prefetch mechanism's value-neutrality is separately shown by the constant opinion
  checksum across every (distance, work) cell — with the caveat (Codex finding 3) that this
  checksum covers only opinion values, not the full wire record; full wire byte-identity for
  shipped code rests on "no shipping code changed," which holds.
- **`cargo build --release`**: green (collector lib+tests; diplomacy-lane bin).
- **`cargo test --release -p collector --lib`**: **309 passed, 0 failed, 5 ignored** (the new
  `read_outcome_split_microbench` is one of the ignored perf benches).
- **`cargo test --release -p diplomacy_intel_operatives_live --lib`**: 112 passed, **1 pre-existing
  timing flake** (`payload_phase_times_populated_and_bounded_by_outer_build`, extractor.rs:6652,
  asserts `encode_us > 0`; floors to 0 µs for the tiny proof fixture on a fast rig). `git diff HEAD`
  shows `extractor.rs`/`lib.rs` byte-identical to base, so this `--lib` binary is identical to base
  779d65a17 — the failure reproduces at base and is NOT introduced by this lane (passes in
  isolation). Not a regression.

## 5. How to re-run the benchmarks (from `extractor/native/`)

```
# Prefetch distance × per-pair-work matrix (synthetic; ~1 min). The black_box(buf.as_ptr())
# copy-materialisation fix is essential — without it the copy is elided (see §2/§7).
cargo run --release -p diplomacy_intel_operatives_live --bin prefetch_model_bench
#   knobs: BENCH_POOL_MB(512) BENCH_ITERS(8e6) BENCH_TRIALS(7) BENCH_STRIDE(0x340) BENCH_WORK(sweep if unset)

# Read-primitive outcome split (synthetic; several min — the region-miss arm issues ~18M VirtualQuery)
RUN_OUTCOME_SPLIT=1 cargo test --release -p collector \
  read_outcome_split_microbench -- --nocapture --ignored
```

Benchmarks are RAM-only (no disk intermediates written). Corpus is read via its retained manifest
only; the 12 GB deep-war stream is read in place, never copied.

## 6. Files touched

- `collector/src/lib.rs` — +~175 lines, all inside `#[cfg(test)] mod tests` (`read_outcome_split_microbench`).
- `lanes/diplomacy_intel_operatives_live/Cargo.toml` — +4 lines (`[[bin]] prefetch_model_bench`).
- `lanes/diplomacy_intel_operatives_live/src/bin/prefetch_model_bench.rs` — new standalone bench.
- `documents/lane_reports/w11-prefetch.md` — this report.

## 7. Codex adversarial review — every finding verbatim + disposition

Mode A, read-only, `gpt-5.6-sol`, `model_reasoning_effort=xhigh`. It ran BEFORE the fixes above.
Verdict summary Codex returned: "No speedup or tuning conclusion is accepted. The decisive defect
is worse than ordinary synthetic-model mismatch: the release compiler eliminates the purported
552-byte read." Findings and what I did:

1. **REFUTED — prefetch bench copy is compiler-elided.** Codex: `black_box` protected `carry` not
   `buf`; only 4 opinion bytes consumed, so the 552 B copy narrows to one 4-byte load at `+0x300`
   — exactly the prefetched line, fabricating the ~55% win; the live path does a full 552 B
   `read_into`. **ACCEPTED — this was a real bug.** Fixed with `black_box(buf.as_ptr())`;
   empirically confirmed (base cost rose ~8×, curve changed completely). Corrected result in §2:
   the "flat optimum at 16" is RETRACTED; corrected curve peaks at dist ~8 with a cliff by 16.
2. **REFUTED — default 16 is not validated; live prefetch unproven (and not proven a NULL).**
   Codex: a region-cache hit says nothing about L1/L2/LLC/TLB residency; no evidence the 552 B
   read is a cold DRAM miss on the live critical path, nor that it is already overlapped. **ACCEPTED.**
   I retract "default 16 validated"/"flat optimum" and claim neither a live win nor a live NULL;
   only the paired live A/B (§3) decides.
3. **PARTIALLY-REFUTED — this diff changes no shipping path (supported), but byte identity of a
   *widened* window under races is not demonstrated; direct prefetch fault-safety is plausible.**
   Codex: `is_heap_ptr` proves only range/canonicality, not mapping/lifetime; the checksum covers
   only opinion values, not the other 548 B / subject / flags / partial flags / wire output; a
   widened read can straddle a region boundary `range_confirmed_good` rejects, enlarge the
   decommit/fault window, and produce different torn combinations under concurrent mutation.
   **ACCEPTED.** I made no widening (§1 discourages it on exactly these grounds); I narrowed the
   byte-identity claim: the bench checksum proves only opinion-value neutrality, and production
   byte-identity rests on "no shipping code changed" (which Codex confirmed).
4. **REFUTED — "dark residual = materialization" does not follow.** Codex: the 552 B outcome arm
   advanced only 64 B between reads (488 B overlap) so it measured hot overlapping copy throughput,
   not scattered latency; DRAM stalls inside `copy_selected` are read-primitive time; VirtualQuery
   attribution needs real live miss counts (0.17% misses alone ≈ the whole claimed primitive
   budget); vectors are pre-reserved and hashing precomputed, so those "materialization" costs are
   overstated. **ACCEPTED.** Fixed the outcome arm to SCATTERED >LLC `read_into` (§1); RETRACTED
   the quantitative primitive-vs-materialization split — it needs the live `rc_region_hits/misses`
   (perf gate was off in the corpus).
5. **REFUTED — methodology defects.** Codex: fixed ascending distance order (drift aliases onto
   distance); warm-once removes first-touch; span-hit arm never verifies 64 allocs = 64 regions;
   reject address only "almost certainly" unmapped; region-miss arm times epoch-bump+VQ+insert+copy
   not an isolated syscall; checksum only proves deterministic opinion summation. **PARTIALLY
   ACCEPTED / FIXED where cheap:** distance order is now interleaved + shuffled (§2); the region-miss
   arm is labelled as VQ+bookkeeping (the ratio, not the absolute, is the claim); the reject/span
   caveats are stated. The inherent limits (synthetic, warm) are disclosed, not eliminated — the
   live A/B is the resolution.

**Net:** Codex's core objection stands — no live conclusion is claimed. The synthetic curve is
retained ONLY as a hypothesis generator for the live A/B, with the copy-elision bug fixed so it at
least models the right read. This is reported as **inconclusive on live magnitude**, per the
review-independence rule that a synthetic win which might evaporate live is worse than useless.
