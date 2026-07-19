# Production Endgame Evidence Reference — historical experiments and decisions

**PURPOSE:** retain unique production-endgame measurements, experiment methodology, null results,
and live-harness lessons. This file is not a plan of record, completion ledger, residual roll-up,
or current queue. Those roles belong only to
[`MASTER_EXTRACTION_CONTRACT.md`](MASTER_EXTRACTION_CONTRACT.md); per-domain status belongs only
to [`../native/domain_registry.json`](../native/domain_registry.json). All wave/gate/status
language below is historical unless explicitly linked to the 2026-07-14 master.

---

## Historical correctness evidence retained from the old status roll-up

- Columnar d53 encoding measured **−2.9%** (a null) in
  [`d53_columnar_probe.py`](../native/scripts/d53_columnar_probe.py).
- The d47 root-refresh/dedup capture produced 831,039 rows over 509 hours, with zero parse errors
  and one `dead_roots` event. Current d47 status belongs to the master/registry.
- d69's fixed-nine-slot stale-skip model was refuted: a live group can temporarily carry a
  base-class vptr that looks like a stable non-group mismatch, so skipping it can silently drop
  data. The durable fail-closed evidence is to avoid that skip, retain the vacant/tombstone
  member-word classification, and use `d69_stale_vtable_rva_sample` when re-deriving the manager's
  active-group count from a suitable live oracle. Current d69 disposition belongs to the master.

## Historical performance experiments (2026-07-11/12)

The optimization A/Bs below ran at game speed 2. Their relative comparisons are usable; their
absolute frame times are not speed-5 production claims. The authoritative combined speed-5 proof
is tracked only in the master.

Order chosen so each window answers one question; ~15 min each; gate-bracket every run
(pause/unpause = discard). DLL-hold gotcha: -SkipBuild between same-binary windows, 35s+
spacing (build-retry loop against the hold), NEVER cargo-build while a capture is in flight.

### Serial-wall scheduler A/B

Six 240-second windows ran at speed 2 on the advancing game (time order = load order):

| window | code | ft_avg | ft_p95 | ft_max | crit path (ms/frame) |
|---|---|---|---|---|---|
| A1 | OLD | 98.0 | 119.0 | 154 | d53 38.7 |
| A2 | OLD | 96.8 | 115.4 | 142 | d53 39.5 |
| B1 | NEW | 68.7 | 84.9 | 117 | d47 35.4 |
| B2 | NEW | 149.9 | 313.1 | 573 | d200 42.3 + d47 46.4 |
| A3 | OLD | 220.0 | 370.8 | 540 | d200 49.2 |
| B3 | NEW | 257.8 | 506.1 | 1210 | d200 75.1 + d47 63.3 |

The new scheduler was 29% faster at matched moderate load (B1 vs A1/A2) and reduced main-wall
46.4→~0.2 ms. Later blow-up tracked game depth: OLD at A3 was worse than NEW at B2, while the
deepest critical path shifted to `country_bundle` + d47. d47 `dead_roots` scaled with war churn
(2–11/window), making invalidation/rebuild cadence a depth cost.
Paused-state scheduler equivalence is by construction for identical readers over immutable
`CGameState`; it does not prove live temporal equivalence. The discriminating live case remains a
bounded-skew/dataflow argument: d47 consumes the same frame's join output and its heap reads move
earlier, not later. The historical completeness artifact emitted all then-scheduled domains on the
paused baseline; no byte-neutral claim follows for an advancing game.
- **Candidate-prefetch null:** clean 6-window
  palindrome [32,16,0,0,16,32] at speed 2, p2cand_*_152732, applied distance verified
  per-window, mixed=false all six). Paired means: d53 read_time 41.9/41.5/42.7 ms and
  relations phase 36.1/35.4/35.9 ms for dist 0/16/32 — ~3% spread, inside single-window
  noise (same-arm windows differ more). This experiment did not support a candidate-prefetch win.
