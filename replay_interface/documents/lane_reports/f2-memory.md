# Lane F2 — memory ceiling for the replay timeline

**Branch:** `lane/f2-memory` (base `50f06e113`, wave-5 tip, verified) · **Plan item:** F2
(REPLAY_PRODUCTION_PLAN.md) · **Status:** DONE · **Date:** 2026-07-10
No commits made (orchestrator commits). No codex. extractor/ + fixtures untouched.
No modules renamed/moved. Previous F3 content of this scratch file is archived at
`replay_interface/documents/lane_reports/f3-soak.md`.

## 1. O(timeline-length) / per-seek allocation audit (mission items 1+2)

**replay_core — every O(hours) allocation, bounded or justified:**

| Allocation | Verdict |
|---|---|
| `StoreSource.hours: Vec<i64>` (8 B/hour) | JUSTIFIED floor — random-access index, built once at open, scrub-invariant. 210,240 B @ 3 years. Doc comment added (`source.rs`). |
| `StoreSource.files_by_hour: BTreeMap<i64, String>` | JUSTIFIED floor — hour→file index required to serve `frame(hour)` without re-reading the store index. Per-open. Doc comment added. |
| `StoreSource.warnings: Vec<String>` | JUSTIFIED — bounded by malformed index entries, per-open. |
| `WorldTimeline::open`'s `hours.to_vec()` copy | JUSTIFIED floor — defensive copy of the validated spine, kept per the plan's two-index-vecs framing; soak's `INDEX_COPIES = 2` accounting unchanged. Doc comment added (`world.rs::open_with_config`). |
| Decoded `WorldFrame`s | BOUNDED — reachable only via the LRU cache (now budget-capped) + caller Arcs. |

**View-accessor clone audit (incl. the 12 new fx-streams views):** all fx-streams
accessors (research / focus / politics_detail / production / intel / trade /
leaders / occupation / power_balance / designs / naval_air_combat / scripted_gui /
raids + statuses) return `&`-borrows — CLEAN. One offender found:
`WorldFrame::visibility_for` cloned the full per-POV view (province BTreeMap +
spotted-unit vecs) on EVERY call — per-seek garbage on the e2e/POV paths.
**Fixed:** returns `Cow<'_, PovVisibilityView>` — borrowed when the POV exists,
allocation-free fail-closed view (empty map, Absent/Unobtained leaves, never
fabricated visibility) when absent. `spotted_units_for` signature unchanged.
Trichotomy semantics unchanged; existing rt_pov_visibility / world_timeline /
e2e gates pass byte-identically.

**Consumer-side find (replay_app):** `AnomalyPolicy.hold_timeline`
(`replay_app/src/anomaly.rs`) pushed one `(hour, Vec<Hold>)` entry per OBSERVED
HOUR unconditionally — O(recorded hours) growth (~26k entries / 3 years) even on
an anomaly-free recording. **Fixed:** change-points only (append only when the
snapshot differs from the previous change-point; same-hour re-observation still
overwrites; an initial all-clear stores nothing). `active_holds_at` is
query-equivalent (last entry at-or-before) and now answers by `partition_point`
binary search with one clone of the answer (was O(entries) clones per query).

## 2. Configurable, memory-aware LRU budget (mission item 3)

- `replay_core::TimelineConfig { frame_cache_entries: usize }` (exported);
  `Default` = 16 — DERIVED: the pre-F2 fixed `FRAME_CACHE_CAP`, F3-soak-proven.
  0 = caching disabled (documented; still correct, minimum working set).
  Budget deliberately in ENTRIES, not bytes: a frame's heap size is
  recording-content-dependent and estimating it would be a fabricated value;
  bytes = entries × observed frame size, measured by the soak harnesses.
- `WorldTimeline::open_with_config(source, config)`; `open` delegates with the
  default. New accessor `frame_cache_entries()`. `FrameCache` carries `cap`.
  **No global statics.**
- Threaded through replay_app's EXISTING open path: the budget rides
  `TimelineMode::Recording { path, timeline: TimelineConfig }` into
  `build_provider` → `open_with_config`. New CLI flag `--frame-cache <entries>`
  (USAGE updated), fail-closed: rejects 0, rejects non-numeric (value named),
  rejects use without `--recording` (a budget that silently applies to nothing
  would let a weak-PC profile believe it took effect).

## 3. Gates (mission item 4) — numbers, not adjectives

Dev profile, default knobs (BATTLE_PLAN_SOAK_SCRUB=20000, BATTLE_PLAN_SOAK_HOURS_LONG=26280,
BATTLE_PLAN_SOAK_SWEEPS=2000). New file `replay_app/tests/soak_f2.rs`; F3's
`tests/soak.rs` UNTOUCHED and green.

