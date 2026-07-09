<<<<<<< HEAD
# LANE REPORT — lane/f4-anomaly (Phase F4: mid-playback data-anomaly policy)

Plan item: `REPLAY_PRODUCTION_PLAN.md` Phase F — **F4** (crash/error policy:
"log + hold-last-good, never invent" for anomalies that appear **mid-playback**
in an otherwise-valid recording). Worktree `C:\Users\Example\hoi4_lanes\f4-anomaly`,
branch `lane/f4-anomaly`. Crates under work: `replay_fixture_gen` (fixtures +
manifest) and `replay_app` (policy at the `replay_core` **consumer seam**).
`replay_core` and `extractor/` untouched (read-only for this lane).

Startup fail-closed diagnostics already exist (the `--validate-recording`
path + the negative-fixture rejection matrix from lane/e-shell). F4 closes the
open gap: anomalies that surface **mid-stream** in a recording that opened
cleanly.

## Gate numbers (all green)

| Gate | Result |
|---|---|
| `cargo build -p replay_app` | **Finished, 0 errors**, no new warnings |
| `cargo test -p replay_app` | **178 passed / 0 failed** (+8 new anomaly-policy tests over the 170 e-shell baseline) |
| `cargo test -p replay_core` | **30 passed / 0 failed** (untouched; incl. `fixture_conformance` 3/3 — negative matrix 4/4 intact) |
| `cargo run -p replay_fixture_gen` regeneration | **byte-identical**: two fresh same-seed runs identical, and regen == committed across positive+anomaly+negative+manifest |
| Negative-fixture rejection matrix (e-shell) | **unchanged** — positive/ and negative/ fixture bytes are untouched in git; only `manifest.json` gained an additive `anomaly` section |

## Anomaly taxonomy (the 5 seeded fixtures)

All are packed **snapshot-flavor** `.battleplan` containers that OPEN through the
shared `ReplaySource` validator and then **degrade mid-stream** — the opposite
of the `negative/` set, which REJECTS at open. They live in a separate
`fixtures/anomaly/` tree + a separate `"anomaly"` manifest section, so the
strict-negative matrix semantics stay crisp (anomaly ⇒ opens+degrades;
malformed ⇒ rejects). Seed `424242`, 12 hourly snapshots each (11 for the gap).

