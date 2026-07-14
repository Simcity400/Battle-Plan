# Burst-correlation study — do WE cause the stalls? (2026-07-10, orchestrator, offline)

Corpus: the retained deep-war stream (read in place, header-only scan), 1421 `frame_end`
records, 1420 usable transitions, 370 skipped hours. Script:
`scratchpad/burst_corr3.py` (rounds 1–2 were void, see "Method error" below).

## Method error, caught and corrected (record it, don't repeat it)

The `FRAME_END` payload is **`[0:8] elapsed_wall_ms (u64)`, then `[8:16] frame_time_us (u64)`**
(`collector/src/lib.rs:41189`). My first two attempts read offset 0 as `frame_time_us` and got a
monotonically increasing counter. The tell was `autocorr(frame_time, lag 1) = +1.0000` and
"frame times" of 0.1 ms. **Every number from those rounds was meaningless**, including a
confident-looking `corr = +0.9302`.

Round 3 validates against the manifest before interpreting anything: reconstructed frame_time
mean **151.5 ms**, p95 **264.8** (manifest 263.6), max **477.2** (manifest 477.2). Layout confirmed.

*Lesson: a sanity cross-check against an independent source is what separates a result from a
plausible artifact. An autocorrelation of exactly 1.0 is not a finding, it is a bug.*

## Headline: the hypothesis that we cause the bursts is REFUTED

Using wall-clock (`elapsed_wall_ms`), the game's pace is now measurable **independently** of our
sweep cost — `interval = wall_delta / gap` (ms of real time per game-hour).

| test | value | reading |
|---|---:|---|
| `corr(prev frame ms, interval of next cycle)` | **+0.4364** | after a **heavy** frame the game runs **slower**, not faster |
| `corr(cur frame ms, interval of same cycle)` | +0.3799 | our cost and the game's per-hour cost **rise together** |
| `corr(gap, prev frame ms)` | +0.0993 | our cost barely predicts skips |
| `corr(gap, cur frame ms)` | +0.0278 | — |

H1 predicted a **negative** `corr(prev ft, next interval)` (we stall the game, it then bursts to
catch up). The measured sign is **positive and moderately strong**. The pattern is instead
**common cause**: heavy world workload makes *both* the game's hour slower *and* our sweep slower.
Skips cluster where the game is **fast**, not where we are slow — skipped cycles ran at a mean
interval of **101.1 ms** vs **211.0 ms** for clean cycles.

## Corrected pace numbers (the old ones came from a circular estimator)

The previously-recorded "min inter-hour interval 28.2 ms" was `min(frame_time/gap)` — a *lower
bound* on the true interval, since our sweep is only part of the cycle. Measured from wall clock:

- **min 33.5 ms**, p1 44.3, p5 71.0, p50 176.0, mean 186.0 ms/game-hour.
- Our sweep is **75.5%** of the mean cycle (`frame_time / wall_delta`), so shrinking the sweep
  shrinks the cycle almost one-for-one.

The world is still bursty (min 33.5 ms against a 186 ms mean). That conclusion survives.

## The acquisition-budget curve (the actionable result)

Holding the game's observed pace fixed, if the whole per-hour cycle cost **D** ms:

| D (ms) | hours skipped | skip rate |
|---:|---:|---:|
| ≤28 | 0 | **0.0%** |
| 40 | 1 | 0.1% |
| 60 | 15 | 1.1% |
| 100 | 109 | 7.1% |
| 151 | 344 | 19.5% |

Model fidelity: at the observed D≈151 ms it predicts 344 skips against **370 actual** (7% under).

**This reframes the target.** Zero-skip does not require the "unreachable ≤25 ms" of the old
analysis; a **~30 ms cycle** gets there, and even **40 ms costs 0.1%**. From today's 151 ms that is
a **~3.8× cut** — and the prefetch lane measured d53 sitting ~3× above its pure memory-latency
floor. The target is plausibly reachable by making acquisition cheaper, without any change-driven
read scheme.

## The confound I cannot resolve offline (do not skip this)

The budget curve assumes the game's pace is **exogenous** — that shrinking our sweep does not make
the game run faster. The `+0.4364` correlation is evidence it is **not** fully exogenous, and an
observational study **cannot** separate common-cause from causation. If a lighter sweep frees CPU
and the game speeds up, intervals shrink and some skips return. So:

> **D≈28 ms ⇒ zero skips is an upper bound on the benefit, not a prediction.**

**The experiment that settles it** (live, single-player first): hold the save and speed fixed, vary
the sweep cost by scheduling different domain subsets (e.g. ~40 / ~80 / ~151 ms sweeps), and measure
whether the *interval* distribution shifts. If intervals are unchanged, pace is exogenous and the
budget curve holds. If intervals shrink as our sweep shrinks, we are CPU-coupled and the honest
model is a fixed-point, not a lookup.

## Consequence for the acquire/process split

Existing measurements already bound the split's ceiling: d53's full **encode+decode-gate is 7.9 ms**
against a **49.3 ms** read — processing is roughly **14%** of that domain's wall. Deferring *encode*
therefore buys little. The split only pays if **materialisation** (building rows from game memory)
can be deferred, which requires copying raw bytes at the hour boundary and decoding later.

⇒ Instrumentation must separate **three** phases, not two: (1) guarded reads that touch game
memory (not deferrable), (2) materialise/decode (deferrable **only** behind a raw-byte copy),
(3) encode/emit (deferrable, and already known-small). Build that before building the split.
