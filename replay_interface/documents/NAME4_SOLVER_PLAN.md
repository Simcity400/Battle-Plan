> **STATUS 2026-07-08 — COLLISION-SOLVER HYPOTHESIS RETIRED by binary RE.**
> Decompiling `hoi4.exe` proved country-name placement is
> `CGraphicalMap::UpdateMapNameMode(EMapNameMode)` → per-area
> glyph layout), run inside a TBB `parallel_for` with **zero label-vs-label
> logic**. A parallel_for cannot de-conflict labels ⇒ there is **no global
> collision solver**; CHILE is pure per-area geometry. The plan below (candidate
> generation + collision de-confliction) is therefore SUPERSEDED — the real task
> is a faithful PORT of the two decompiled functions, gated on the 63-label
> oracle. See `ENGINE_RE_MAP.md` §6. This file is kept for the oracle/harness
> parts (P0 fixture, validation gates), which still apply.

# NAME4 — engine country-title SOLVER plan (2026-07-08)

Goal: reproduce the REAL engine country-name placement, including the
collision-driven cases (Chile) that no single-country geometry law can explain.
"Good enough" is not accepted. Co-designed with Codex (xhigh); the engine's exact
placement is a candidate-selection + collision de-confliction solver, not one
formula. This file is the plan of record; NAME3 (distance-core anchor, shipped
`0d2879f2d`) is candidate `analytic_0` inside it and the compact-country floor.

## Why a solver (evidence)

The South America capture (`hoi4_vanilla_frame3088546.rdc`, decoded) proved the
NAME3 anchor is right for most elongated countries (Argentina 61 px, Peru 51,
Colombia 9, Bolivia 12, Venezuela 21, Paraguay 15) but **Chile** is anchored by
the engine in its NARROW south (y=1848), while its widest part, centroid,
capital, and distance-core all sit north/center. Same PCA angle, shorter arc,
slid ~200 px south into an open ocean slot ⇒ **label collision de-confliction**
(Codex ~80% confidence). That depends on where every OTHER label landed first —
a global solver, not in any game file. So the target is the solver's OUTPUT,
fit + gated against captured ground truth, never a hardcoded per-country table
(that would break mod-compat).

## Ground-truth ORACLE

63 decoded engine labels so far — 38 Europe (`frame4700`) + 25 South/Central
America (new capture): ARG BOL BRA CHL COL ECU PAR PRU URG VEN + MEX & Caribbean
(COS DOM ELS GUA HAI HON NIC PAN) + colonial ENG/FRA/HOL/POR/SPR/LIB. Per label:
`{tag, text, anchor(x,y map-px via (x,2048−z)), angle_deg, arc, h_cap, glyph_quads,
capture_id}`. Capture-level train/validate split (train Europe, validate SA).

## The algorithm (7 parts)

1. **Candidate generation** per component: emit NAME3 `analytic_0` (preserves
   compact countries) + skeleton candidates: Zhang-Suen thin the stride-4
   placement mask, prune leaf branches < max(24px, 0.04·major) or max(D)<0.25·D99,
   collapse to endpoint/junction paths, keep top-8 by `len·mean(D)^0.75`. On each
   path emit anchors every clamp(0.2·arc,16,48)px + the point nearest the core;
   per anchor fit local tangent angle, geo-arc (trace until turn>50° or
   D<0.35·D_anchor), local breadth (q25 of 9 normal rays through the extent mask),
   height_cap, display_arc, bow. ≤64 raw → keep top 12.
2. **Single-label score** (weighted feature loss, weights fit by pairwise ranking
   vs oracle): border clearance, text-fit, dist-to-core, arc prior, angle prior,
   curvature, VP-centroid (only aspect>3).
3. **Collision / global pass** (deterministic): priority = home first, then
   descending target-label-area, then area, VP, (tag,comp). Predicted glyph OBBs
   inflated 0.12·h; pair penalty = SAT overlap / min(area). Greedy place, then ≤4
   coordinate-descent passes (reselect each label's top-12 vs others fixed).
   **Never move a label with zero overlap** (graceful-degrade guarantee).
4. **Height/arc re-calibration** fit together (K_breadth≈0.22, S_max≈2.0,
   skeleton D cutoff, turn cutoff) vs oracle arc + h_cap; the NAME2 size gate must
   stay ≥86% within ±25% on Europe, plus a NEW hyper-elongated arc gate
   (Chile/Norway/Vietnam within ±25%).
5. **Low-anisotropy angle**: candidates = raw PCA / skeleton tangent /
   mainland-core PCA (fringe removed) / VP axis. Use PCA when aspect≥1.8 AND
   PCA≈skeleton (≤20°); else the stable non-PCA angle with path_len/width≥1.6.
   Targets Greece/Ecuador without flipping Spain/Germany.
6. **Validation harness**: per-label Huber loss on anchor/angle/arc/height;
   pass-gate to call it engine-matching = ≥95% matched, anchor median ≤18px p90
   ≤45, angle median ≤7° p90 ≤18, arc median ratio ±12% p90 ±28%, size gate held,
   glyph-quad RMS ≤0.35·h, no new overlap where engine labels are separate.
7. **Risk / fallback**: if the engine uses a hidden priority/continuous
   relaxation not inferable from the captures, keep the candidate solver but only
   let collision move a label when predicted glyph overlap is REAL — data-driven
   Chile slides, no compact regressions, honest partial match.

## Execution phasing (mine to implement; each phase gated)

- **P0** — oracle fixture (63 labels) + Rust validation harness measuring the
  CURRENT law's loss = the baseline the solver must beat. (foundation)
- **P1** — skeleton candidate generation + single-label score + arc/angle
  re-calibration, NO collision. Fixes Chile/Norway arc + Ecuador/Greece angle.
  Gate: no compact regression (analytic_0 wins for them), elongated arc/angle
  improve on the oracle.
- **P2** — collision de-confliction pass. Fixes Chile anchor (and Balkans).
  Gate: Chile slides south, zero movement where no overlap.
- **P3** — global re-lock + live render parity across Europe + South America.

## Capture requests (user — these unlock the UNKNOWNS; parallelize with P0/P1)

Ranked by how much they constrain the solver:
1. **Crowded Balkans zoom** (Greece, Albania, Yugoslavia, Bulgaria, Romania,
   Hungary, Turkey, Italy) — unlocks collision priority + low-aniso angle.
2. **Northwest South America** (Ecuador, Colombia, Peru, Bolivia, N Chile) —
   Ecuador angle + Andes collision sliding.
3. **Southeast Asia** (Vietnam, Siam, China coast, Japan/Korea/Philippines) —
   hyper-elongated arc beyond Chile/Norway.
4. **Africa wide** (Libya, Egypt, Ethiopia, South Africa, Congo) — impassable-
   inclusive breadth + large low-VP interiors.
5. **North America / Caribbean** (USA, Canada, Mexico, Cuba, Haiti/DR, Central
   America) — small-crowded-label de-clutter/hiding.

Same framing discipline as the existing captures: vanilla 1936, political
mapmode, a mid-zoom that frames the whole cluster in one frame.
