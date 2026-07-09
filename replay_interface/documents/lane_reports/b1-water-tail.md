# LANE B1 — water-border residual tail + island loops + fade interior curve

Branch `lane/b1-water-tail`. All numbers from the preserved answer-key pool
`C:\Users\Example\hoi4_border_pool_preserved\session_BORDERZOOM_3` (31,824,096 B,
40,959 strips) with `BATTLE_PLAN_GAME_DIR=<local-path> of Iron IV`.

## Gate numbers (before -> after)

| metric | before (BORD10) | after (BORD11) |
|---|---|---|
| water\|water open strips matched < 1e-3 | 8,534/8,560 | **8,537/8,560** |
| sea_region exact | 8,534 | **8,537** |
| land\|land open | 27,018/27,018 | 27,018/27,018 (held) |
| `cargo build -p map_viewer` | — | GREEN |
| `cargo test -p map_viewer` | — | **631 passed / 0 failed / 1 ignored** |

Net **+3 water strips**, land untouched. Remaining open water residuals: **23**
(was 26).

## What closed, with cited law

**BORD11 — the water smoother is single precision, and f32 BETWEEN passes.**
- f32 accumulators zeroed `XORPS XMM1/XMM2` @0x141b0e284/0x141b0e287
- window summed left-to-right in f32 `ADDSS XMM1,[RDX]` / `ADDSS XMM2,[RDX+4]`
  @0x141b0e2e0/0x141b0e2e4 (stride 8 = one `float2`)
- count via `CVTDQ2PS` @0x141b0e2fb; divide `DIVSS` @0x141b0e2fe/0x141b0e302
- store f32 `MOVSS` @0x141b0e313; radius `min(i, n-1-i, 6)` (const 6 @0x141b0e27e)
  buffer back, so pass 1 reads f32-quantized pass-0 rows)

**The fix that landed:** round each smoothed row to f32 between the two passes
(f64 accumulation within a pass), reproducing the engine's inter-pass f32
storage. This lifts the pool match 8,534 -> 8,537. Code: `smooth_open_rows` in
`replay_interface/native/map_viewer/src/border_world_synthesis.rs`.

`(half_width * delta) / len` — half-width multiplied BEFORE the divide
(`MULSS`@0x141b0da09 then `DIVSS`@0x141b0da0e; `MULSS`@0x141b0da20 then
`DIVSS`@0x141b0da24), `len` from `SQRTSS`@0x141b0d9f7 over `neg_dx^2+dz^2`,
vertices `center ± perp` (`ADDSS`@0x141b0da17/2c, `SUBSS`@0x141b0da58/5d). The
water emit now uses this order; it flips no strip at 1e-3 (smoothing precision
dominates) but is the engine-faithful sequence.

## Key NEGATIVE result (saves the next lane)

A **literal f32 in-pass accumulation** matching the ADDSS/DIVSS decompile
**REGRESSES** the gate to **8,373/8,560**. A naive left-to-right f32 sum of the
station seeds does not reproduce the engine's exact f32 accumulation; by contrast
`f64-accumulate-then-round-to-f32` tracks `round_f32(exact average)`, which the
engine's f32 sum coincides with on far more rows. So the last sub-pixel rows are
**NOT closeable by precision-porting the decompile alone** — do not re-attempt a
naive f32 smoother. Evidence: the exact experiment is documented in the
`smooth_open_rows` comment and `BORDER_LAW.md` §BORD11.

## What stays open, and exactly why

**21 sub-pixel f32 residuals.** Worst deltas are exact 2-3 ULP at world-x ~5000
(0.000977 = 2*2^-11, 0.000244 = 2*2^-13, 0.001465 = 3*2^-11); they miss the 1e-3
gate by 7e-6..4.7e-4 and sit at the f32 reconstruction noise floor (the centerline
is reconstructed from f32 vertex pairs, ULP ~0.0005 at that magnitude). CLOSE
CONDITION: reproduce the engine's EXACT f32 accumulation order + f32 station-seed
construction (not the naive left-to-right sum) so the whole f32 pipeline is
bit-identical; the pool is on disk to re-validate.

**2 real geometry/tracing outliers** (NOT f32 noise):
- `5267|5294` (err ~7.2 wu): junction chain-split mismatch — the engine split the
  pair boundary at a different degree-!=2 corner than our walk. CLOSE: match the
- `8760|8866` (err ~39 wu): north-map-edge chain — our tracer extends the chain to
  raster row j=2047 (world z=-0.5, off the top edge) while the engine stops at
  j=2010. CLOSE: match the engine's map-edge / pole termination of water chains.

**~277 island loops (untraced).** Root cause pinpointed in
`chains_from_pair_edges`: an island whose A|B pixel boundary self-touches at a
degree-4 "pinch" corner is NOT a simple cycle, so the `walk` (which breaks at any
corner of pair-degree != 2) shatters it into open chains starting at the pinch
instead of tracing one closed loop. These get emitted with open-chain law and go
unmatched vs the pool's loops. This does NOT fail the current gate (which asserts
open-strip counts, not loop coverage). CLOSE: robust planar tracing (left/right-
hand turn rule through degree-4 pinches) to keep each island boundary a single
connected cycle, then validate the loop decomposition against the preserved pool;
the engine's exact behavior at pinches is not yet pool-validated (BORD6's
midpoint/min-z/CCW law is proven only on the 605 clean simple-cycle loops).

**Fade interior curve — FILE-DERIVED, effectively closed.** The interior fade
curve of the VISIBLE (gradient-border) system is linear/clamp, no smoothstep, and
already ported (BORD4): `cam_distance` (`standardfuncsgfx.fxh:184-187`, pure
linear clamp), camera fill `1-clamp(cam_distance(100,350),0,0.8)` (`:815`,
`constants.fxh:195-197`), and `gradient_border_distance_to_alpha =
1-saturate((1-vDist)*(vCamDist-0.05)/0.25)` (`:818-823`, `constants.fxh:198-199`).
The ONLY residual is the `border.shader` thin-border path's `vTime_Transparency.y`
easing — NOT used in vanilla political mapmode (BORD4: zero `border.shader`
draws), CPU consumer untraced; its linear reading is file-inferred because
`cam_distance` is the sole shipped camera-height fade primitive (no smoothstep
camera fade exists in any shader).

## Files touched
- `replay_interface/native/map_viewer/src/border_world_synthesis.rs`
  (`smooth_open_rows` inter-pass f32; water extrusion order; gate 8534->8537;
  unit-test reference; module/gate doc comments)
- `replay_interface/documents/BORDER_LAW.md` (§BORD11)
- `replay_interface/documents/FIDELITY_LEDGER.md` (BORD11 + fade rows)

No `extractor/` changes; no `REPLAY_DATA_REQUIREMENTS.md`/`EXTRACTION_NEEDS.md`
changes (border geometry is static map data — no new runtime-state dependency).
Not committed (orchestrator verifies + commits).
