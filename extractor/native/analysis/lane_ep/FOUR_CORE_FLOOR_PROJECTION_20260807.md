# The 4-core floor, projected from t32/t35/t36 — and the switch that will falsify it

The capture floor targets a 4-core/8 GB ship machine. Every 4-core number
published so far is DERIVED from measurements taken on a 16-logical-processor
dev machine; none has been measured with four cores. The re-priced floor plan
says so itself: its S6 disposition ends "then run the same measurement on an
actual 4c/8GB machine"
([floor_reprice_reply.txt:98](../../runs/s0_census/t35/floor_reprice_reply.txt)).

This is the arithmetic that should be falsified when that measurement happens,
published in enough detail that it can be. It is a *prediction*, not a finding,
and §5 says plainly which parts of it the evidence cannot support.

Companion change: `extractor/native/scripts/Invoke-HeadlessRawCapture.ps1` gains
`-CpuAffinityCores` / `-CpuAffinityOneThreadPerCore`, which pin the game and the
recorder to the same n physical cores. That measures the core-count half of the
floor and nothing else — see §7.

## 0. What counts as a measurement here

Two distinctions decide most of the numbers below, and getting either wrong
silently inflates the answer.

**CPU time vs permit-occupancy time.** `lane_busy_sum` in the sweep audit is
*not* CPU time. It is the summed elapsed wall duration during which gated
workers held a permit — computed from timestamp differences — and it is
documented as such
([sweep_span_audit.rs:36](../../recorder_host/src/bin/sweep_span_audit.rs)). A
worker blocked on a page fault is still "busy" by that definition. Every claim
about CPU *demand* below therefore comes from process CPU counters
(`CLOCK_BASELINE_V1.json`) and never from `lane_busy_sum`; `lane_busy_sum` is
used only as an occupancy proxy, and is labelled a heuristic wherever it appears.

**`main_wall` is inside the span.** `lw_lmain_wall_us` is the serial MAIN-lane
reads, which run on the sweep thread *inside* `lw_worker_span_us`
([sweep_span_audit.rs:30](../../recorder_host/src/bin/sweep_span_audit.rs)).
Adding it to the span double-counts. BUILD is `prewarm + span + previous-frame
tail`.

## 1. Measured inputs

All from read-only evidence already on disk. Nothing here was re-run.

### 1a. Clock baselines (`CLOCK_BASELINE_V1.json`, `free_run_span` /
`with_collector_span`)

| run | G, engine CPU core-ms per game hour | free-run hour | engine cores | target CPU core-ms/hour with collector | with-collector hour | cores | wall loss |
|---|---:|---:|---:|---:|---:|---:|---:|
| t32 | 270.70 | 56.59 ms | 4.783 | 657.88 | 90.13 ms | 7.299 | **37.2 %** |
| t35 | 267.12 | 54.20 ms | 4.928 | 625.25 | 83.01 ms | 7.532 | **34.7 %** |
| t36 | 259.85 | 55.42 ms | 4.689 | — | — | — | — |

Each row is internally consistent: `cpu_ms_per_game_hour × hours_per_second ÷
1000` reproduces `target_cpu_cores` to three decimals (t35: 267.124 × 18.4497 ÷
1000 = 4.928).

`collector_cost.cpu_ms_per_game_hour` is the *difference* of the two span rows,
so the collector's cost is charged to **hoi4.exe**, not to collector_record.
That is correct and worth stating plainly: the reader lanes live in the injected
DLL and run inside the target process. t35 358.13 core-ms/hour (2.604 cores),
t32 387.18 (2.516 cores).

Three caveats carried forward rather than smoothed over:

* t35's `problems` array warns that its `collector_cost` is unreliable because
  573,950 ms of wall time went unbracketed and a pause inside that gap would be
  charged to the collector. t32 has an empty `problems` array and lands within
  8 % of it. Both are carried as a range; neither is treated as exact.
