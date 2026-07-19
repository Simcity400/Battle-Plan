# Overnight capture session — 2026-07-03 (autonomous)

Status report from the unattended session. TL;DR: **the map-load freeze is fixed and
proven live** — the game now loads into the map with the capture DLL injected, every time.
Terrain and water are captured and validated; all full-map input textures and the
feature-pass shaders are captured. The remaining feature *frame bundles*
(rivers/borders/trees) are blocked on a newly-root-caused DLL pass-close bug, which a
Codex job is fixing as of this writing.

## What is committed
- `05b54a3d6` — hook hot paths lock-free (context hooks: 842 lock acquisitions across
  14.27M calls; create-pixel-shader: 95/4704) + harness evidence-alias fix + pre-filter
  sweep test over real captured bytecode (0 false negatives). Verified independently
  (tests/fmt/check, both harness profiles, diff scope). Includes a new
  `terrain_capture/.gitignore` so run artifacts (`/harness_runs/`, `/out/`) are never
  committed again.
- `1f15afb4f` — the original load-freeze fix (non-map shaders reject before lock/reflect).

## Captured data (on local disk, non-OneDrive: `C:\Users\Example\AppData\Local\hoi4_captures\`)
Save: continue-last-save = Ireland 1943 (`IRE_1943_11_09_17`). Settings verified good:
`multi_sampling=0` (no MSAA rejection), `draw_postfx=no`, windowed.

