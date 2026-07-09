# B4 — Tree scatter law + season CB1 (lane/b4-trees)

Derive the tree-scatter law (per-instance anchor, rotation, model selection) and
the season CB1 constants from shipped installed content, and gate numerically against
the preserved live capture corpus
`C:\Users\Example\hoi4_capture_corpus_preserved\live_trees_measure`.

## TL;DR

- Scatter **count + batch structure + model + LOD**: FILE-derived, gated **8/8
  exact** against the corpus (0 count error).
- Per-instance transform **math** (anchor/yaw/slope consumption + height jitter):
  FILE-derived from `gfx/FX/tree.shader` VS source (exact).
- Per-instance **anchor value** (exact cell-center vs jitter) and **yaw value**:
  **NOT gate-able from this corpus** — the per-instance vertex buffer (IA slot 1,
  `TEXCOORD4`/`TEXCOORD5`) was **not recorded**. Yaw generation is CPU-side and in
  no shipped file → the current rotation is a **FIT** stand-in.
- **Season CB1**: `vTreeFade` FILE-exact (gated 8/8, |err| < 1e-4); `vSeasonLerp`
  / `vSeasonColumn` are a CAPTURE snapshot (0.0 / 0.0); the date→season map is
  UNKNOWN (CPU-side).

## Evidence corpus structure (read-only)

`live_trees_measure/map_frame_trees_pass_bundle.json` = the trees pass, captured
as **8 `DrawIndexedInstanced` draws** (`trees_frame_draw000..007_*`). Each draw:
IA slot 0 = the template mesh only (stride 56 = pos3+norm3+tan4+uv0_2+uv1_2);
IA slot 1 (the per-instance stream the VS declares) is **absent**; VS CB0 (416 B,
scene), VS CB1 (80 B, season/shadow), VS CB2 (3200 B), PS CBs. Camera height
`vCamPos.y = 348.06973` (scene CB0 byte 212).

The 8 draws = **2 models × 4 visible chunks**:
draws 0-3 = `mapobject_5` counts 37/86/452/296; draws 4-7 = `mapobject_6` counts
69/141/783/416.

## Derived scatter law (with citations)

**1. Instance generation (FILE).** One instance per nonzero `map/trees.bmp` cell.
The palette index is the model id. `map/trees.bmp` is 1650×600, 8-bpp indexed;
cells map to the 5632×2048 world (heightmap dims) by cell center.

**2. Batching + draw order (FILE, corpus-consistent).** The engine builds one
`DrawIndexedInstanced` per (visible 256-world-unit chunk, palette index), instance
count = nonzero cells of that index in the chunk. Draw order = model (palette)
ascending, then chunk x ascending, then chunk z **descending**, zero-count chunks
skipped. `TREE_CHUNK_WORLD_UNITS = 256.0`
(`tree_instance_synthesis.rs`; the four visible chunks are exactly the CB0 frustum
ground footprint).

**3. Model selection (FILE).** Palette index `N` → `gfx/entities/trees.gfx`
binding `mapobject_N` mesh (index 5 = `forest_dunge_blend.mesh`, 6 =
`forest_dunge.mesh`). The drawn LOD shape = the largest `loddist` bracket ≤ camera
height (`pdx_model` `lod_distances`). Both proven byte-exact (template VB/IB +
LOD bracket) in `gate_tree_scatter_chunk_law_vs_live_trees_measure`.

**4. Per-instance transform consumption (FILE — `gfx/FX/tree.shader:200-258`).**
The VS input is `VS_INPUT_INSTANCE` with per-instance
`float4 vPos_YRot : TEXCOORD4` and `float3 vSlopes : TEXCOORD5`. The world vertex is:
```
vRandom      = vPos_YRot.w / 2π
heightScale  = 0.75 + frac(vUV1.x + vRandom) * 0.5   // per-instance height jitter
pos.y       *= heightScale
pos.xz       = rotate(pos.xz, yaw = vPos_YRot.w)      // yaw about Y
pos.y       += pos.x*vSlopes.x + pos.z*vSlopes.y      // slope tilt
pos.xyz     += vPos_YRot.xyz                          // anchor (world position)
clip         = mul(ViewProjectionMatrix, pos)
```
So **anchor = `vPos_YRot.xyz`, yaw = `vPos_YRot.w`, slopes = `vSlopes.xy`**, and
the mesh height is randomly scaled in [0.75, 1.25) from the yaw + `uv1.x`. This is
the exact GPU-side scatter law; it is proven from shader source, not fit.

**5. Anchor + slope VALUES (FILE-hypothesis, ungated).**
`tree_instance_synthesis.rs:149-154`: anchor = cell-center scaled to world +
`heightmap*0.1` (flipped z); slopes = heightmap central-difference gradient. The
8/8 **count** gate is consistent with one-instance-per-cell but does **not** prove
the anchor is the exact cell center (an intra-cell jitter would give identical
counts). No per-instance ground truth exists in this corpus → the anchor value is
a file-motivated hypothesis, not numerically gated.

**6. Rotation VALUE (FIT / UNKNOWN).** `vPos_YRot.w` is generated CPU-side by the
engine and appears in **no shipped file**. `deterministic_tree_rotation` is an
explicit stand-in **FIT**. Not gate-able here (instance VB unrecorded).

## Season CB1 (`gfx/FX/tree.shader:187-193`, `ConstantBuffer(1,32)`)

Layout (80 B): `float4x4 ShadowMapTextureMatrix` (0..64), `float vSeasonLerp`
(64), `float vSeasonColumn` (68), `float vTreeFade` (72).

