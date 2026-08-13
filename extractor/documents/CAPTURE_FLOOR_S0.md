# Capture floor, stage S0: the loss ground truth

Task #132. Derived entirely from committed capture manifests — **no capture was
taken for this document and no production code was changed.**

Primary evidence: `extractor/native/runs/live_unlock/t50_pricing/` (landed at
`746e474b`). Every table below is reproduced by

```
python extractor/native/runs/he_s0_ground_truth/s0_ground_truth.py \
  > extractor/native/runs/he_s0_ground_truth/GROUND_TRUTH.txt
```

which exits non-zero if any identity fails. The committed output is
`extractor/native/runs/he_s0_ground_truth/GROUND_TRUTH.txt`; section numbers
below are that file's.

---

## 0. The deciding numbers, first

| | |
|---|---|
| **The hour is over before the first bracket closes.** | The median boundary-witnessing frame had **0 of up to 68 brackets closed** when its hour ended. A single measured distribution, not a combination. |
| **The build is 1.53 hours long on its own.** | Mean build (`fanout_origin.avg` + `worker_span`/frame) = **104 194 µs per extracted frame** against a 68 000 µs free-run hour. A genuine per-frame mean; the conclusion needs no marginal-sum arithmetic. |
| **The sweep is one domain.** | d81 `rail_replay_state_live` holds the critical lane **61.89 ms of the 62.57 ms mean critical path — 98.91 %**. The other 67 domains sum to 0.67 ms/frame. |
| **The serial prefix is one domain's prewarm.** | `fanout_origin` p50 35.5 ms, of which the d47 prewarm is **96.1 %**. It runs before any bracket opens, so no in-bracket change can reach it. |
| **Eviction is evenly spread, not bursty.** | 346 evicted hours form **346 runs of length 1** on the labelled axis — not one adjacent pair in 805 hours. On the **raw** axis, where clock-skipped hours also lose data, runs reach length 3. Both are reported; neither may be quoted alone. This bounds *displacement*, not the loop period — see §2.2. |
| **The phase lock is not degrading.** | It is **disarmed by its own guard**: 416 of 454 starts took `SweepExceedsHour`. Lateness is a symptom of sweep length, not an independent lever. |

**The loss, on the denominator that counts every lost hour** (§0). The
897-hour axis is `[first_game_hour_raw, last_game_hour_raw]`; it equals
805 labelled + 92 clock-skipped hours, and that identity is asserted by the
script.

| outcome | hours | share of the 897-hour axis |
|---|---:|---:|
| built a frame | 459 | 51.17 % |
| evicted to backpressure | 346 | 38.57 % |
| skipped by the clock (never labelled) | 92 | 10.26 % |
| **published d47 data** | **60** | **6.69 %** |

Quoting d47 delivery as 60/805 = 7.45 % understates the loss by omitting the
92 skipped hours. Both bases appear below and every use names which it is.

---

## 1. The lateness table (§1)

### 1.1 The hour

Four committed estimates. The SPAN terms are disqualified by t50's own
`CLOCK_BASELINE_V1.json` `problems` string (unbracketed pause exposure), so the
pause-safe pair is used everywhere:

| estimate | ms/hour | basis |
|---|---:|---|
| **free_run (chosen)** | **68.000** | 9 hours over a 612 ms bracket |
| with_collector | 71.625 | 8 hours over a 573 ms bracket |
| free_run_span | 43.578 | 1946 hours — **not pause-safe** |
| with_collector_span | 66.813 | 705 hours — **not pause-safe** |
| window/axis cross-check | 66.890 | `duration_ms` 60000 over 897 axis hours |

`H_free = 68.0 ms` is used in every projection. That is the **conservative**
choice: the hour lengthens under collector drag and can only shrink back
toward the free-run value as the build gets cheaper, so a projection already
priced at the free-run hour cannot be flattered by the improvement it projects.

**Caveat on the record:** n = 9 and n = 8 hours. These are the narrowest inputs
in the document and every projection inherits their width. The independent
window/axis cross-check lands within 1.6 % of `H_free`, which is why the
brackets are trusted despite n.

### 1.2 Where the frame sits inside its hour

Basis: 459 extracted frames, except `frame_time_us` (805 labelled hours) and
`at_us` (435 boundary-witnessing frames). All percentiles are exact order
statistics — `recorder_host::timing_summary_json` sorts the complete sample
vector (`main.rs:24168`). The one bucketed distribution in the manifest,
`sink_write_us`, is labelled as such and is not used.

