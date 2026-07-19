# Lane F3 — Long-run soak harness

**Branch:** `lane/f3-soak` · **Plan item:** F3 (REPLAY_PRODUCTION_PLAN.md) · **Status:** DONE
**Deliverable code:** `replay_interface/native/replay_app/tests/soak.rs` (new). Also edited:
plan F3 row + this report + mirror `documents/lane_reports/f3-soak.md`.

> Note: this file previously held stale git-conflict content from unrelated lanes
> (f4-anomaly / b1-loops); it has been replaced with the F3 report.

## Mission recap

A long-run soak: a scripted play + scrub loop over a synthetic recording that asserts
zero leaks, zero error accumulation, and stable/bounded memory, while the A4 invariant
`seek(h) == play_to(h)` holds throughout. Headless (drives the timeline + data path, not
D3D). `replay_core` is consumed **read-only** (fx-streams / f2-memory lanes own it).

## Why headless / device-removed scope

The plan's F3 phrasing includes "zero device-removed". Device-removed is a D3D11 swapchain
event that only arises with a live window/GPU — there is none in CI, and the mission
explicitly says to "drive the timeline + the data path, not necessarily D3D". This harness
therefore exercises the entire path a renderer consumes — `replay_core::WorldTimeline`
(ingest → frame cache/LRU → typed views) — under a multi-thousand-iteration seek/play/scrub
script, and proves the **data path** is leak-free and bounded. Device-removed remains a
windowed concern for the F1 frame-budget / product-surface lanes.

## Harness design

Integration test `replay_app/tests/soak.rs`, two `#[test]` gates.

### Memory measurement method — counting global allocator

The test installs a `#[global_allocator]` (`CountingAlloc`) that wraps `std::alloc::System`
and maintains a process-global **net live-bytes** counter (`alloc`/`alloc_zeroed` add
`layout.size()`, `dealloc` subtracts; `realloc` routes through the default `GlobalAlloc`
impl = alloc + dealloc, so it is accounted without an override). A `PEAK` high-water mark is
also tracked. `live_bytes()` sampled at loop boundaries — when the harness holds **no frame
`Arc`s** — reflects exactly what the timeline retains: fixed structures + the O(length) hour
index + the ≤16-entry frame cache. A genuine per-iteration leak accumulates here
monotonically; a bounded design stays flat. Deterministic (no GC), portable (no
winapi/psapi, no `rand`), scoped to this test binary only.

### Gate 1+2+3 — `soak_real_fixture_leakfree_and_invariant`

Opens the **real committed production ingest path**: `open_recording()` on
`fixtures/positive/fx_owner_controller_flip.battleplan` → `WorldTimeline::open`. After a
warmup sweep (fills the ≤16-frame cache to steady state) it samples the baseline, then runs
`BATTLE_PLAN_SOAK_SWEEPS` (default **2000**) iterations of: full forward seek-sweep (with
`seek==play_to` asserted every hour), full backward play_to-sweep, plus 3 pseudo-random
in-range jumps (deterministic xorshift LCG). ≈ **120k** seek/play calls total. Scrub-call
errors are counted, not unwrapped, so "zero accumulated errors" is an explicit gate.
Live heap is checkpointed at quarters for a monotonic-trend gate.

### Gate 4 — `soak_f2_working_set_bounded_by_cache_not_length` (the F2-ceiling hook)

Runs `BATTLE_PLAN_SOAK_SCRUB` (default **20,000**) random-access seeks over two synthetic
in-memory `ReplaySource`s — a 24-hour one and a 26,280-hour (**3-year hourly**) one
(`BATTLE_PLAN_SOAK_HOURS_LONG`) — comparing `seek==play_to` content each iteration. The synthetic
source generates frames **on demand** (stores only the hour vector), so the only thing that
can grow during a scrub is the timeline's frame cache. Post-scrub live heap is measured for
each; the legitimately O(length) hour-index vectors are subtracted (below) to isolate the
**frame** working set, then it asserts the long-timeline frame working set exceeds the short
one by `≤ F2_CEILING_BYTES` (64 KiB).