- **`vTreeFade` — FILE.** The PS returns it as the tree output alpha
  (`float4(vColor, vTreeFade)`, `:408`). It equals
  `saturate((TREE_FADE_FAR - vCamPos.y)/(TREE_FADE_FAR - TREE_FADE_NEAR))` with
  `TREE_FADE_NEAR = 250.0`, `TREE_FADE_FAR = 350.0`
  (`common/defines/00_graphics.lua:1068-1069`). At `cam_y = 348.06973`:
  `(350 - 348.06973)/100 = 0.019303` — matches the captured `vTreeFade = 0.019303`
  on **all 8 draws to |err| < 1e-4**. This closes the fade *interpolation shape*
  (linear) for trees, previously "C++-side" in the ledger.
- **`vSeasonLerp` / `vSeasonColumn` — CAPTURE.** Season-atlas blend factor and
  column index; the shader reads `SeasonMap` at `u = vSeasonColumn/8 + 1/16` and
  lerps `ColorMap`↔`ColorMapSecond` by `vSeasonLerp` (`:254`, `:342`). Both are
  **0.0** in this capture and bit-identical across all 8 draws (a single dated
  snapshot). The date→(column, lerp) function is CPU-side → **UNKNOWN**.
- `TREE_SEASON_MIN = 0.5`, `TREE_SEASON_FADE_TWEAK = 2.5`, `TREE_SPECULAR = 0.1`,
  `TREE_ROUGHNESS = 0.6` — FILE (`gfx/FX/constants.fxh:113,114,124,125`), consumed
  by the PS season/lighting path.

## Gate results (numeric)

| Gate (test) | Result |
|---|---|
| `gate_tree_scatter_chunk_law_vs_live_trees_measure` | **8/8** captured instance counts reproduced exactly from `map/trees.bmp` chunk law (chunks x 9..=11, z 5..=6, cam_y 348.06973); model order + byte-exact template + LOD bracket all match; **0 count error** |
| `gate_tree_season_cb1_vs_live_trees_measure` | **8/8** draws `vTreeFade == 0.019303` file law (|err| < 1e-4); `vSeasonLerp`=0.0, `vSeasonColumn`=0.0 bit-identical across draws |

Independent cross-check (Python, off-repo) of the 8 counts vs `map/trees.bmp`:
idx5 (10,6)=37 (10,5)=86 (11,6)=452 (11,5)=296; idx6 (10,6)=69 (10,5)=141
(11,6)=783 (11,5)=416 — identical to the captured draw instance counts.

## FILE / FIT / UNKNOWN table

| Parameter | Class | Evidence |
|---|---|---|
| One instance per nonzero `trees.bmp` cell | FILE | `trees.bmp` + 8/8 count gate |
| Per-chunk (256 wu) batching + draw order | FILE | corpus 8/8 draw match |
| Model = palette index → `trees.gfx` mapobject_N | FILE | byte-exact template gate |
| LOD shape = loddist bracket ≤ cam_y | FILE | LOD bracket gate |
| Transform math (anchor/yaw/slope + height jitter) | FILE | `tree.shader` VS source |
| Anchor **value** (cell center + h·0.1, flipped z) | FILE-hypothesis (ungated) | count-consistent; no per-instance ground truth |
| Slope **value** (heightmap gradient) | FILE-hypothesis (ungated) | same |
| Rotation **value** `vPos_YRot.w` | **FIT** | CPU-generated; in no file; `deterministic_tree_rotation` is a stand-in |
| `vTreeFade` | FILE | fade window `00_graphics.lua:1068-1069`; gated 8/8 <1e-4 |
| `vSeasonLerp`, `vSeasonColumn` | CAPTURE snapshot | 0.0/0.0; date→season map UNKNOWN |
| `TREE_SEASON_MIN/FADE_TWEAK/SPECULAR/ROUGHNESS` | FILE | `constants.fxh` |

## Blocked — exact next targets

The **anchor-value** and **rotation-value** gates are blocked on **capture, not
binary RE**: this corpus never recorded the tree per-instance vertex buffer.

1. **CAPTURE (prerequisite, ref ledger T-INST / CAP9):** re-capture `trees_pass`
   recording **IA slot 1** (the per-instance stream: `float4 vPos_YRot` +
   `float3 vSlopes`, stride 28) bound to the tree `DrawIndexedInstanced`. Today
   only slot 0 (template) is captured. With slot 1 in hand, the anchor/yaw/slope
   values become directly gate-able per instance.
2. **RE (only if slot-1 values ≠ cell-center + hash):** the hoi4.exe CPU routine
   that fills the tree instance buffer per chunk — specifically the per-instance
   **yaw RNG** (keying: cell coords? global index?) and whether the anchor is the
   exact cell center or intra-cell jittered. Owned by another lane's Ghidra
   project; out of scope here.
3. **Season:** the date→(`vSeasonColumn`, `vSeasonLerp`) mapping (CPU-side). Needs
   either a dated multi-season `trees_pass` capture or the RE of the season
   selector. Already tracked as UNKNOWN in FIDELITY_LEDGER §2.11.

No new recording-side data dependency was found (trees are static geometry; season
column follows the already-recorded date, and snow/season state is the
pre-existing UNKNOWN in the ledger).

## Changes in this lane

- `map_viewer/src/tree_files.rs`: factored `trees_measure_capture_dir()` and added
  the **preserved corpus** path so the scatter gate runs against the durable copy
  (the live `AppData` capture is gone); added
  `gate_tree_season_cb1_vs_live_trees_measure`.
- `map_viewer/src/tree_second_subdraw.rs`: added the preserved corpus paths to
  `evidence_session()` so the second-sub-draw gates run against the durable copy.

`cargo build -p map_viewer` clean; `cargo test -p map_viewer` = **619 lib passed /
1 ignored / 0 failed** + 13 integration passed / 0 failed. Both new/enabled tree
gates green. `rustfmt --check` clean on both edited files.
