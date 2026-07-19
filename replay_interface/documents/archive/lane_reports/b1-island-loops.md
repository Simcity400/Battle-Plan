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