## Invariant gate (A4 reuse)

`assert_seek_eq_play` compares `seek(h)` and `play_to(h)` by (a) `hour_index()`, (b) the raw
domain map `raw()`, and (c) the `owner_by_state()` typed view — the same equality A4 gates.
Enforced on every hour of every fixture sweep and periodically (every 997th seek) on the
synthetic timelines. Zero mismatches across the whole run.

## Numbers (measured, `--test-threads=1`)

| Gate | Metric | Value | Ceiling | Verdict |
|---|---|---|---|---|
| Real fixture | seek/play calls | ≈120,000 | — | — |
| Real fixture | accumulated errors | **0** | 0 | PASS |
| Real fixture | live-heap growth (2,470,536 B → 2,470,600 B) | **64 B** | 1 MiB | PASS |
| F2 hook | frame working set @ 24 h | 262,582 B | — | — |
| F2 hook | frame working set @ 26,280 h | 262,582 B | — | — |
| F2 hook | `cache_extra` (long − short, index subtracted) | **0 B** | 64 KiB | PASS |

Raw log lines:
```
[soak] real-fixture: sweeps=2000 hours=12 baseline=2470536B end=2470600B growth=64B errors=0
[soak] F2: seeks=20000 short_hours=24 long_hours=26280 live_short=262966B live_long=683062B \
       index_short=384B index_long=420480B cache_short=262582B cache_long=262582B cache_extra=0B
```

## Index-subtraction rationale (length-robustness)

`live_long − live_short = 420,096 B`, which is **exactly** `2 × (26280 − 24) × 8 B`. Two
O(length) `i64` hour-index vectors are held live during a scrub: the `ReplaySource`'s own
`hours` vector **plus** `WorldTimeline::open`'s defensive `to_vec()` copy of it. These are
the snapshot **index spine**, not frame payloads, and scale with length by design. The gate
subtracts `INDEX_COPIES(=2) × hours × 8 B`, leaving `cache_extra = 0 B`. Without this the
gate would false-fail on longer timelines (e.g. a 10-year run) even though frame retention is
bounded. Documented in-code so a future reader (or F2) understands what is / isn't capped.

## F2-ceiling validation hook

This harness is the thing that keeps F2 (snapshot spine + delta windows + LRU) honest. On
**today's pre-F2 code** the frame cache is already a fixed 16-entry LRU
(`replay_core::world::FRAME_CACHE_CAP`), so `cache_extra = 0 B` — the working set does not
scale with timeline length. When F2 lands its snapshot-spine/LRU rework, this same test
re-runs unchanged: if the LRU stops capping, `cache_extra` climbs past 64 KiB and Gate 4
fires. No fake pass — the gate measures real bytes and would have reported unbounded growth
had it existed on the current code.

## Iteration counts / long-soak param

Defaults are CI-fast (whole `replay_app` suite = 186 unit + 2 soak ≈ 4 s wall). A real
multi-hour soak is a param, no recompile:
`BATTLE_PLAN_SOAK_SWEEPS` (fixture sweeps), `BATTLE_PLAN_SOAK_SCRUB` (synthetic seeks),
`BATTLE_PLAN_SOAK_HOURS_LONG` (synthetic timeline length).

## Gates run

- `cargo build -p replay_app` → **green** (exit 0; only 6 pre-existing bin warnings, none new).
- `cargo test -p replay_app` → **186 unit + 2 soak pass, 0 failed**.
- Soak file adds **0** new warnings.

## Constraints honored

Read-only `replay_core` (the custom `ReplaySource` impl lives in the test and uses only the
public `ReplaySource`/`RawFrame`/`SourceError` API). No extractor/, fixture, or generator
edits (real fixtures consumed as-is; the multi-year timeline is a synthetic in-memory source,
not a generated fixture). No module renames/moves. No new crate dependencies. No git commit.
No-eyeball: every gate is a numeric byte/count/equality assertion.