* The short bracket windows (`free_run` / `with_collector`, 9 and 7 samples)
  give a headline `loss_fraction` of 5.4 % (t32). Those are the flattered
  numbers; the span rows are the max-speed truth, which is the standing ruling
  and matches the corrected 28–36 % drag figure. The spans are used throughout.
* t36 produced no unpaused with-collector witness sample, so it is a free-run
  point only.

### 1b. Sweep audit (`sweep_span_audit.txt`, t35, n=552 frames)

| term | p50 | p90 |
|---|---:|---:|
| prewarm (1 thread, before the span) | 33.62 ms | 37.32 ms |
| worker fan-out span (12 lanes, **includes** `main_wall`) | 46.44 ms | 57.22 ms |
| previous-frame build tail (o12) | 6.63 ms | 9.32 ms |
| **BUILD wall** | **86.68 ms** | **103.86 ms** |
| `lane_busy_sum` (12 lanes, permit occupancy — not CPU) | 375.48 | 422.37 |

The p50 matches `floor_reprice_reply.txt`'s independently stated 86.6 ms, and
the session manifest's `acceptance_telemetry.frame_time_us.p50` of 89.55 ms —
three routes to the same build wall.

`hour_label_audit.txt`: 21,006 of 37,536 bracketed domain-hours mismatched
(**56.0 %**); build-start lateness p50 21.73 ms, mean 31.38 ms, max 111.56 ms;
zero frames where the hour frontier ran ahead of the label.

### 1c. Frame yield and completeness — from the manifest, not extrapolated

`native_session_manifest.json` answers this directly and supersedes any
frames-per-second estimate:

| quantity | value |
|---|---:|
| `game_hours_advanced` | 703 |
| `acceptance_telemetry.clock_skew.mono_elapsed_us` (capture loop) | 71.623 s |
| frames | 698 |
| **frames per game hour** | **0.993** |
| `game_hours_without_data.no_frame_at_all` | 17 (2.4 %) |
| `game_hours_without_data.closed_with_no_expected_domain_data` | 400 |
| `game_hours_without_data.total` / `unaccounted` | 417 (59.3 %) / 0 |

So **t35 retained 40.7 % of its game hours**, essentially all of the loss being
mislabelling rather than missing frames: only 2.4 % of hours got no frame at
all.

The hour period has two defensible anchors and they disagree:

* **101.88 ms** — 703 hours over the 71.623 s capture loop. Whole-run, includes
  ramp.
* **83.01 ms** — the witness `with_collector_span` (501 hours over 41.589 s of
  bracketed segments). Pause-safe, but a subset.

Both are carried. BUILD/hour today is therefore **0.85 (loop anchor) to 1.04
(span anchor)** — i.e. today's build either just fits the hour or just misses
it, depending on which anchor you accept. That ambiguity matters in §5.

### 1d. Host process cost

1.601 GB of stream over 703 hours = 2.17 MiB/hour, so zstd-3 at the measured
1,483 MB/s costs **1.5 core-ms/hour**. With the writer, take 2–20 core-ms/hour.
The fold is post-capture and off this path.

Total CPU demand per game hour, **from CPU counters only**:

```
D = with_collector_span.cpu_ms_per_game_hour + host
t35: 625.25 + 2…20  =  627 … 645
t32: 657.88 + 2…20  =  660 … 678
```

**D = 627–678 core-ms/game-hour** is used below.

## 2. Assumptions, stated so they can be attacked

* **A1 — affinity changes core count and nothing else.** A masked 9800X3D keeps
  its full 96 MB L3 and its full DRAM bandwidth. A real 4c/8 GB box has neither.
  Every 4c number below is therefore a **lower bound** on the real floor
  machine.
* **A2 — SMT capacity multiplier 1.15–1.30.** Four physical cores with both
  siblings enabled deliver 4.6–5.2 dev-core-equivalents of throughput on this
  mixed workload. This is an assumption, not a measurement, and it is the single
  largest lever below. The affinity switch makes it directly measurable.
