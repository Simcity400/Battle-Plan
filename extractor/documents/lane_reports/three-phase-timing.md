# lane/three-phase-timing — phase-1 guarded-acquisition timing (2026-07-10)

Dispatch-queue Job 1 (`CODEX_DISPATCH_QUEUE.md`). Implemented by the orchestrator directly
(Codex quota outage; user authorized temporary Opus subagents 2026-07-10 — used for the
adversarial review, not the implementation, so implementer ≠ reviewer holds).

## What this measures and why

The burst-correlation study reframed zero-skip as a ~30–40 ms whole-cycle problem (a ~3.8×
cut from the ~151 ms sweep). The acquire/process split is a candidate lever, but it only pays
if **materialisation** (row building) is a large share of lane wall — and that split was
unmeasured: the lane-wall counter gives per-domain wall, `encode_append_span_us` gives frame
encode, and nothing separated "touching game memory" from "building rows" inside a lane.

This lane adds **phase-1 (guarded acquisition) time**, per domain, riding the existing
domain-250 read-counter probe (zero recorder_host changes — the host routes domain-250
diagnostic units into the manifest's `read_ab_probe` name→sum summary):

- `rc_d{id}_guarded_spl_ns` / `rc_d{id}_guarded_spl_n` — SAMPLED time/count over the hot
  logical read primitives (`read_scalar`, `read_bytes`, `read_into`).
- `rc_d{id}_scan_guarded_ns` — UNSAMPLED time over `read_bytes_scan` (heap-scan chunks).
- Frame-level twins: `rc_{seh,rpm}_guarded_spl_ns`, `..._guarded_spl_n`, `..._scan_guarded_ns`.

**Offline estimator (per domain, per capture segment):**

```
guarded_ns(d) ≈ guarded_spl_ns(d) × (reads(d) / guarded_spl_n(d)) + scan_guarded_ns(d)
materialise_ns(d) ≈ lane_wall_us(d)×1000 − guarded_ns(d)
phase 3 = encode_append_span_us (frame-level; known-small, d53 ≈ 7.9 ms)
```

The estimator scales by the *reported* reads/samples ratio, not the nominal stride — that
corrects sample QUANTITY. Representativeness needs the per-thread phase seed (see the 1R
review section below; the original "cannot bias" claim here was retracted). Per-domain rows exist ONLY for domains that open an RcDomainScope; a missing row means
unattributed (global-only), NOT free — e.g. d22 map_control opens no scope. Among scoped
domains, the sharded ones (200, 20, 23, 26, 44) report a main-thread FLOOR only. See the
1R/1R2 sections.

## Why sampled, and the self-cost budget

Manifests of record measure **2.06–3.41 M logical reads/frame** (w3a0_s2 3.10 M, w3a0_war
3.41 M, production_speed5 2.06 M). Direct timing = 2 QPC calls/read ≈ 40–60 ns × 3 M ≈
**120–200+ ms/frame — more than the phase being measured**. Dead on arrival; sampling is not
optional here.

- Stride: every 64th logical read (`RC_SPL_MASK = 63`, free-running per-thread counter that
  deliberately survives domain-scope flushes — a drained counter would over-sample the first
  read after every flush).
- Gate-ON self-cost, MEASURED (`rc_probe_self_cost_measure`; the original "does not grow"
  claim was wrong): probe machinery = **+1.1–1.6 ns/read over pre-lane rc_bump**, i.e.
  **~3.4–5 ms/frame at 3 M reads**, sitting in lane_wall but NOT in sampled guarded time —
  the estimator prints and must subtract this tax. QPC pairs are inside the sampled windows.
- Gate OFF (production): one relaxed-load early-return per read (pre-lane bulk reads paid
  up to two loads — a benign reduction, not literal identity); stream byte-identical (the
  probe record was already perf-gate-only).
- `read_bytes_scan` is timed on EVERY call, not sampled: few calls/frame at ~ms each — a
  1/64 sample would misweight the tail badly, and two QPC reads against a millisecond body
  are free.
- NANOSECOND accumulation everywhere: a scalar read is ~100–300 ns and would truncate to 0
  in microseconds.

## What changed (all in `collector/src/lib.rs`)

- `RC_GUARDED_SPL_NS/N`, `RC_SCAN_GUARDED_NS` counter slots (RC_N 8→11); per-domain twins
  (RC_PD_N 4→7) + `rc_pd_index` routing; names in both frame arrays + the leaked per-domain
  name set.
- `RcLocal.spl_ctr` (free-running, not drained by `flush_indexed`).
- `rc_read_probe(bulk_len)` / `rc_read_probe_end` — replaces the direct `rc_bump` pairs in
  `read_scalar` / `read_bytes` / `read_into`; identical counter semantics (verified by the
  pre-existing count-based tests passing unchanged).
- `rc_scan_probe` / `rc_scan_probe_end` in `read_bytes_scan` (all exits).
- Tests: stride determinism (exactly every 64th at pinned phase) + per-domain routing;
  scan-probe accumulation; gate-OFF inertness (no counts, no samples); updated name-table
  test; `eventually()` retry harness because RC_ENABLED is process-global and fixture frame
  assembly stores it concurrently (the pre-existing rc test lives with a µs window; these
  have ~100 µs windows, so they detect-and-retry instead of flaking).

## Gates

| gate | result |
|---|---|
| cargo check --release (workspace) | **PASS** — 10.2 s, zero new warnings (all 11 pre-existing, none touch new symbols) |
| full workspace test suite (`--no-fail-fast`) | **PASS** — 1216 passed / 1 failed; the 1 is the documented pre-existing flake `payload_phase_times_populated_and_bounded_by_outer_build` (extractor.rs:6652, encode_us floors to 0 on a fast rig; reproduced on clean base by two prior lanes). Base 1214 + 3 new tests = 1217 total. |
| G1 fold identity | **PASS** — lane fold_check on the 600 MiB deep-war prefix: chain `bc517771692b2052122fd1f83e6fda1c29c84b49c536cf058d8935997c1c63f7`, 198 hours, byte-identical to reference. Prefix regenerated (`head -c 629145600`) and first re-validated against the reference hash with MASTER's binary, so the cut itself is proven before gating the lane. |
| gate-OFF byte identity | by construction (probe record already perf-gate-only; no stream-path edits) + gate-OFF inertness test |
| adversarial review (independent of implementer) | Codex 1R: **REFUTED** → one fix round applied (see §1R below) → re-review pending. (The interim Opus review was killed before reporting when its authorization was revoked.) |

## What this does NOT deliver

Numbers. The instrument's real output needs one **live perf-gated capture segment** (agenda
item 1). Until then the acquire/process-split fork (queue doc, Job 2) stays open. The
fixture-path tests prove the plumbing, not the split.