| term (µs) | p50 | avg | p95 | max | n |
|---|---:|---:|---:|---:|---:|
| `hour_start_lateness.age_us` | 20 503 | 26 987 | 85 135 | 114 210 | 459 |
| `prewarm_time_us` (serial, pre-fan-out) | 34 141 | 34 416 | 39 094 | 43 845 | 459 |
| `fanout_origin_us` (serial prefix) | 35 524 | 35 791 | 40 464 | 45 308 | 459 |
| `crit_wall_us` (widest gated lane) | 62 400 | 62 571 | 76 734 | 91 609 | 459 |
| `worker_span_us` (whole fan-out) | — | 68 403 | — | — | 459 |
| `outside_build_us.total` | 556 | 6 879 | 40 950 | 131 728 | 459 |
| `hour_boundary_position.at_us` | 37 965 | 50 138 | 100 102 | 134 091 | 435 |
| `frame_time_us` | 105 115 | 65 252 | 125 923 | 148 824 | 805 |

**Phase sketch — NOT a measured frame.** Every line is a sum of *marginal*
statistics from separate distributions, so it describes no frame this capture
observed (the p50 of a sum is not the sum of p50s). It is shown because the
shape is the point, and it is load-bearing for nothing: the defensible
aggregate is the **mean build, 104 194 µs per extracted frame**, which already
exceeds the hour by 1.53×.

```
p50  build origin        20 503 µs into the hour   ( 30.2 % of H_free)
     first bracket OPENS 56 027 µs into the hour   ( 82.4 %)
     widest lane CLOSES 118 427 µs into the hour   (174.2 % = 1.74 hours)
```

As a fraction of the 68.0 ms free-run hour: lateness **30.2 %**, serial prefix
**52.2 %**, fan-out **91.8 %**.

### 1.3 Which term kills each frame

| | frames | share of 459 |
|---|---:|---:|
| build never saw a later hour | 24 | 5.23 % |
| build saw the next hour arrive | 435 | 94.77 % |

Brackets already closed when the boundary arrived: **p50 0**, avg 15.1, p95 55,
max 56. The boundary lands at `at_us` p50 37 965 µs against a `fanout_origin`
p50 of 35 524 µs — **+2 441 µs**, i.e. essentially at the moment the first
bracket opens. (Both share the frame build origin, `main.rs:24045`, so the
difference of the two *distributions* is exact; it is not a per-frame joint
measurement.)

Scored per domain-hour for d47, whose identities close exactly:

| gap reason | domain-hours | the term it names |
|---|---:|---|
| backpressure | 346 | **period** (no frame built at all) |
| hour_label_mismatch | 39 | **lateness** (bracket wholly outside its label) |
| torn | 360 | **duration** (boundary crossed the open bracket) |
| data | 60 | published |

Duration outweighs lateness **9.2 to 1**. And the decisive reading, stated on
the **mean** so it is a real per-frame quantity rather than a sum of marginal
medians: **even at zero lateness** the mean build is 104 194 µs against a
68 000 µs hour — **1.53 hours**. Lateness is not what makes the frame not fit.

---

## 2. Backpressure decomposition (§2)

### 2.1 The counting trap, first

Backpressure gap **records** carry `span_hours > 1`, and those spans swallow the
clock-skipped hours next to the eviction:

| span_hours | records |
|---:|---:|
| 1 | 256 |
| 2 | 86 |
| 3 | 3 |
| 10 | 1 |

The 346 records **cover 447 hours** but contain exactly **one labelled hour
each** (asserted by the script). Counting covered hours would overstate the
eviction by 101 hours, 29.2 %. Every figure here is on the reconstructed
labelled axis.

### 2.2 Structure: evenly spread, and exactly so

- 346 evicted labelled hours = **346 runs of length 1**. Mean run 1.000,
  longest 1 (asserted by the script).
- Distance to the next eviction on the labelled axis: +2 (75.07 %), +3
  (19.42 %), +4 (2.90 %), +5 (2.61 %). **Adjacent evictions: zero.**

**What this bounds: displacement, not period.** With request capacity 1
(`PRODUCTION_DOMAIN_QUEUE_CAPACITY_V1 = 1`; the loop skips intervening hours
rather than building a stale one, `attach_log.rs:3855`), a frame that occupies
the loop while k *labelled* requests arrive displaces `max(0, k−1)` of them,
consecutively, because the last arrival becomes the next build's label. So run
length r means that frame displaced r labelled requests, and longest run 1 says
exactly: **no frame displaced two consecutive labelled requests.** That is the
whole result.

**It does not bound the loop period P**, and two earlier drafts claimed it did —
first `P ≤ 2H`, then `P < 3H` (Codex rounds 1 and 2, finding 2). **Both are
retracted.** The labelled axis counts *admitted observations*, not hour
boundaries: on a multi-hour advance only the observed hour is stamped and the
intervening ones become `skipped_hour` (`attach_log.rs:43804`), so a skipped
boundary creates no labelled request and therefore no eviction to lengthen a
run. A long period can hide inside skipped hours and still show a run of 1.