| Fixture | Class | Injection (crossing = tick 6) | Detected via (consumer-seam signal) | Policy behavior |
|---|---|---|---|---|
| `fx_anomaly_missing_domain` | MissingDomain | `weather` domain present hours 0–5, then the whole key removed hours 6–11 | `frame.weather_status()` transitions `Present → DomainAbsent` (Absent-after-Present) | log once; hold last-good weather value MARKED `HELD weather from hour <5>`; never invent rows |
| `fx_anomaly_type_flip` | TypeFlip | `countries.GER.politics.political_power` number→string at hour 6 | `CountryCore.political_power` goes `Measured → Unobtained` **and** a `coverage_warnings` entry names the field `wrong-typed` | log once; hold last-good number MARKED; never coerced |
| `fx_anomaly_out_of_range` | OutOfRange | GER `manpower.available = -5000` + province id `987654` (absent from the map) from hour 6 | `topbar.manpower_available = Measured(<0)`; `control_by_province` key not in the known map set | flag both (two distinct monitors, each once); hold last-good manpower MARKED; province flagged not rendered; never clamp |
| `fx_anomaly_hour_gap` | HourGap | the mid-recording snapshot at tick 6 is dropped (series jumps by 2) | adjacent recorded hours differ by >1 (`AnomalyPolicy::gaps`) | log once; `classify_hour(missing) → GapUnavailable`; scrubs as unavailable, **never interpolated** |
| `fx_anomaly_identity_mutation` | IdentityMutation | `provenance.mod.modset_id` + `save_version` change at hour 6 (tick-0 keeps the manifest identity, so the container still opens — the reader's identity cross-check is representative/first-snapshot only) | identity fingerprint `(save_version, provenance.mod.modset_id)` differs from the first frame's | log once; hold the baseline identity MARKED; never adopt the mutated identity |

## Policy design (consumer seam, `replay_app/src/anomaly.rs`)

- `AnomalyPolicy` is a **thin consumer-seam** component. It reads only the typed
  `replay_core::WorldFrame` surface — `ViewStatus` per view, `Leaf` value
  trichotomy, `coverage_warnings()`, `save_version()`, and raw `domain()` for
  `provenance` — and never touches recording bytes or `replay_core` internals.
- **Log**: each anomaly episode emits exactly one structured `Diagnostic`
  (`class`, `hour`, `monitor`, `detail`) via `Diagnostic::log_line()`. It fires
  on the *transition* into the anomaly, not every hour it persists; a monitor
  that recovers (view back to `Present`, value back in range) clears so a later
  re-break re-logs.
- **Hold last-good, MARKED**: `Hold { class, monitor, since_hour, marker }`,
  where `marker` is e.g. `HELD weather from hour 41`. `active_holds_at(hour)`
  serves the marked holds for any (scrub-reachable) hour. A held value is never
  presented as live.
- **Never invent**: out-of-range values are flagged, never clamped; an
  hour-gap hour resolves to `HourResolution::GapUnavailable` (never
  interpolated); a domain that never had a good value holds nothing.
- **Wiring**: `timeline.rs::build_provider` scans the recording at open
  (`scan_timeline`), logs every mid-stream diagnostic to stderr, and stores the
  policy in `RecordingTimeline`. The playback loop surfaces the marked holds +
  gap-unavailability through a new fail-safe default `OwnerProvider::anomaly_note(hour)`
  appended to the app status line (empty on clean recordings). The known-province
  set for the out-of-range province check comes from the bundle's
  `province_to_state` map.

## Gate tests (`replay_app/src/anomaly.rs`, inline `#[cfg(test)]`)

8 tests, all green:
- `all_anomaly_fixtures_open_and_play_without_panic` — every anomaly fixture
  OPENS (degrades, not rejects) and playback crosses every recorded hour.
- one test per class: the diagnostic **fires exactly once**, at the **crossing
  hour**, with the **right classification & monitor**; the held value is
  **MARKED** and dated from the last-good hour; **pre-anomaly hours have no
  holds** (unaffected).
- `hour_gap_is_unavailable_not_interpolated` — recorded hours resolve `Exact`,
  the missing hour resolves `GapUnavailable` with the surrounding hours.
- `clean_positive_fixture_raises_no_anomalies` — no false positives on a
  well-formed positive fixture (empty diagnostics, no holds).
- `diagnostic_log_line_is_structured`.

## `replay_core` API asks

**None.** The full policy is implementable at the consumer seam with today's
`replay_core` surface — `ViewStatus` (DomainAbsent/DomainUnavailable/Present),
the `Leaf` trichotomy, `coverage_warnings()` (wrong-typed values are already
`Unobtained` + a warning), `hours()` (gap detection), `save_version()`, and raw
`domain("provenance")` for the modset fingerprint. No additive API is required
for the post-a5 reconciliation lane. (Optional nicety, not needed: a typed
`provenance.mod.modset_id` accessor would let the identity fingerprint avoid the
one raw-domain read; today's raw read through the public `domain()` accessor is
seam-legal and sufficient.)

## Files touched
- New: `replay_app/src/anomaly.rs` (policy + 8 gate tests),
  `replay_interface/fixtures/anomaly/*.battleplan` (5 seeded fixtures).
- Edited: `replay_fixture_gen/src/main.rs` (anomaly scenarios + `anomaly`
  manifest section; positive/negative generation untouched),
  `replay_interface/fixtures/manifest.json` (additive `anomaly` section),
  `replay_app/src/main.rs` (`mod anomaly`), `replay_app/src/timeline.rs`
  (policy scan at open + `anomaly_note`), `replay_app/src/app.rs` (status-line
  hold marker), `REPLAY_PRODUCTION_PLAN.md` (F4 row → DONE).
- Untouched: `replay_core/*`, `extractor/*`, all `*_product_path_blocked`
  gates, `fixtures/positive/*`, `fixtures/negative/*`.
=======
# LANE B1 — island-loop residual: degree-4 pinch fix POOL-REFUTED; residual is coastline loop-closure

Branch `lane/b1-loops`. Referee = preserved answer-key pool
`C:\Users\Example\hoi4_border_pool_preserved\session_BORDERZOOM_3` (31,824,096 B,
40,959 strips) with `BATTLE_PLAN_GAME_DIR=<local-path> of Iron IV`,
via `gate_vanilla_preserved_pool_bord9_water_smoothing_counts` in
`map_viewer/src/border_world_synthesis.rs`.

## Verdict

The mission's root-cause hypothesis — that the ~277 island loops are self-touching
islands shattered at **degree-4 pinch** corners, closeable by planar left/right-hand
tracing — is **refuted by the pool**. I implemented the planar turn-rule closure
(right / left / straight), A/B-tested each against the pool, and every mode moves
the match count the WRONG way. Per gate 3 ("if NEITHER matches the pool, label the
choice FIT and document what evidence would decide it"), **no tracer is shipped and
the source is at baseline.** The pool decode reveals the real residual: land|water
**coastline loops** the engine closes through **odd-degree (degree-3) junctions**,
which the degree-4 pinch mechanism cannot produce.

## Gate numbers (before / after each experiment)

| experiment | matched strips | unmatched pool-closed | land open | water open | sea_region |
|---|---|---|---|---|---|
| **baseline (shipped, break at pair-deg != 2)** | **40,324** | **166** | **27,018/27,018** | **8,537/8,560** | **8,537** |
| right-hand pinch close | 40,312 | 174 | 27,018/27,018 | 8,537/8,560 | 8,537 |
| left-hand pinch close | 40,308 | 178 | 27,018/27,018 | 8,537/8,560 | 8,537 |
| straight (crossing) close | 40,308 | 178 | 27,018/27,018 | 8,537/8,560 | 8,537 |

* The two **required OPEN gates hold in every mode**: land 27,018/27,018, water
  8,537/8,560 (the 23 water residuals are the known separate f32 issue — untouched).
  They are handedness-invariant because the pinch change only ever touches all-even
  components, never an open land|land or water|water chain.
* Right-hand closes **+4** genuine pinch loops (`matched_closed` 713 → 717) but
  mis-closes **−16** coastline arcs the engine keeps OPEN → **net −12 matches**.
  Left/straight are worse. Every handedness is a net regression, so **the tracing
  rule is a FIT and none is shipped.**
* Final state after revert: **matches = 40,324, land 27,018/27,018, water
  8,537/8,560, sea_region 8,537** (baseline exactly restored). `cargo build -p
  map_viewer` GREEN; `cargo test -p map_viewer` border suite 50/50 pass.

## The tracing rule that was tested (and why it fails)

Component-scoped planar decomposition: each province-pair boundary graph is split
into connected components; a component with any odd-degree corner keeps the exact
BORD5 open-chain decomposition (protecting the gate), and an all-even component is
traced as closed loop(s) with a turn rule at degree-4 pinches — right-hand
`[right, straight, left, back]`, left-hand `[left, straight, right, back]`, straight
`[straight, right, left, back]` in image space (`+i`=E, `+j`=S). Land/water open
counts confirm this is gate-safe, but the pool shows it closes the wrong things.

## What the residual actually is (direct pool decode)

Dumping the unmatched pool-closed strips against the synth (`BORD_DIAG` /
`BORD_DIAG2` env gates, since removed): the **~166 unmatched pool-closed strips are
almost all land|water COASTLINE loops** — every nearest synth candidate is
`sea_coast`. Each is a *truly* closed ribbon (pool first row == last row) built with
the BORD6 closed-loop **midpoint** law, lying exactly on top (bbox distance ~0) of
one of our **OPEN** coast arcs, but with **+1..+2 more edges**.

Worked case — pool strip 6697, pair `2189|5240`:

* pool = **14-edge CLOSED** midpoint loop, 15 rows, first==last=`(5078.0, 7.5)`
  (integer-x ⇒ edge-midpoint rows, no corner endpoints);
* synth = **12-edge OPEN** arc, 14 rows, first==last=corner `(5079.5, 10.5)`
  (`.5,.5` ⇒ corner-endpoint rows), PLUS a separate 3-edge open tail chain.

So the pair boundary is a **"lollipop"** (loop + tail) whose junction corner J is
**degree-3 (odd)**. The engine CLOSES the coastline loop through J and emits the
tail separately; BORD5's "break at pair-degree != 2" shatters the loop at J into
open arcs. This is a **class-dependent coastline loop-closure law through
odd-degree junctions**: land|water closes, while land|land and water|water keep the
BORD5 break (the pool-exact 27,018 / 8,537 open gate proves the break is correct for
those two classes). Degree-4 pinch handedness is the wrong mechanism entirely.

## FIT flags

* **FIT (not shipped):** degree-4 pinch handedness (right vs left vs straight).
  None matches the pool; all regress it. Documented, not merged.
* **OPEN engine-rule gap:** the coastline loop-closure rule (which land|water
  degree-3 junction corners close the loop vs break) and the **+1..+2 edge** count
  discrepancy on the closed loop vs our arc. No shipped file encodes the
  corner loop/break decision.

## RE target (follow-up, not guessable from files)

(`0x141b0b700`) at its corner/chain handling to recover: (a) which land|water
junction corners close the coastline loop vs break it, and (b) the exact edge set
of the engine's closed loop (the +2-edge delta). Re-validate against the preserved
pool (on disk; no new capture needed). Until then the island-loop residual stays
OPEN and SeaRegion + `--borders-from-files` remain blocked.

## Files touched (docs only — source reverted to baseline)

* `replay_interface/documents/BORDER_LAW.md` (§BORD12).
* `replay_interface/documents/FIDELITY_LEDGER.md` (BORD12 border row).
* `replay_interface/documents/lane_reports/b1-island-loops.md` (this report).
* `border_world_synthesis.rs`: **no net change** (pinch tracer implemented,
  A/B-tested, and reverted; gate exactly 40,324 / 27,018 / 8,537). No `extractor/`,
  `replay_core`, or fixture edits. Not committed.
>>>>>>> lane/b1-loops