- **Materialization profile:** the
  Debug three-phase window (threephase_deepwar_p1, 372 frames, speed 2) split the top lanes
  into guarded-read vs materialise (pure local CPU — the weaker-PC law's favorite target):

| domain | lane ms | guarded | materialise |
|---|---|---|---|
| d53 diplomacy | 49.6 | 39.6 (80%) | 10.1 |
| d29 ships | 40.8 | 35.3 (86%) | 5.6 |
| d44 supply | 40.2 | 14.6 (36%) | **25.6** |
| d28 oob_command | 39.3 | 15.6 (40%) | **23.7** |
| d23 map_detail | 32.7 | 1.1 (3%) | **31.6** |

(d47's attribution in this window is polluted by prewarm scope — re-measure; d52 shows
the sampling error bar ~±10%.) Read-bound lanes (d53/d29) get acquisition levers only;
the ~80ms/frame of materialise across d23/d44/d28 is CPU-side and byte-neutral to fix.

### M1 cache experiment and multi-window harness lesson

The M1 change replaced the building-level cache's Mutex+clone path with RwLock+Arc, removing about
26k vector clones/frame in the targeted design. The first two A/B attempts were invalidated when
the game exited between windows; the arm under test was not implicated because each first window
completed under either OLD or NEW.
The third run isolated the crash to the second injection/DLL-swap cycle, independent of arm:
`WH_GETMESSAGE` still referenced the first module while a different DLL was renamed onto the
shared `collector.dll` path. Multi-window tests must use a unique module path/directory per
window and allow the prior hook to tear down; single-window-per-launch tests did not hit this bug.
Validation `mw_d47frzmw_094056` ran tip→old→tip in one PID with all three windows alive and
372/364/359 real frames. Freeze a fixed save for clean A/Bs and preserve unique per-window DLL
directories; changing a live-hooked shared module path invalidates the harness.
The clean same-session six-window result (`mw_m1clean_095124`, frozen save, one PID) was a d23
null. Canary-normalized ratios were d23 0.973, d44 0.960, and d48 0.964, all within ±20% adjacent-
pair noise. Do not cite an M1 d23 speed win. The n,o,n,o,n,o order had position bias; future A/Bs
should use a counterbalanced order such as o,n,n,o,o,n.

### Hour-latch measurement

`p3latch_101330` (600 s, 1724 deep-war hours) reported 1724 observed/accounted, zero unaccounted/
coalesced/backward/anomalous hours, 169,851 polls, zero latch-read failures, queue depth 1, and
1724 unique main-path frames. Frame time was 108.6 ms average / 189.4 p95; the ~283-poll/s latch
showed no visible cost against matched-load windows. Its internal `capture_failed:1724` was the
backstop view, not the emitting main path. This historical run does not decide current defaults.

### Pace-exogeneity experiment

The experiment compared full/mid/light domain subsets in a
`[full,mid,light,light,mid,full]` palindrome at fixed speed. Its metric was game-hours per wall
second; the palindrome centered each tier in session time to cancel linear war-ramp drift.
The first run (`mw_p4pace_102622`) was invalid: the production profile force-added the production
roster after explicit subset selection, so every arm ran the same load. A flat 0.996 light/full
ratio from that run is not pace evidence.
The historical fix, `--scheduled-domains-exact`, suppressed only the production-roster union,
required at least one explicit domain, and retained other production presets. The parser test
`parse_args_scheduled_domains_exact_suppresses_production_union` pinned the distinction.
The valid run (`mw_p4pace_112906`) live-confirmed distinct historical subset sizes and kept the
game alive for all six 240-second windows at speed 2. Mean game-hours/s: full 2.952 (88–118 ms
frames), mid 2.927 (90–100 ms), light 3.146 (10–21 ms), so light/full = 1.066. On this corpus,
~90–100 ms of extraction work cost about 6–7% of game pace; pace was not exogenous.
The current speed-5 acceptance gate lives only in the master. Any performance report should include
accounting/coalesce/skew metrics, frame p50/p95/p99/max, CPU/pace, and the measured critical lane;
the historical experiments showed that assuming d53 is always the critical path is unsafe.

## Historical zstd parameter sweep (2026-07-11)

The w6 corpus was 9.65 GB decoded; measurements used w27+LDM unless noted:

| level/window | artifact | change vs level 3 | observed cost |
| --- | ---: | ---: | --- |
| 3/w27 | 59.02 MB | baseline | — |
| 12/w27 | 49.68 MB | −15.8% | — |
| 19/w27 | 42.37 MB | −28.2% | 57 MB/s single-core-idle; about 12 min per 40 GB |
| 22/w30 | 40.32 MB | −31.7% | only 4.8% beyond level 19; about 1 GB decode memory |

Commit a09c772db changed the then-current default level 13→19 while retaining the explicit
level-3 A/B path and window 27. `level19_window27_roundtrip_is_byte_lossless` verified streaming
SHA-256 and byte identity through the production container reader. The historical launcher
projection omitted pack time when host fold was skipped, so it was not a full finalization measure.

## Current correctness/robustness/finalization routing

The former Wave C/R/F implementation queues, release checklist, and definition of done were removed
as a superseded second ledger. Current fix-pipeline status, open items, residuals, platform scope,
production gate, and done rule live only in
[`MASTER_EXTRACTION_CONTRACT.md`](MASTER_EXTRACTION_CONTRACT.md).
The retained proposed shape for the user-gated cross-modset experiment is one cosmetic mod, one
DLC-disabled combination, and one major overhaul; the master alone owns its current acceptance and
completion state.