**NEGATIVE 10:** the per-frame period distribution is not derivable, so neither
*"no stall tail exists"* nor any numeric bound on P is claimed here. *Closed by*
the packaged-wire run of NEGATIVE 1. §3 never uses a run-length bound — it
derives the *mean* period from the hour axis and carries the non-build part as a
named overhead. (P is the loop period, not the build: `attach_log.rs:43274`
records that a phase-lock hold can itself cost an hour to backpressure.)

**What survives, and it is what the lever ranking needs:** eviction is
*distributed*, not concentrated. 346 evictions in 805 labelled hours with no two
adjacent, and next-eviction distance concentrated at +2 and +3, is a picture
with no burst structure to chase. The loss is treated as a **mean-period**
problem on that basis (`perf-measure-decay-not-rate`,
`loss-tracks-game-speed`), not on a bound the evidence cannot support.

**On the raw hour axis** — the one a reader of the recording experiences, where
an hour is lost whether it was evicted or clock-skipped — the runs are longer:

| run len | runs | hours | share |
|---:|---:|---:|---:|
| 1 | 257 | 257 | 58.68 % |
| 2 | 86 | 172 | 39.27 % |
| 3 | 3 | 9 | 2.05 % |

Mean run 1.266, longest 3. So "every run is length 1" is a statement about
**eviction on the labelled axis** and does not survive on the raw axis. Both are
published so neither can be quoted without the other.

### 2.3 Does eviction cluster behind specific domains?

**No, and the negative is structural rather than statistical.** The
`backpressure` class in the `gaps` table is a SHARED class (`domains: []`) —
the frame was never built, so no domain read is implicated by construction.
Re-expressed per domain, **all 68 scheduled domains lost exactly 346
domain-hours**, the signature of whole-frame eviction (cf. t19, where all 68
lost exactly 198).

**NEGATIVE 1 — the duration/eviction correlation is UNDERIVABLE** and is the
largest hole in this document. The manifest publishes `frame_time_us` and every
`lane_wall` term only as sorted-vector summaries; no per-frame series and no
frame→hour key survives, and the run directory ships the portable container,
not the collector wire (`replay_entries.raw_stream_absence_reason`: recorded
without `--package-raw-stream`). *The one measurement that closes it:* capture
with `--package-raw-stream` and run `sweep_span_audit` over the packaged wire —
it already decodes the per-frame lane-wall probe records and every `FRAME_END`
carries its own hour, so the join is a decode, not a model. Constraints: t50's
wire is 1.18 GB decoded, so only the audit report may be committed; and t50
defect **HB-4** (`--keep-spool-after-seal` refused by
`load_verified_replay_authority`) must be cleared first.

### 2.4 What stage 2 would have to do differently

The not-recommended S2 design was *"evict obvious in-bracket compute"*. Two t50
measurements it did not have:

1. **98.91 % of the critical path is one domain.** The other 67 domains hold
   the critical lane for **669 µs/frame** in total. Evicting all of their
   in-bracket compute moves the sweep by under 1 % of an hour.
2. **The serial prefix is 35.5 ms p50, 34.4 % of the whole build, and runs
   before any bracket opens.** In-bracket compute eviction cannot reach it at
   all.

