# LANE B5 — River class→width ramp + rivers.bmp palette semantics

Branch `lane/b5-rivers`. Plan item **B5** (REPLAY_PRODUCTION_PLAN.md:220).
Goal: close the river residuals — the class-3+ width ramp and full palette
semantics — from shipped files, or precisely document the engine-code / RE
target when a value is genuinely not file-derivable. **No fabricated widths.**

Every parameter below is labelled **FILE / CAPTURE / FIT / UNKNOWN**.

---

## 1. Palette semantics table

Source of truth for the split: `common/defines/00_defines.lua:936-938`
(resolver-read via `RiverPaletteDefines`, mod-aware). Verified against the real
shipped file by `gate_vanilla_defines_files_carry_the_river_palette_and_fade`
and pinned to the exact vanilla `rivers.bmp` histogram by
`gate_full_map_river_extraction_vanilla`.

| Palette index | Vanilla px | Meaning | Label | Citation |
|---|---|---|---|---|
| 0 | 317 | River **source** (strip start) | **FILE** (small-range start) + **CAPTURE** (every class-0 px is a strip start in the captured world VB) | `00_defines.lua:936` `RIVER_SMALL_START_INDEX=0`; `river_world_synthesis.rs` doc-comment (317 sources) |
| 1 | 250 | **flow-in** marker | **wiki convention / UNKNOWN in files** (only known to be inside FILE small range) | the game wiki; no shipped-file citation |
| 2 | 14 | **flow-out** marker | **wiki convention / UNKNOWN in files** | the game wiki; no shipped-file citation |
| 3..=6 | 35104 / 10985 / — / 13144 | **small** river channels | **FILE** small | `00_defines.lua:937` `RIVER_SMALL_STOP_INDEX=6` |
| 7..=11 | 1154 / — / 1677 / 43 / 27665 | **large** river channels | **FILE** large | `00_defines.lua:937-938` (`>6`, `<=RIVER_LARGE_STOP_INDEX=11`) |
| ≥12 (vanilla 254 land, 255 sea) | 7479319 / 3964664 | **not a river** | **FILE** | `00_defines.lua:938` (`>RIVER_LARGE_STOP_INDEX`) |

Notes:
- Classes **5 and 8 are absent** from the vanilla map (0 px).
- FILE small/large gameplay totals: **small 59,814** (0..=6), **large 30,539** (7..=11).
- The per-index 0/1/2 source/flow meaning is finer than the shipped files. Only
  the **small/large/not-river** partition and **index-0 = source** are grounded;
  1/2 = flow-in/flow-out is community/wiki convention and is labelled
  **UNKNOWN-in-files** in code (`RiverPixelKind`).
- River-ending fade: `RIVER_FADE_FROM=20.0` / `RIVER_FADE_TO=3.0`
  (`00_graphics.lua:993-994`, **FILE**, span 17.0) via `RiverFadeDefines`.

## 2. Class → width table

**Width is NOT in any shipped file.** Proof (FILE): the river shader takes the
vertex buffer geometry as-is —

```
gfx/FX/river.shader:198   Out.vPosition = float4( v.vPosition.xyz, 1.0f );
```

— it never derives width from the palette class. `00_defines.lua` carries only
the small/large split and crossing penalties (`RIVER_CROSSING_PENALTY*`), no
width. Therefore per-class width is **baked into the river VB by engine CPU
mesh-generation**. The best available grounding is **CAPTURE**, measured from
the captured world river VB (`river_world_synthesis::river_world_width_for_class`):

`width(k) = 1.11 + 0.17·(k − 3)` world units

| Class k | Width | Label | Basis |
|---|---|---|---|
| 3 | 1.11 | CAPTURE | measured median (delta<0.02) |
| 4 | 1.28 | CAPTURE | measured median |
| 5 | 1.45 | CAPTURE-law **interpolation** | absent in vanilla; bracketed by measured 4/6 |
| 6 | 1.62 | CAPTURE | measured median |
| 7 | 1.79 | CAPTURE | measured median |
| 8 | 1.96 | CAPTURE-law **interpolation** | absent in vanilla; bracketed by measured 7/9 |
| 9 | 2.13 | CAPTURE | measured median |
| 10 | 2.30 | CAPTURE | measured median |
| 11 | 2.47 | CAPTURE | measured median |
| 0/1/2, ≥12 | — (`None`) | — | source/flow markers & not-river have no steady width; **fail-closed, never fabricated** |