* **A3 — which four cores does not matter here.** `GetLogicalProcessorInformationEx`
  reports 8 cores, all `EfficiencyClass 0`, all processor group 0, siblings
  adjacent (core k = logical processors 2k, 2k+1). No P/E split, single CCD, no
  cross-CCD hop inside four cores. Cores 0–3 carry more DPC/ISR work than 4–7;
  the applied mask is recorded so that can be checked.
* **A4 — the 8 GB half is not emulated.** See §8.
* **A5 — work conservation.** At saturation the hour period is total CPU demand
  divided by capacity. Ignores lock contention and cache thrash, both of which
  make the real answer *worse*.
* **A6 — the engine's hour is latency-floored at 54.2 ms on these cores.** t35's
  free run wanted 4.93 cores out of 16 logical processors, so it was not
  core-limited; 54.2 ms is what the engine does when cores are free.
* **A7 — the collector's CPU per frame is invariant to core count.** Same reads,
  same bytes, same lanes. On a real 4c box cache pressure would raise it.
* **A8 — frame yield.** D is a *function* of yield: `D(f) = G + f·K_frame +
  host`, and the D above is measured at the observed f = 0.993. Any prediction
  that the yield falls must re-solve D, and §5 does not pretend otherwise.
* **A9 — t35's configuration.** 60 s requested, speed 5, unmodded, 68 bracketed
  domains, `--reader-concurrency 12`, `--read-only-scheduler-queue-cap 1`,
  uncompressed stream. t32 is the second point. Nothing transfers to a different
  roster or lane count.

## 3. Capacity, and why four cores has no headroom at all

Demand with the collector attached is **7.30–7.53 cores** against the dev
machine's **8 physical cores** — 91–94 % of its physical cores, with SMT
supplying the rest. The step from 8 cores to 4 is not a step from "loads of
headroom" to "some"; it is a step from "just fitting" to "not fitting".

The sharpest single fact, and it needs no model at all:

```
engine CPU alone ÷ 4 cores = 267.1 / 4 = 66.8 ms per game hour
engine latency floor                    = 54.2 ms per game hour
```

**At 4c/4t the engine on its own is CPU-bound**, 23 % slower than its own
latency floor, *before the collector runs at all*. At 4c/8t (C = 4.6–5.2) it is
marginal: 50–59 ms against the 54.2 ms floor. And at the projected hour period,
the collector needs `K/H = 358/162.5 = 2.20` cores while the engine wants
`G/H = 1.64` — **3.85 of 4 cores, a 96 % load**. There is nothing left to
absorb a bad frame.

| configuration | capacity, dev-core-equivalents |
|---|---:|
| dev, unmasked (8c/16t) | ~9.2–10.4 (A2) |
| 4 cores, both siblings (4c/8t) | 4.6–5.2 (A2) |
| 4 cores, one thread each (4c/4t) | 4.0 |

## 4. Hour period and wall loss — the solid part

At four cores both arms are capacity-bound (free run 66.8 ms > the 54.2 ms
floor), so the wall loss reduces to a ratio in which capacity cancels:

```
loss = 1 − H_free / H_collector = 1 − G / D
t35: 1 − 267.1 / 627…645 = 57.4 … 58.6 %
t32: 1 − 270.7 / 660…678 = 59.0 … 60.1 %
```

**One exception, and it is the report's own top row:** at C = 5.2 the free run
falls back onto the 54.2 ms latency floor and stops scaling, so the cancellation
no longer holds. That corner gives `1 − 54.2 / 120.6 = 55.0 %`. The loss is
therefore *nearly* SMT-independent, not exactly so.

| # | prediction | value |
|---|---|---|
| **P1** | wall loss on four cores | **55–60 %**, against 34.7–37.2 % measured unmasked |
| **P2** | with-collector hour period | **121–170 ms** (4c/4t 157–170, 4c/8t 121–147), against 83–102 ms today |
| **P3** | free-run hour period | **65.0–67.7 ms** (4c/4t); **54.2–58.9 ms** (4c/8t), i.e. barely moved |