So a stage 2 worth running attacks (a) the serial prefix and (b) the single
critical domain — both **copy-only conversions (#119)**, not compute eviction.
That is why the original S2 was correctly not recommended and why the plan's S4
is the stage that bites.

---

## 3. The floor projection (§3) — every number here is a projection

### 3.0 What the model actually predicts, and the floor that sits under it

Over a wall window T the game advances T/H hours and the loop completes T/S
frames, so the fraction of **elapsed** hours that get no frame is
`max(0, 1 − H/S)`.

**The modelled quantity is the NO-FRAME rate over elapsed hours, not the
`backpressure` gap reason.** The ledger splits t50's 438 frameless hours into
346 `backpressure` (labelled, then evicted) and 92 `skipped_hour` (the clock
observer never labelled it). The model counts hours that got a frame and is
agnostic about which arm the rest fall into.

**The consequence for the floor is new and is not comfortable: driving the
model's rate to 0 requires the skipped arm to vanish too.**

**NEGATIVE 11** — whether the 92 skipped hours fall with build duration is not
derivable. The observer polls at `sample_ms = 1` against a 68 ms hour, so a skip
needs an observer stall of roughly a whole hour — consistent with CPU starvation
by the 12 reader lanes, but not shown by anything committed. **If the skipped
arm does not fall, the loss floor is 10.27 % of elapsed hours (92 of 896) no
matter how short the build gets**, and no build duration in §3.3 reaches <1 %.
On the §0 slot basis the same count reads 10.26 %; the two must not be
interchanged. *The one
measurement that closes it:* an observer poll-gap histogram (largest gap between
consecutive clock reads per hour) as a lane-wall diagnostic unit — a gap near H
*is* the skip, and its correlation with build length answers the question in one
capture. **This gates the entire <1 % target and is the second-cheapest open
item in the document.**

### 3.1 The period, and why it is not taken from the loop wall

The model is the producer's own, quoted from the accumulator built to serve it
(`attach_log.rs:21861`): *"The frame period is what decides how many game hours
a capture keeps — loss = 1 − hour_period/frame_period."*

The capture-loop wall covers attach and teardown, when no hours were being
labelled, so `loop/builds` (141 985 µs) overstates the steady-state period.
Worse, with hour period and sweep period both taken from that one wall,
`1 − H/S` collapses to `1 − frames/labelled` — **an identity, not evidence for
the queue model.** The pause-safe form needs no wall clock:

```
S = H_free × (elapsed hours / builds) = 68 000 × 896/459 = 132 741 µs
```

**Elapsed hours = `game_hours_advanced` = 896**, the count of hour
*transitions*. It is one fewer than the 897 axis *slots*, because the first and
last slots are partial, and it is the transitions that measure wall time (Codex
round 2, finding 3). The §0 outcome table uses slots, which is right there and
wrong here; the two differ by 0.11 %.

| term | µs |
|---|---:|
| S (H_free × elapsed/builds) | 132 741 |
| build (`fanout_origin.avg` + `worker_span`/frame) | **104 194** |
| overhead (S − build) | 28 547 |
| — of which `buildtail_previous_frame` | 7 913 |
| — of which `outside_build.total` | 6 879 |
| — of which phase-lock hold idle (upper bound) | 5 360 |
| — uninstrumented remainder | 13 755 |

**The eviction-proportional part of the overhead.** 346 labelled hours produced
a `FRAME_END` but no build; the frame_time totals leave ≤ 13 592 µs per evicted
hour (an inference, and an upper bound), which is 10 246 µs amortised per
build. That term goes to zero *with* the eviction rate, so carrying it into a
zero-loss projection would be double-charging.

**NEGATIVE 2** — the overhead cannot be decomposed further from committed
evidence. *Closed by the same packaged-wire capture as NEGATIVE 1:* consecutive
`FRAME_END` records give the true per-frame period as a series instead of a
subtraction of means.

Every projection is therefore given twice. Overhead = 0 is **not** offered,
because the loop demonstrably has non-build time:

- **pessimistic** S = build + 28 547 µs (the whole overhead survives)
- **optimistic** S = build + 18 301 µs (only the eviction-proportional part
  vanishes)

### 3.2 Projected no-frame rate vs build duration

> **Read the first row as an identity, not a validation.** S was *defined* as
> `H_free × elapsed/builds`, so the pessimistic cell at today's build reproduces
> `1 − builds/elapsed` = 48.77 %, the measured no-frame rate over elapsed hours.
> That agreement is arithmetic. (On the 897-slot basis of §0 the same rate reads
> 48.83 % — a 0.11 % difference from the single partial endpoint slot.) Only the
> rows below today's build carry predictive content.

| build µs | pess @H_free | pess @H_coll | opt @H_free | opt @H_coll |
|---:|---:|---:|---:|---:|
| **104 194 (today)** | 48.77 % | 46.04 % | 44.49 % | 41.53 % |
| 80 000 | 37.35 % | 34.01 % | 30.82 % | 27.14 % |
| 70 000 | 31.00 % | 27.32 % | 22.99 % | 18.89 % |
| 60 000 | 23.20 % | 19.11 % | 13.16 % | 8.53 % |
| 50 000 | 13.43 % | 8.81 % | 0.44 % | 0.00 % |
| 45 000 | 7.54 % | 2.61 % | 0.00 % | 0.00 % |
| 40 000 | 0.80 % | 0.00 % | 0.00 % | 0.00 % |
| **36 600** | **0.00 %** | 0.00 % | **0.00 %** | 0.00 % |

### 3.3 What <1 % and <0.1 % require

The target is **strict** (`< t`), so the bound on S is strict: `S < H/(1−t)`.
At whole-microsecond precision the largest admissible S is `ceil(H/(1−t)) − 1`,
and every build limit is **floored**, never rounded to nearest (Codex round 1,
finding 7).

| target | at | S must be ≤ | build ≤ (pess) | build ≤ (opt) |
|---|---|---:|---:|---:|
| <1 % | H_free 68 000 | 68 686 | **40 139** | **50 384** |
| <1 % | H_coll 71 625 | 72 348 | 43 801 | 54 046 |
| <0.1 % | H_free 68 000 | 68 068 | 39 521 | 49 766 |
| <0.1 % | H_coll 71 625 | 71 696 | 43 149 | 53 394 |

Against today's 104 194 µs build that is **2.60×** (pessimistic) or **2.07×**
(optimistic) for <1 % at the free-run hour. <1 % and <0.1 % are within 2 % of
each other in required duration — once the sweep fits inside the hour, the
no-frame arm collapses fast. **The hard target is <1 %; <0.1 % is nearly free
after it** — subject entirely to the §3.0 skipped-arm floor, which no build cut
addresses.

### 3.4 The torn arm does not track the no-frame arm

The no-frame rate depends only on S vs H. Torn depends additionally on the lateness
distribution, and t50's phase lock is disarmed (416 of 454 starts took
`SweepExceedsHour`, which fires on `half_yield ≥ period`, where
`half_yield = fanout_origin + crit_wall/2`, `attach_log.rs:43398/43536`). So the
projection has **one discontinuity**: below `half_yield < H` the lock re-arms,
lateness collapses toward the hold slack, and the torn arm falls with it.