Source-strip ramp: `width = min(0.1·station, width(k))` (**CAPTURE**, 0.1 world
units/station); ending fade span 17.0 (**FILE**, `00_graphics.lua:993-994`).

**Retired this lane:** the fabricated per-class table `1.1, 1.4, 1.7, 2.0, 2.4,
2.8, 3.2, 3.7, 4.2` and the FIT literal `RIVER_SOURCE_WIDTH_RAMP_STEPS=11.0`
that previously lived in `river_mesh_synthesis.rs`. They were uncited plausible
numbers (~2× wrong at class 11 vs the measured 2.47) and violated the lane
no-fabrication rule. `river_width_for_class` now delegates to the single
CAPTURE-grounded law; an unclassifiable polyline **fails closed** with an error
instead of a `1.1` fallback.

### RE target (binary; out of scope for this lane)

To promote width from **CAPTURE → FILE**: find the hoi4.exe routine that builds
the **stride-32 river vertex buffer** from `map/rivers.bmp`. Its per-class
half-width lookup is the **half-distance between the paired side-0 / side-1
`POSITION.xz`** at each steady-state station. Ghidra projects are owned by other
lanes; this lane documents the target only.

## 3. Gate numbers (all offline; vanilla install present, no capture needed)

- `cargo build -p map_viewer` — **clean** (pre-existing `light_law.rs` warnings only).
- `cargo test -p map_viewer` — **634 passed / 0 failed**.
- River tests specifically — **19 passed / 0 failed**.

Key new/updated gates:
- `gate_full_map_river_extraction_vanilla` (NEW) — real shipped `rivers.bmp`:
  - total **11,534,336** px (5632×2048), **0 unclassified**,
  - exact per-index histogram (see table §1),
  - **90,353** river px, FILE small **59,814** / large **30,539**,
  - synth mesh **583 strips**, **90,118 reached pairs**, **1 singleton**,
    **234 sourceless-dropped** — the 234 matches the captured world VB drop
    exactly (12 four-connected sourceless components),
  - present width classes `[3,4,6,7,9,10,11]`, absent `[5,8]`.
- `gate_river_pixel_classification_is_total` (NEW) — every u8 buckets into
  exactly one `RiverPixelKind`, agreeing with the FILE palette predicates.
- `extraction_stats_reconcile_on_synthetic_map` (NEW) — counts reconcile.
- `width_law_matches_measured_class_table` (EXTENDED) — measured 7-class law +
  5/8 interpolation bracket + `None` for 0/1/2/12/254/255.
- Retained: `gate_river_palette_defines_boundaries`,
  `gate_river_fade_defines_span_matches_retired_fitted_literal`,
  `gate_vanilla_defines_files_carry_the_river_palette_and_fade`,
  `gate_river_world_comparison_against_captured_sessions` (capture-gated).

## 4. Files changed

- `map_viewer/src/river_mesh_synthesis.rs` — retired fabricated width table +
  `RIVER_SOURCE_WIDTH_RAMP_STEPS`; delegate width to the CAPTURE law; fail-closed.
- `map_viewer/src/river_world_synthesis.rs` — `RiverPixelKind`,
  `classify_river_pixel`, `RiverExtractionStats`, `river_extraction_stats`;
  full-map + classification + reconciliation gates; width-law citation +
  5/8-interpolation coverage.
- `map_viewer/src/lib.rs` — export the new public API + `RiverPaletteDefines` /
  `RiverFadeDefines`.
- `replay_interface/documents/FIDELITY_LEDGER.md` — §2.10 rows + tail item 11.

All game-file reads go through `GameFileResolver` (MOD_COMPATIBILITY.md).
No recording-side dependencies added (rivers are static).

## 5. Open residuals (exact next evidence)

1. **Per-index 1/2 flow-in/flow-out meaning** — no shipped-file citation. Next
   evidence: official the game map-format documentation, or binary RE of the VB
   builder's per-index branch (does it treat 1/2 differently from source/channel
   at merge/split junctions?). Currently UNKNOWN-in-files, wiki-labelled.
2. **Width CAPTURE → FILE promotion** — RE target in §2. Also would confirm
   whether classes 5/8 (unused by vanilla) truly follow the same linear law
   (currently interpolated) — needs a modded map exercising those classes or the
   binary lookup table.
3. **Engine per-strip centerline smoothing / fade smoothing** — float32-error
   dependent, not recoverable offline (documented ENGINE-LAW gap in
   `river_world_synthesis.rs`); the world synth stays a comparison/off-default
   tool, not a default render path.