## 1R Codex adversarial review — REFUTED (2026-07-10), one fix round applied

The Codex gate refuted three claims. All three were legitimate; fixes below. (An earlier Opus
review of this diff was killed mid-run when the user revoked Opus authorization — it never
reported.)

1. **Nested-shard blindspot (MEASUREMENT-LIE) — DISCLOSED, not code-fixed.** The RcDomainScope
   tag is thread-local and does not propagate to nested shard threads (pre-existing, documented
   at the tag definition — it equally affects the existing rc_d{id}_reads counts). For sharded
   domains (country bundle 200, and 20/23/26/44 per the review) child-thread guarded time lands
   GLOBAL-only, so `lane_wall - guarded(d)` OVERSTATES materialise there. Consequences:
   - the per-domain split is valid only for non-sharded domains;
   - frame-level guarded totals are correct regardless (untagged counts still reach the global
     sink via the Drop-flush);
   - `scripts/three_phase_report.py` marks these ids and refuses to print a materialise number
     for them. Tag propagation into spawned shards is a possible follow-up lane if the
     non-sharded numbers turn out to decide nothing.
2. **Phase-lock sampling bias (MEASUREMENT-LIE) — FIXED.** Frame workers get fresh TLS every
   frame, so an unseeded counter made every worker sample reads 64, 128, ... of its traversal —
   structured traversals could phase-lock onto the same reads every frame, and ratio scaling
   corrects quantity, not representativeness. Fix: lazy golden-ratio phase seed per thread
   (`RC_SPL_PHASE.fetch_add(0x9E37_79B9_7F4A_7C15) | 1` on first probe). Phases now differ
   across threads and frames; the estimator is unbiased in expectation across frames. Residual
   (disclosed): within one thread-frame the stride is still deterministic, so period-64
   structure in a single traversal can still correlate inside that frame.
3. **Under-specified test oracles (CORRECTNESS) — FIXED.** Added: an exact stride-boundary test
   (phase 126: read #1 must not sample, read #2 must — kills stride-63 and phase-shifted-64
   implementations the aggregate count admits); per-call drains in the scan test (a dropped
   call now fails instead of hiding under an aggregate >0); a fresh-thread seeding test
   (unseeded phase-0 start fails).

Claim revision: the earlier "thread-start phase offsets cannot bias the estimator" was too
strong and is retracted in favor of the mechanism + residual caveat above.