P2's band is the full independent envelope of D = 627–678 against C = 4.0–5.2;
no correlation between them is assumed. P3 is the cheapest test of A2: if 4c/8t
free-runs at 54 ms while 4c/4t returns 67, A2 sits at the top of its range and a
4c/8t mask is a weak proxy for the floor.

## 5. The build wall — and what this evidence cannot decide

Two models. They do not agree, and the disagreement is the honest result.

**Uniform stretch** — every collector phase stretches with the hour. BUILD/hour
is then unchanged *by construction*, so this model cannot answer whether the
build still fits; it only sizes it.

| capacity | hour | BUILD p50 | BUILD p90 |
|---|---:|---:|---:|
| 4.9 (4c/8t) | 128–138 ms | 134–144 ms | 160–173 ms |
| 4.0 (4c/4t) | 157–170 ms | 164–177 ms | 196–212 ms |

**Occupancy share** — the engine takes `G/H` cores first and the 12 lanes share
what is left, with `lane_busy_sum` standing in for core occupancy. This is a
**heuristic**, for the reason in §0: permit occupancy is not CPU time, so
`lane_busy_sum / cores` is not a rigorous bound.

| capacity | hour | cores left for the lanes | BUILD | BUILD/hour |
|---|---:|---:|---:|---:|
| 4.0 | 157–170 ms | 2.30–2.42 | 195–204 ms | **1.15–1.30** |
| 4.9 | 128–138 ms | 2.81–2.97 | 167–174 ms | **1.21–1.36** |
| dev today | 83–102 ms | — | 86.7 ms (measured) | 0.85–1.04 |

**P4 (bounded, not predicted) — BUILD/hour at four cores lands between 1.04 and
1.36, against 0.85–1.04 today.** The upper half of that band is above 1, where
the build no longer fits the hour and, at `queue-cap 1`, whole hours stop being
built. The lower half is where it still fits. **This evidence cannot decide
which**, and that is the single most valuable thing the masked run will settle.

Why the ratio should rise rather than hold: the serial prewarm barely stretches
(one thread always finds a core), while the 12-lane fan-out is compressed from
12 lanes onto ≤4. The build's composition shifts toward the fan-out, which is
the part that generates lateness.

**An attempt to close this by solving the yield fixed point failed, and the
failure is reported rather than hidden.** Iterating `D(f) = G + f·K_frame +
host`, `H = D/C`, `B = prewarm + occupancy/(C − G/H) + tail`, `f = min(1, H/B)`
diverges to f ≈ 0.12 with an absurd 636 ms build. The divergence is structural,
not a bad initial guess: as the yield falls, D falls, H falls, and `G/H` — the
engine's core demand — rises toward 4.0, because at four cores *the engine alone
saturates the machine* (§3). The model then starves the collector completely.
A real scheduler shares by runnable thread count, which nothing in this evidence
measures. So:

**P5 is withdrawn as a prediction and restated as the open question.** Retained
domain-hours today are 40.7 % (§1c). At four cores they must fall — P1 and P4
both point down — but this evidence does not support a number. Producing one is
what the measurement is for.

## 6. Against the re-priced S1–S6 4c column

The re-priced table gives no S0 row for 4c. Working backwards from its S1 4c
BUILD of 80–110 ms (S0 *minus* the prewarm that DR deletes), its implied S0 4c
BUILD is roughly 110–150 ms. This projection says 134–204 ms across both models
of §5. **Same order, this projection more pessimistic.**