Best complete set in `live_session\` (all-moments run before budget trip):
- **TerrainIDMap** 5632x2048 fmt=87, FNV `f493368c56224ab1` — **byte-identical to the
  previously-validated reference** in the terrain_capture README. Confirmed good.
- **Water**: `map_frame_water_pass_bundle.json` 0 blockers; `pdxwater` shader captured.
- **Terrain**: `map_frame_bundle.json` sequence_complete, 0 blockers.
- Full-map input textures (camera-independent, whole-map): gradient_border_channel1 &
  channel2 (2816x2050, both FNV `089afb7c652d0325` — NOTE: identical hash for both
  channels, worth confirming that is real and not a capture aliasing quirk),
  province_secondary_color_map (2816x1024 `b3df2c5b99e22325`), snow_mud_data (1408x512),
  light_index_map (64x64), light_data_map (128x1).
- Feature-pass **shader bytecode** (camera-independent): terrain (pdxmap, 2 variants),
  water (pdxwater), rivers, trees (2 variants). Vertex shaders incl. map/pdxmap.

Missing / incomplete:
- **rivers/borders/trees frame bundles** — see blocker below.
- **borders shader** — never observed even with `borders_pass` armed (rivers/trees/water
  were). Likely borders render *inside* the terrain/pdxmap pass (gradient_border_apply),
  not as a separate pass. Codex job 20 is confirming.
- **terrain_chunk_library** — only ~7 chunks / 2.3% coverage. Needs a full-map camera pan,
  which requires camera control (unavailable this session — see below).

## Root causes found this session
1. **Load freeze (FIXED, proven live):** create-pixel-shader hook ran D3D reflection ~5x
   per shader under the global lock during the game's load-time shader storm. Now rejected
   lock-free. Game reaches the map every launch (session 8 froze at 19 events with zero
   work; now hundreds of events and full map render).
2. **Per-frame work budget (2000ms) is NOT a TDR guard** — it is cumulative CPU readback
   time across a frame. Safe to raise; raised to 12000ms and per-frame work peaked at
   9718ms with no trip. Higher budget just means a longer capture stutter (the game
   recovers, confirmed).
3. **Feature-pass runaway (rivers/borders/trees) — BLOCKER, Codex job 20 fixing:** these
   passes *blend* onto the terrain render target and the capture never reaches
   `sequence_complete` — the close condition "waiting for next render-target clear frame
   boundary" never fires live (there is no clear between the blend pass and the next
   pass/Present). So the pass re-captures a 16.5MB output-merger render target *every
   frame* and accumulates draws until the session byte budget trips → 2 persistent
   blockers ("still active…" + "post-pass render target payload missing"). The terrain
   pass closes fine because its boundary condition IS met live. The offline harness did
   NOT reproduce this (its rivers phase closes cleanly) — that fidelity gap is part of
   the Codex job.

## Constraint discovered: no camera control
The computer-use access grant cannot resolve the game (Steam-launched exe, not a Start-menu
app), so I could not pan/zoom the map. Impact is limited: all full-map input textures and
all pass shaders are camera-independent and were captured at the default view. Only the
**chunk library** (per-region terrain geometry) needs a full-map pan for complete
coverage. To finish it, a human-driven session (or a resolvable launch path) is needed.

## Update — blend-pass close (commits fe70b6f2c, + job 21 in flight)
- `fe70b6f2c` (Codex job 20, verified): blend passes (rivers/borders/trees) now close
  deterministically instead of running away to the byte budget. **First fully-clean live
  feature capture achieved** — terrain+rivers+trees finished `captured` (not
  budget-exceeded), all bundles `sequence_complete` 0 blockers.
- **Rivers: DONE and proven correct** — the live rivers pass is a single distinct draw
  per frame (the 23 accumulated copies in `live_R_rivers` are byte-identical), captured
  cleanly.
- **Trees: 1 of 2 sub-draws captured; proper fix (job 22) in flight, live-gated.** The
  live trees pass is TWO draws per frame using TWO different pixel shaders
  (`tree_pixel_shader_277b7b5e...` and `...6b77b6df...`, same tree mesh). The job-20
  min-count close (min=1) captures only the first (277b7b5e). Two dead ends, both caught
  by LIVE verification (not harness):
  - min=2: does NOT align to frame boundaries — captured 277b7b5e TWICE (across two
    frames) and still missed 6b77b6df. Reverted.
  - Job 21 (run-end close via PS-transition/present/clear): those boundaries DO NOT FIRE
    live for blend passes, so it REGRESSED rivers back to the runaway budget trip. Its
    synthetic harness gave a false PASS. Reverted.
  The proven-live boundary is FRAME-REPEAT (these passes re-issue byte-identical draws
  every frame): close when an incoming blend draw matches the pass's first accumulated
  draw. Job 22 (dispatched) implements this; it will be committed ONLY if a live capture
  confirms rivers=1 draw (no runaway) AND trees=2 draws with two DISTINCT tree shaders.
  If it does not cleanly verify live, the stable job-20 state stands and trees remains
  1-of-2 for a supervised follow-up (spec at scratchpad cdx_job22_trees_framerepeat_SPEC).
- **Borders: confirmed NOT a separate pass** — border shader never observed; borders are
  integral to the terrain/gradient-border path (`GradientBorderChannel1/2` are in the
  terrain bundle). No separate borders bundle is expected.

## Morning update — user-assisted sessions (commits fe70b6f2c, 1f4421288)
- Job 21 (run-end close) and a trees min=2 variant were BOTH REJECTED at the live gate
  (job 21 regressed rivers to the runaway; min=2 captured the same tree sub-draw twice
  across frames). The offline harness produced false PASSes in both cases — live capture
  is the only trustworthy gate for close-boundary changes. Trees remains 1-of-2
  sub-draws (job-22 spec exists for a content-identity frame-repeat close; also failed
  live once — needs a supervised retry).
- Chunk library, two user pan sessions + a decisive Japan-hold probe: the renderer
  RE-BASES its coordinate space into a local window — with the camera at Japan, recorded
  chunks AND the CB0 vCamPos decode to central-Europe coordinates; only 4 distinct CB0
  payloads appeared across a whole-map pan (payload caching also reuses stale CB0
  snapshots). Result: chunk library is capped at ~16% world coverage BY DESIGN of the
  game's renderer, not by pan quality. Committed (1f4421288): capture stays pending until
  coverage/dwell, additive world_quad field, honest limitation note. MORE PAN SESSIONS
  CANNOT HELP until the window re-basing math is derived offline (all needed payloads are
  in live_PAN2). Alternative worth evaluating first: replay chunks with their per-frame
  CB0 (camera-following window, like the game itself) instead of forcing world-absolute
  placement.

## RESOLUTION — chunk coverage dissolves into template synthesis (post-analysis)
Offline analysis of the PAN2 records settled the mechanism and removed the need for any
further pan sessions:
- All 44 chunk draws share ONE index buffer (draw_count 24576) and 4 vertex-buffer
  variants — a TEMPLATE GRID MESH at 3 LOD tile sizes (512/256/128 world units),
  positioned solely by CB1 QuadOffset_Scale (pdxmap.fxh ConstantBuffer(1,32)).
- Every PS texture slot is a single GLOBAL world texture across all records
  (TerrainDiffuse, HeightNormal, TerrainColorTint, TerrainIDMap 5632x2048,
  ProvinceSecondaryColorMap, GradientBorderChannel1/2, etc.) — all already captured
  byte-exact. Payload files verified fresh (44/44 identity-fnv == file-fnv), so this is
  not a stale-cache artifact.
- The clustered offsets + marching-grid record order during a STATIONARY camera hold
  show these draws are a background tile bake, not camera-frustum geometry — hence pan
  coverage was never the right lever.
- REPLAY PLAN: synthesize the full-world chunk set in the replay app by iterating
  QuadOffset_Scale over the map per LOD, patching CB1 exactly as TerrainSceneCB0 is
  already patched, and issuing the captured template draw with the captured global
  SRVs. Chunk "coverage" is thereby complete by construction; validate a synthesized
  tile byte-exact against a captured record with the same offset (44 references
  available in live_PAN2).

## Next steps
1. Offline: implement chunk template synthesis in map_viewer/replay_app (CB1
   QuadOffset_Scale patching over the captured template draw); validate synthesized
   tiles byte-exact against same-offset captured records.
2. Trees 2nd sub-draw: supervised retry of the frame-repeat close with content identity.
3. Offline: validate the GPU gradient-border generator against the live
   gradient_border_channel captures (2816x2050, FNV 089afb7c652d0325 — both channels
   share that hash, confirm); integrate captured shaders/textures into replay_app.