**The asymmetry the S0 plan did not price:** the serial prefix is worth *twice*
its own length to the phase lock, because it enters `half_yield` at full weight
while the fan-out enters at half weight.

**NEGATIVE 3** — the torn *rate* below the discontinuity cannot be projected
from t50, which contains no frames in that regime. *Closed by:* the first
capture taken after a conversion that puts `half_yield` below H, compared
against §1's table.

**NEGATIVE 9** — the guard's own inputs (a running `half_yield` median and a
running observed-period median) are on no wire, so §3E's threshold arithmetic
is **indicative, not verifiable**. §5 shows the reconstructed ratio does not
predict the guard's counter across captures. *Closed by:* emitting those two
running medians as lane-wall diagnostic units beside the existing
`start_reasons` counters — a two-field producer change, no capture-protocol
change. **Cheapest open item in this document.**

### 3.5 The 4c/16GB ship floor — weakest basis here

t50 ran unmasked on the dev machine. The only committed 4-core evidence is
**t39_4c** (same-machine affinity mask, `-ReaderConcurrency 4`): masked free-run
70.0 ms/hour, frame p50 130 ms vs 106 ms unpinned.

| dev build µs | 4c build µs (×1.226) | no-frame rate @70 000 µs (pess / opt) |
|---:|---:|---|
| 104 194 | 127 742 | 55.21 % / 52.07 % |
| 50 000 | 61 300 | 22.09 % / 12.06 % |
| 40 000 | 49 040 | 9.78 % / 0.00 % |
| **36 600** | 44 872 | **4.66 % / 0.00 %** |
| 30 000 | 36 780 | 0.00 % / 0.00 % |

The column is the **no-frame rate**, not the `backpressure` gap reason: its
residuals inherit the elapsed-hour calibration, which contains both arms, and
NEGATIVE 11 says the two cannot be separated under projection.

With the same strict-rounding rule as §3.3, <1 % at 4c needs a dev build
≤ **34 388 µs** (pess) / **42 745 µs** (opt).

Three caveats on the record: t39_4c is a same-machine mask with full cache and
RAM, so it is a *lower* bound on a real 4c box; the ×1.226 factor is
cross-session and does not decompose into serial and parallel parts — which is
exactly what §4's conversions change; and t39_4c predates everything t48/t49/t50
priced.

**NEGATIVE 4** — the 4c floor is not derivable at the current tip. *Closed by:*
one capture, t50's protocol with `-CpuAffinityCores 4 -ReaderConcurrency 4`.
That replaces this whole subsection.

---

## 4. Lever ranking (§4)

**The column that decides the order is critical-path holding, not read time.**
A domain that is not on the critical path contributes zero sweep length however
slow its own read is. The two orderings disagree sharply in t50 and *the
disagreement is the finding*: a worst-first order taken from read time alone
would spend its first several lanes on domains worth under 1 µs/frame.

| dom | name | crit µs/frame | share |
|---:|---|---:|---:|
| **81** | **rail_replay_state_live** | **61 889.3** | **0.98911** |
| 26 | division_equipment | 112.3 | 0.00179 |
| 42 | air_combat_losses_live | 107.3 | 0.00171 |
| 27 | army_orders | 92.3 | 0.00148 |
| 47 | characters_leaders_cabinet_live | 41.9 | 0.00067 |
| … | (15 more, all < 70 µs/frame) | | |

The full 68-domain table with surviving-read p50/max, data records and gap
counts is §4C.

### The span after the top domain is cut — two terms, and not a range

12 reader lanes; total lane busy 362 218 µs/frame ⇒ a perfectly balanced
makespan of 30 185 µs, against an actual fan-out span of 68 403 µs. **The span
is set by the longest single job, not the total work.**

Two computable terms, and **neither is the textbook
`max(longest_job, total/machines)` bound** — that needs a per-frame longest
*job*, which requires bundling per-frame `DOMAIN_PERF` read times.
`sweep_span_audit.rs` does exactly that, on the packaged wire this analysis does
not have. An earlier draft called the pair a list-scheduling lower bound; it is
not (Codex round 1, finding 4).

