# E14 throughput profile — per-reader frame time on the wired build (2026-07-04)

Measurement (not the final speed-5 proof). Fully-wired build, all 46 production
domains scheduled, 2 captures (89 + 54 frames), PID 33768, build 870ebf. The
per-reader `read_time_us {avg,p95,max,count}` is captured natively in the session
manifest — no timing harness needed, no tracked-source change. Evidence (gitignored):
`runs/e14_throughput/EVIDENCE.md` + `e14_per_reader_timing.json`.

## Headline: readers run in PARALLEL — the tail is the slowest reader, not the sum
Collector scoped threads run readers concurrently, so **frame time ≈ slowest single
reader + ~18ms overhead**, NOT the sum. Sum-of-avgs = 353ms but **frame avg = 143ms
(p95 152ms, max 182ms)**. The ~100ms/hour speed-5 budget is blown by the SLOWEST
reader — so the fix is attacking the top reader(s), not the aggregate.

## #1 target: map_detail (D23) — 126ms avg / 132ms p95 / 169ms max
The current frame tail. **NOT a discovery-cache case.** Code-verified:
`read_compact_map_detail` runs the full state+province topology walk (up to 3
retries) *before* the source-availability check — so it costs 126ms even while
returning 0 rows / SOURCE_UNAVAILABLE. Two-part remedy (a `map_detail` optimization,
tracked separately):
1. **Early availability short-circuit** — check source availability BEFORE the
   topology walk (confirmable at 1936: map_detail is SOURCE_UNAVAILABLE there, so the
   short-circuit drops its cost from 126ms to near-0 on unstable frames).
2. **Cross-frame topology cache** — the state+province topology is static; walk it
   once and reuse (this is the real speed-5 win for the DATA-AVAILABLE case, and must
   be confirmed on a live D04 1944 capture where map_detail returns real rows).

## Second tier (under budget on avg, tails present)
supply_logistics(D44), map_control(D22), divisions(D20), division_equipment(D26),
oob_command(D28), ships(D29) — 25–88ms max. D22/D23/D44 SHARE the map topology, so
they're the natural batch if optimizing map_detail alone doesn't bring the frame
under budget.

## Discovery-cache domains — cost HIDDEN at 1936 (critical caveat)
d53, naval 49/50, d47 show **near-zero here** because they **fail-fast at 1936** (no
data to discover → SOURCE_UNAVAILABLE; `naval_combat_discovery_scan_us: 0`). Their
real per-frame cost is not measurable in this capture. They are ALREADY on
discovery/prewarm caches (this session's work); the remaining task is a **live D04
1944 capture** to CONFIRM the per-frame win (historical baseline: d53 was ~7.4s/frame
pre-cache; d47 prewarm ~516K hits off the measured path).

## Caveat: this is measurement + candidate list, NOT the speed-5 verdict
The definitive zero-skip-at-speed-5 proof needs the **D04 1944 state running at speed
5** (reload D04 + set speed 5 — a user action). The current process is 1936, unpaused,
fast-advancing (134 game-hours in 45s, hours skipped), so this is a real per-reader
profile of the wired build but NOT the skip verdict, and the discovery-cache domains'
true cost is fail-fast-hidden.

## Net: to hit speed-5 hourly capture
1. Optimize **map_detail (D23)** — the #1 tail (topology-cache + early short-circuit).
2. Confirm the **discovery-cache domains** stay off the measured path on a 1944 capture.
3. Re-measure at 1944 speed 5 for the zero-skip verdict.
Both (1) and (2)/(3) need the D04 1944 state — they join the D04-reload-gated queue.