Fix-round gates re-run (2026-07-10): full suite `--no-fail-fast` **1219 passed / 0 failed**
(the known flake happened to pass this run; 1217 prior + 2 new tests = 1219), G1 chain hash
**exact** on the 600 MiB deep-war prefix. Awaiting 1R2 re-review before merge.

## 1R2 re-review — REFUTED again (2026-07-10); fix-round cap reached, orchestrator fixed directly

Six findings; adjudication and disposition:

1. **Estimator script absent from the lane diff** — TRUE as seen from the lane worktree: the
   script lives on MASTER (`extractor/native/scripts/three_phase_report.py`, committed there
   because analysis tooling followed the burst_corr.py precedent). The lane merges into that
   master, so the reference resolves post-merge. Process note recorded: a lane report should
   not cite artifacts outside its own branch without saying where they live.
2. **"Valid for non-sharded domains" was FALSE as stated** — per-domain attribution exists
   only where a reader opens an `RcDomainScope`; unscoped domains (e.g. d22 map_control) do
   guarded reads that land global-only, and the old wording implied their absence meant
   validity. CORRECTED here and in the estimator header: missing row = unattributed, not
   free; scoped+sharded rows are main-thread floors; scoped+unsharded rows are the only full
   per-domain splits. (Codex verified the sharded-scope list 200/20/23/26/44 complete.)
3. **`| 1` parity bias — REAL, FIXED.** Odd-only seeds meant only odd-numbered reads of a
   traversal could ever be sampled (alternating scalar/bulk patterns would be systematically
   misseen). Seeds are now taken as-is (all 64 phases; zero draw falls back to the constant;
   the "sentinel unreachable" comment was false and is corrected — a wrapped counter simply
   re-seeds). Test upgraded: two fresh threads must draw DISTINCT phases (kills constant-seed
   implementations). Main-thread `spl_ctr` persists across frames by design — its phase
   drifts with total read count rather than re-randomizing; disclosed.
4. **Frame-level totals claim over-broad — SCOPED.** Frame-scoped workers Drop-flush before
   the frame drain (Codex verified) and the main thread flushes explicitly; but PERSISTENT
   threads (heap-sweep loop) and detached d53/d60 validation readers hold TLS until exit, so
   their counts land late or in a later frame — a PRE-EXISTING property of the counter
   machinery that now also applies to guarded time. CORRECTION (1R3): those threads are NOT off in the intended
   measurement config — d53/d60 are production-profile domains and their readers start the
   persistent heap sweep + detached validators, so perf-gated captures HAVE late-flushing
   threads. Frame-level totals in real captures are approximate to the extent of those
   threads' guarded share (undercounted per-frame, appearing at thread exit/shutdown).
5. **Gate-ON self-cost MEASURED (was asserted):** `rc_read_probe`+end = **1.4 ns/read** vs
   pre-lane `rc_bump` 0.3 ns/read ⇒ **delta ≈ 1.1 ns/read ≈ 3.4 ms/frame at 3M reads**,
   measured by `rc_probe_self_cost_measure` (ignored test, run with `--ignored --nocapture`).
   This tax sits in lane_wall but NOT in sampled guarded time, so derived materialise is
   overstated by up to ~3.4 ms/frame across all lanes combined on gate-ON runs — subtract it
   (or re-measure on the capture rig) when reading the split. Production (gate OFF) pays none
   of it.
6. **Scan-probe integration untested — FIXED**: `read_bytes_scan_integrates_scan_probe`
   drives the REAL `read_bytes_scan` over live own-process memory and asserts accumulation —
   deleting the probe wiring inside the function now fails a test. `eventually()` retains its
   disclosed tradeoff (retries mask intermittence that global RC_ENABLED contention causes by
   design; the alternative is a flaky suite).

## 1R3 final gate — REFUTED on editorial residue; mechanism CLEARED (2026-07-10)

1R3's self-refutation explicitly upheld the seed algorithm (all 64 phases, sound zero
fallback), the scan-probe integration (deleting the wiring fails the test), the estimator
location disposition, and the benchmark; it ran the sampling/scan/gate-OFF tests green in the
release binary. Its four findings were doc/test-polish, all fixed in this tree: (1) the
"background threads off by default" disposition was FALSE — corrected above (d53/d60
production readers start them; frame totals in real captures are approximate to that
extent); (2) the distinct-phase test now compares low-6-bit PHASES, not whole counters;
(3) the disproven "cost does not grow" text in report+comment replaced with the measured
+1.1–1.6 ns/read; (4) gate-OFF load-count and read_ab_probe routing wording corrected.
Orchestrator adjudication: with the mechanism cleared by the gate itself and every remaining
finding editorial and fixed, the lane MERGES; the fix-round cap forbids another full cycle
and no finding alleges data loss or measurement corruption in the final state.