| term | value | standing |
|---|---:|---|
| machines term `(total busy − l0 busy)/12` | 25 207 µs | a genuine **lower bound** on the remaining makespan, under the list-scheduling assumption that the remaining work is divisible |
| next lane's mean wall | 35 224 µs | an **observed occupancy**, not a bound in either direction — a lane wall aggregates several jobs, and work displaced from l0 may migrate rather than disappear |

**These two do not bracket the answer, and an earlier draft used them as if they
did** (Codex round 2, finding 2). The second is explicitly a bound in neither
direction, so it cannot be an interval endpoint; and the machines term removes
*lane l0's* busy time (59 734 µs/frame), which is not the same quantity as d81's
critical-path holding (61 889 µs/frame) — the lane→domain map that would
reconcile them is NEGATIVE 7.

**So §4 does not assume a post-removal span at all.** It solves for the
**break-even span**: the largest fan-out span at which the projected rate still
meets a target. That converts an unknown into a stated condition, which is the
only honest form available here. The two terms above survive only as clearly
labelled *illustrations*.

**NEGATIVE 12** — the achievable post-removal span is not derivable. *Closed
by* `sweep_span_audit`'s `makespan_floor` / `longest_job` columns on the
packaged wire; that tool documents its own bundle collapse as heuristic, so even
it returns a bound rather than an achieved span.

### The ranking

Projected quantity is the **no-frame rate** over elapsed hours (§3.0). The
skipped-arm floor applies on top of every row unless NEGATIVE 11 resolves
favourably, and on this section's **elapsed-hour** basis that floor is
**10.27 %** (92 skipped of 896 elapsed hours) — *not* the 10.26 % slot-basis
figure of §0, which cannot be applied to an elapsed-basis rate (Codex round 3,
finding 5). **Every cell in the last column is a resultant rate**, so
the rows are comparable (Codex round 2, finding 6) — ranks 3 and 5 are not
deltas dressed as rates.

Ranks 1 and 1+2 depend on the **unknown** post-removal fan-out span
(NEGATIVE 12), so they are stated as **break-even conditions** rather than
assumed spans, with two clearly-labelled illustrations.

| rank | lever | projected build µs | resultant no-frame rate (pess / opt) |
|---:|---|---:|---|
| — | today | 104 194 | 48.77 % / 44.49 % |
| **1** | **d81 copy-only** — *illustration*, span = machines term 25 207 | 60 998 | 24.06 % / 14.25 % |
| **1** | **d81 copy-only** — *illustration*, span = next lane wall 35 224 | 71 016 | 31.70 % / 23.87 % |
| **2** | **d47 copy-only (#119, lane HC)** — no span dependency | 69 778 | 30.84 % / 22.80 % |
| **1+2** | **both** — *illustration*, span 25 207 | **26 583** | **0.00 % / 0.00 %** |
| **1+2** | **both** — *illustration*, span 35 224 | **36 600** | **0.00 % / 0.00 %** |
| 3 | every other domain, together | 103 525 | 48.51 % / 44.18 % |
| 4 | bounded retry-until-clean | *raises* the build | torn arm only, unquantified |
| 5 | custody prealloc / cap raise | 104 194 (unchanged) | 48.77 % / 44.49 % — **loss recovered is zero, and that half is measured** |

**The break-even conditions, which is where the real content is:**

Thresholds are **floored**, not rounded to nearest: they bound a span that may
be fractional, and rounding up would admit spans that fail (Codex round 3,
finding 2).

| lever | break-even span for a 0 % rate (pess / opt) |
|---|---|
| rank 1 alone | 3 661 µs / 13 907 µs — exact 3 661.802 / 13 907.542 |
| **ranks 1+2** | **38 077 µs / 48 323 µs** — exact 38 077.503 / 48 323.244 |

The floored value is the largest span *guaranteed* to meet the target; the
target is guaranteed *missed* one microsecond higher (3 662 / 38 078). A
fractional span between the floor and the exact threshold still meets it, so
the floored figure must not be reused as the exact boundary (Codex round 5,
finding 1).

**Rank 1's insufficiency is itself a condition, not a proof** (Codex round 3,
finding 1). NEGATIVE 12 supplies no lower bound on the post-removal span, so
"d81 alone is not enough" is true exactly when that span exceeds
**3 661.802 µs**. What
*is* derivable, and needs no span estimate, is the **ratio of the two
conditions on the same unknown**:

Ratios are taken on the **exact** thresholds, not the floored display values:

| lever | span must be cut to | reduction from today's 68 403 µs |
|---|---:|---:|
| rank 1 alone | 3 661.802 µs | **18.7×** |
| ranks 1+2 | 38 077.503 µs | **1.8×** |

The requirement on rank 1 alone is **10.4× harsher**. That is the derivable
content of "d81 is not sufficient without d47"; the earlier unconditional
phrasing is retracted.