The multipliers are comparable: the reprice's 4c column is dev cost × 2.0–3.0
per component ("150–200 MiB A/B movement 3–6 ms dev / 6–12 ms 4c"; "S6 12–22 dev
/ 24–50 4c" = 2.0–2.3×), and this projection's today-code factor is 1.5–2.0×
(uniform stretch) rising to 2.4× (occupancy share).

**What cannot be concluded, and was claimed in an earlier draft of this file:**
that the reprice's multiplier is *purely* core-count-and-contention.
`floor_reprice_reply.txt` never decomposes it into core count versus per-core
speed, cache, and bandwidth, and it describes a "4c/8GB floor"/"cheap 4c
machine" throughout. The valid statement is that the two projections agree on
the multiplier and neither establishes its physical decomposition. It follows
that a masked run tests the *combined* multiplier against one specific physical
cause (core count), which is informative precisely because the decomposition is
unknown — and that a real floor machine, paying a per-core and bandwidth penalty
on top, would be worse by an amount neither document can currently bound.

### DR (S1) is worth less at four cores than on dev

Stated as a **ratio**, because BUILD/hour is what this evidence supports;
converting a ratio to a loss percentage would need the lateness-to-mismatch
response curve, which is measured only over 0–70 ms.

DR deletes the serial prewarm: 33.6 ms of the 86.7 ms build (39 % of the wall)
but only ~9 % of the collector's CPU per frame.

* **dev S1:** BUILD = 46.44 + 6.63 = 53.07 ms against an 83.01 ms hour →
  **0.64**. Adding the measured build-start lateness p50 of 21.7 ms gives
  74.8 ms, still inside the hour. DR is decisive here, and the reprice's 6–18 %
  dev prediction is credible.
* **4c S1**, occupancy share, D = 596.9: BUILD/hour = **1.18** (C = 4.0) and
  **1.19** (C = 4.9) — barely better than S0's 1.15–1.36, and still above 1.

**Deleting a serial phase buys least on the machine where the parallel phase is
the constraint.** If that holds, DR does not move the 4c gate, and the
reprice's own "range batches must be shardable across workers" becomes the
load-bearing item for the floor machine rather than a refinement. The reprice's
S1 4c loss figure of 25–50 % is not contradicted by anything here — it is simply
not supported by this ratio, and is worth re-deriving before S1 is funded on the
strength of it.

## 7. What the affinity switch can and cannot settle

**Can:** A2 (measure it, stop assuming it); P1–P3; P4's band, and which side of
1.0 BUILD/hour lands on; P5, which is currently unanswerable; whether the
failure mode changes from mismatch to whole-hour backpressure.

**Cannot:** per-core speed, cache size, memory bandwidth, physical RAM. A masked
run is a lower bound. **The S6 gate cannot be closed by affinity alone** — the
reprice's own wording ("on the actual floor machine") stands.

Suggested protocol, so the arms are comparable:

1. Both arms in the same run: the harness already takes a free-run baseline
   before attach, and P1/P3 are ratios against it.
2. `-CpuAffinityCores 4` (4c/8t) and `-CpuAffinityCores 4
   -CpuAffinityOneThreadPerCore` (4c/4t). The pair measures A2.
3. An unmasked control on the same save and build, since t32→t35→t36 already
   drift by 4 % in free-run rate.
4. A second arm at `-ReaderConcurrency 4`. Twelve lanes on four cores is heavy
   oversubscription, and "the sweep must fit the hour without more readers" is
   already the standing rule; whether 12 lanes *hurt* at 4 cores is a separate,
   cheap question. Default unchanged.
5. Keep `-KeepSpoolAfterSeal`, or the d250 lane-wall and lateness probes that
   every number in §5 rests on are trimmed away.

Known perturbations, recorded in the receipt rather than waved away: the harness
PowerShell host, its bounded helpers, the 5 s witness and the OS are **not**
masked. A real floor machine pays for them out of the same four cores, so the
measurement is optimistic by whatever they cost.

## 8. Why the 8 GB half is not simulated

Rejected, with reasons:

1. **A job-object memory cap is the wrong physics.** `JOB_OBJECT_LIMIT_JOB_MEMORY`
   / `PROCESS_MEMORY` caps *committed* memory. An 8 GB machine does not refuse
   allocations; it trims working sets and pages. A commit cap converts "slower"
   into "allocation failure": the game OOMs, and the plan's 300–400 MiB transient
   raw custody fails closed as backpressure for a reason no 8 GB machine would
   produce — turning the one behaviour the floor plan wants to observe into an
   artefact of the instrument.
2. **It contradicts the harness's own published contract.** Assigning the game to
   a job needs `PROCESS_SET_QUOTA | PROCESS_TERMINATE` on the client.
   `passive_capture_policy.json` publishes
   `graded_observer_access: PROCESS_QUERY_INFORMATION|PROCESS_VM_READ`, and the
   recorder publishes `target_quota_right: false` and
   `target_termination_right: false` in `TARGET_LEASE_V1.json`. Taking those
   rights would make two receipts in the same run directory false.
3. **It re-creates the condition the WMI launch exists to avoid.** The game is
   started through `Win32_Process.Create` specifically so it inherits no job.
   The current lease is a named mutex and deliberately not job membership —
   `RetainedTargetLease` in `recorder_host/src/main.rs` says so and its journal
   records `target_job_assignment: false` — so nothing in today's recorder
   *tests* job membership. That is not a licence: nested-job limits combine as
   an intersection over the chain, limit-violation notifications go to the
   outermost port, and a prior capture was already lost to a job-object
   interaction. Injection is not the place to experiment.
4. **`SetProcessWorkingSetSizeEx` + `QUOTA_LIMITS_HARDWS_MAX_ENABLE` is closer
   but still wrong.** It forces trimming rather than failure, but it is
   per-process, does not touch the file cache, and on a 62 GB machine behind an
   NVMe page file reproduces neither the paging latency nor the cache scarcity
   of a small box. It would also perturb the collector's own mapped spool.
5. **The honest way is machine-level.** `bcdedit /set {current} removememory`
   (or `truncatememory`) plus a reboot actually reduces the physical RAM the OS
   sees; then run the same masked capture. That is an owner decision — it
   affects the whole machine and needs a reboot to set and another to undo — and
   it is out of a capture harness's scope.

The receipt records `memory_floor_simulated: false` and carries these reasons,
so a masked run cannot later be mistaken for a floor run.

## 9. Falsification checklist

Every one of these is a single number from a masked run:

| # | claim | falsified by |
|---|---|---|
| P1 | wall loss 55–60 % | `CLOCK_BASELINE_V1.json` `loss_fraction` (span terms) outside 0.50–0.65 |
| P2 | with-collector hour 121–170 ms | `with_collector_span.hours_per_second` outside 5.9–8.3 |
| P3 | free-run hour 65.0–67.7 ms (4c/4t), 54.2–58.9 ms (4c/8t) | `free_run_span.hours_per_second`; the two arms not differing by ~1.2× |
| P4 | BUILD/hour in 1.04–1.36 | `sweep_span_audit.txt` `prewarm + span + o12` p50 ÷ hour period outside that band |
| P5 | *(open)* retained domain-hours | `game_hours_without_data.total` ÷ `game_hours_advanced` — this is the number to produce, not to check |
| A2 | SMT multiplier 1.15–1.30 | 4c/8t hour ÷ 4c/4t hour outside 0.77–0.87 |
| §3 | the engine alone is CPU-bound at 4c/4t | a 4c/4t free-run hour at or under 54.2 ms |
| §6 | DR does not restore fit at four cores | an S1 4c BUILD/hour under 1.0 |

If P4 lands above 1.0, the 4-core gate is failed by today's code and the
sharding requirement is the item to fund, not DR.

---

**Review history.** Codex thread `019fdc22-13b0-7c93-a8fc-f71ae498a852` found
seven arithmetic defects in the first draft of this file: a double-counted
`main_wall` in BUILD, `lane_busy_sum` treated as CPU time, an extrapolated
frame-yield figure where the manifest had the measured one, a fixed-D model
inconsistent with its own predicted yield, an over-broad `1 − G/D` claim, an
unsupported decomposition of the reprice's 4c multiplier, and an S1 calculation
that switched demand basis and conflated wall loss with mismatch loss. All seven
are corrected above; two of them (the yield fixed point and P5) are corrected by
withdrawing a claim rather than by producing a better number.