| Gate | Result | Ceiling/floor |
|---|---|---|
| (a) long-timeline budget @ cap 4: 20k random seeks, 24 h vs 26,280 h synthetic (16 KiB deterministic per-frame payload), index spines subtracted analytically | cache_short = cache_long = **191,560 B → cache_extra = 0 B** | ≤ 131,072 B PASS |
| (a') memory-aware knob, same 26,280 h timeline: cap 16 vs cap 4 | **723,388 B vs 191,560 B → knob_delta = 531,828 B** (weak-PC cap 4 cuts the frame working set ~73%) | ≥ 98,304 B PASS |
| (b) shrunken-cache correctness (caps 4/1/0): committed `.battleplan` fixture, fwd+back+alternating-end thrash; `seek(h)==play_to(h)` + byte-equal raw/owner views vs the default-16 timeline | 0 divergences | exact PASS |
| (b-core) same law inside replay_core (`frame_cache_budget_is_content_neutral`, dir-store fixture, caps 4/1/0, 2 thrash passes, A4 at every budget) | pass | exact PASS |
| (c) no upward heap trend: 2000 fwd+back sweeps + random jumps @ cap 4 | growth **60 B** (957,878 → 957,938 B); checkpoints flat [957,872 / 957,906 / 957,906 / 957,904 / 957,906]; 0 errors | ≤ 1 MiB PASS |
| F3 soak un-weakened | 2/2 pass | PASS |
| `cargo build --workspace` | clean (only pre-existing warnings in untouched files) | PASS |
| `cargo test -p replay_core` | **45 passed / 0 failed** (was 44; +1 budget gate) | PASS |
| `cargo test -p replay_app` | **194 passed / 0 failed** = 190 unit (incl. 2 new `--frame-cache` CLI gates) + 2 soak (F3) + 2 soak_f2 | PASS |
| `cargo fmt --check` on MY hunks | clean. The repo carries **64 pre-existing** fmt diffs under this rustfmt (version skew, proven by stash A/B against pristine `50f06e113`: identical per-file counts — world.rs 15, roundtrip_gates.rs 15, soak.rs 2, etc., all code I did not write). My edits introduced exactly 2 new hunks (main.rs, soak_f2.rs); both fixed. Pre-existing dirt NOT reformatted (scoped-diff rule). | PASS (mine) |

**Memory before/after (product-facing):** 3-year-scale timeline, 16 KiB frames,
after a 20k-seek scrub — default 16-entry budget retains **723,388 B**;
`--frame-cache 4` retains **191,560 B**; both INDEPENDENT of timeline length
(24 h vs 26,280 h identical to the byte at the same cap). Justified O(hours)
floor: index spines 8 B/hour × 2 = 420,480 B @ 26,280 h, subtracted analytically
(`INDEX_COPIES = 2`, same law as soak.rs).

## 4. Files touched

- `replay_interface/native/replay_core/src/world.rs` — TimelineConfig,
  open_with_config, frame_cache_entries, FrameCache cap, visibility_for → Cow,
  audit doc comments.
- `replay_interface/native/replay_core/src/lib.rs` — export TimelineConfig.
- `replay_interface/native/replay_core/src/source.rs` — audit doc comment only.
- `replay_interface/native/replay_core/tests/world_timeline.rs` — new gate
  `frame_cache_budget_is_content_neutral`.
- `replay_interface/native/replay_app/src/main.rs` — `--frame-cache` + USAGE,
  `TimelineMode::Recording` struct variant, 2 new + 1 updated CLI gates.
- `replay_interface/native/replay_app/src/timeline.rs` — build_provider threads
  the config; F4-scan comment updated to the audited truth.
- `replay_interface/native/replay_app/src/anomaly.rs` — hold_timeline
  change-points; active_holds_at binary search.
- `replay_interface/native/replay_app/src/e2e_playback.rs` — 3 call sites to
  `spotted_units_for` (Cow migration; behavior identical).
- `replay_interface/native/replay_app/tests/soak_f2.rs` — NEW (gates a/a'/b/c).
- `replay_interface/documents/REPLAY_PRODUCTION_PLAN.md` — F2 row → DONE.
- `LANE_REPORT.md` (this file).

`FIDELITY_LEDGER.md` deliberately NOT touched: it is the render-fidelity
parameter ledger (FILE/CAPTURE/FIT/UNKNOWN rows per visual layer); this lane
adds/closes no fidelity row — every change is content-neutral and proven so by
byte-equality gates. F2 residuals are performance residuals, filed in the plan
F2 row (below).

## 5. Honesty labels for introduced values

- `TimelineConfig::default().frame_cache_entries = 16` — DERIVED (pre-existing
  FRAME_CACHE_CAP, F3-soak-proven; not a new fit).
- soak_f2 tolerances (128 KiB / 96 KiB / 1 MiB) — DERIVED test-harness bounds:
  1 MiB reuses the F3 leak law verbatim; 128 KiB = F3's 64 KiB doubled for
  cross-test allocator noise in a parallel test binary; 96 KiB = half the
  analytic 12-extra-frames × 16 KiB payload signal. Gates on measurements, not
  render values; measured margins were 0 B / 531,828 B / 60 B.
- 16 KiB synthetic frame payload (`PAD_REPEATS = 1024`) — DERIVED test
  instrumentation (deterministic function of the hour; content asserted equal
  across seek/play), makes the knob signal dominate allocator noise.
- No FILE/RE/FIT values introduced; no fabricated defaults; absent POVs still
  render as fail-closed Absent/Unobtained views.

## 6. Residuals / could-not-close (also in plan F2 row)

1. **Open-time O(hours) full decode (latency, not memory):** `build_provider`'s
   F4 anomaly scan decodes every recorded frame once at open. Memory stays
   LRU-bounded (verified), but a 26,280-hour recording pays ~26k frame decodes
   before the window opens on a weak PC. Needs an incremental/lazy scan design —
   out of F2 (memory) scope; flagged in `timeline.rs` and the plan row.
2. **`Gap.missing` materialization (anomaly.rs):** a gap's missing hours are
   stored and Debug-formatted in full — O(gap span); pathological only for
   recordings with multi-year holes. A range representation would fix it; left
   because it is F4's diagnostic format contract.
3. **Third index spine:** `AnomalyPolicy.hours` copies the hour spine again
   (8 B/hour, per-open, justified — classify_hour binary-searches it). An
   `Arc<[i64]>` share would save 210 KB @ 3 years if ever needed.
4. **Delta windows** (the "snapshot spine + delta windows" half of the F2 item
   text) remain future work behind the `TimelineLookup` A4 seam — pre-existing
   design decision, unchanged here; the memory ceiling holds without them.