**Rank 1 — d81 `rail_replay_state_live`.** Holds the critical lane 61.89
ms/frame, 98.91 % of the critical path. In-bracket 56.33 ms/frame,
post-bracket 5.55 ms/frame. Its internal phase timers **nest and must not be
summed** — plan 18 940 395 + hub 22 177 676 µs against an in-bracket total of
only 25 857 376 µs (a basis-free proof from totals; `DERIVATIONS.md` line 367
records the same nesting for `d81_plan_us`/`d81_phase_plan_us`). **NEGATIVE 8:**
d81's disjoint in-bracket breakdown is not derivable from the manifest;
*closed by* `sweep_span_audit --domain 81` on the packaged wire. The rank-1 rows
leave the post-conversion copy residue (3.53 MB/invocation charged snapshot)
inside the unknown span, where the break-even condition already governs it —
t50 measures no wall for a copy of that size, and the two illustration values
are **not** a range that could bound it (Codex round 3, finding 4).

**Rank 2 — d47 `characters_leaders_cabinet_live`.** Two contributions, routinely
conflated:

- **(a) the serial prewarm, 34.4 ms/frame before any bracket opens.** Basis:
  459 extracted frames. *This is the term rank 2 prices*, and the rank-2 row is
  therefore independent of §4B's two span terms and of NEGATIVE 7.
- **(b) its own bracket read**, which may or may not become the fan-out's next
  constraint once d81 is cut. The two largest non-d81 surviving-read p50s are
  d47 36 745 µs (n = 60) and d53 35 791 µs (n = 103). Those are
  **surviving-invocation percentiles over tens of reads** and are *not*
  comparable with the 35 224 µs lane wall, which is a mean over all 459 frames
  and may aggregate several jobs — an earlier draft's "within 5 % of the lane
  wall" was a basis mix and is **withdrawn** (Codex round 1, finding 5).
  **NEGATIVE 7:** the lane→domain map is not on the wire, so *nothing* is
  claimed about which domain owns the next constraint. *Closed by* the
  packaged-wire run.

**The combined conclusion, stated as the condition it is:**

> Ranks 1+2 reach a **0 % projected no-frame rate for any post-removal fan-out
> span of at most 38 077.503 µs** (exact), even under the pessimistic overhead.
> Today's span
> is 68 403 µs, of which d81 holds 61 889 µs of critical path, so the condition
> **needs converting d81 to buy at least 30 326 µs of span.**

That is a far weaker requirement than any single span estimate, which is why
this conclusion survives NEGATIVE 12 while the earlier interval-based version
did not. `half_yield` at the break-even span is 20 414 µs — far below H — so
**the phase lock re-arms for free and the torn arm falls with the lateness.**
NEGATIVE 3 is exactly what that cannot quantify, and the §3.0 skipped-arm floor
still applies on top.

Note what is *not* claimed: neither rank is shown to be strictly necessary. Both
are conditions on the same unknown span, and the ranking rests on the 10.4×
ratio between them, not on an insufficiency proof (Codex rounds 3–4, finding 1).

**Rank 3 — everything else.** Combined critical-path holding of the other 67
domains: 669 µs/frame, 1.07 % of the critical path. Worth under 1 µs/frame each
until ranks 1–2 land, **after which the ranking must be re-derived from a new
capture** — the critical path moves and this table cannot predict where. The
worst-first order for that moment is §4C's read-time table.

**Rank 4 — bounded retry-until-clean** (`zero_loss_design_2026-08-11/
CODEX_MEMO.md`). Targets the torn arm only (78.43 % of extracted frames for
d47). **NEGATIVE 5:** t50 publishes no per-range retry histogram, so the
fraction of tears that would clear on retry 1/2/4 is unknown. *Closed by* the
memo's own cheapest experiment (retry policies 0/1/2/4 reporting recovered
domains, extra bytes, added bracket wall). **Added bracket wall feeds straight
back into S**, so this lever can worsen the no-frame arm while improving the
torn arm — it must be priced *after* ranks 1–2, never before.

**Rank 5 — custody preallocation / raising the 512 MiB cap.** Projected loss
recovered at t50: **zero, and that is measured, not argued** —
`d81_plan_custody_refused`, `rs_refused_hours`, `rs_overrun_domain_hours`,
`rs_worker_panic_domain_hours`, `rs_abandoned_hours` and
`rs_shadow_drop_memory_overflows` are all 0. Custody is insurance against a
regime t50 did not enter. Ranked last for **loss**; not thereby ranked last for
**risk**.

**Referenced, not duplicated — the engine hour-commit hook** (being RE'd by
lane HF in parallel). Its ceiling is visible from this table and worth stating
once: it can remove at most the **lateness** term, 20.5 ms p50, and
`CODEX_MEMO.md` already records that it *"solves alignment — not duration"*. On
t50's numbers that places it **below ranks 1 and 2**, because §1.3 shows
lateness is not what makes the frame not fit. It rises above them only after
ranks 1–2 land, when the lock re-arms and polling ambiguity becomes the residual
jitter.

### Phase-lock health: is the lock degrading?

**No — it is disarmed by its own guard, correctly.** Basis: loop iterations
(the block says so itself), which is neither of the frame denominators.

| start reason | count | share of 454 no-hold starts |
|---|---:|---:|
| `sweep_exceeds_hour` | 416 | 91.63 % |
| `boundary_overdue` | 20 | 4.41 % |
| `hour_has_room` | 13 | 2.86 % |
| `unestimated` | 5 | 1.10 % |
| `hold_cap` | 0 | 0.00 % |

The guard's own reasoning, from the source comment: half the brackets do not
close inside a whole hour at *any* phase, so holding would skip hours to buy
nothing. Consequence: lateness is set by whenever the previous sweep ended, and
the 20 503 µs p50 *is* that. **Phase-lock lateness at t50 is a symptom of sweep
length, not an independent lever, and it re-arms for free once the sweep
shortens.** This verdict rests only on t50's own in-session `start_reasons`
counter, never on the cross-session t47 comparison.

---

## 5. Cross-session context (§5) — no verdict rests on this

Five different processes on different world states; `AGENTS.md` voids
cross-session nanosecond comparison. This table shows direction and places the
t47 figure the S0 brief cites.

| capture | labelled | frames | bp % | lateness p50 | fanout p50 | crit p50 | prewarm p50 | locked | sweep>hr |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| t46_shadow | 628 | 374 | 40.45 % | 8 743 | 43 955 | 57 921 | 42 193 | 144 | 125 |
| t47_phase_lock | 656 | 425 | 35.21 % | **1 007** | 44 298 | 52 697 | 42 360 | 169 | 93 |
| t48_authoritative | 504 | 295 | 41.47 % | 32 255 | 42 481 | 120 769 | 40 618 | 49 | 318 |
| t49_pricing | 598 | 334 | 44.15 % | 31 550 | 44 001 | 99 083 | 42 379 | 15 | 329 |
| **t50_pricing** | **805** | **459** | **42.98 %** | **20 503** | **35 524** | **62 400** | **34 141** | **82** | **416** |

An attempt to test the guard mechanism in-session across all five — comparing
each capture's own `half_yield` against its own free-run hour — **fails**, and
that failure is reported as NEGATIVE 9 rather than smoothed over: t50 sits at a
ratio of 0.98 (below threshold) yet takes the guard on 91.63 % of starts, while
t46 sits at 1.30 (above) and takes it on 49.80 %. The reconstructed ratio is not
a sufficient test of the guard, and nothing here rests on it.

---

## 6. Residuals, with owners

| # | negative | closing measurement | owner |
|---|---|---|---|
| 1 | duration/eviction correlation | packaged-wire capture + `sweep_span_audit` | next measurement lane — **blocked on HB-4** |
| 2 | frame-period overhead decomposition | same capture | same |
| 3 | torn rate below the phase-lock discontinuity | first post-conversion capture | the lane landing #119 (HC) |
| 4 | 4c/16GB floor at the current tip | one 4-core capture, t50 protocol | 4-core capture lane |
| 5 | retry-until-clean recovery fraction | retry-policy histogram experiment | **unstaffed** |
| 6 | `rs_*` counters vs the gap ledger disagree (885 vs 805 scheduled; 435 vs 346 evicted) | source read of the two emitters' windows + one wire field | **unstaffed** — no capture needed |
| 7 | lane → domain map | packaged-wire capture | as 1 |
| 8 | d81's disjoint in-bracket breakdown | `sweep_span_audit --domain 81` | as 1 |
| 9 | phase-lock guard's running inputs | two lane-wall diagnostic units | **unstaffed** — cheapest item here |
| 10 | per-frame period distribution | packaged-wire capture | as 1 |
| 11 | do the 92 skipped hours fall with build duration? **A 10.27 % elapsed-basis loss floor rides on this** (§3.0; the §0 slot basis reads 10.26 %) | observer poll-gap histogram, one lane-wall unit | **unstaffed** — gates the whole <1 % target |
| 12 | achievable fan-out span after the top domain is cut | `sweep_span_audit` `makespan_floor`/`longest_job` | as 1 |

Negatives 1, 2, 7, 8, 10 and 12 are all closed by the **same** packaged-wire
capture, which is by a wide margin the highest-value single measurement
remaining for S0. Its prerequisite is t50 defect **HB-4**. NEGATIVE 11 is
second: it needs no capture protocol change and it decides whether the <1 %
target is reachable by build cuts at all.

Nothing in this document is complete while these are open; each names its owner
above.
