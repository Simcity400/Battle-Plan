# Lane report — residual-audit (FIDELITY_LEDGER reconciliation)

Branch `lane/residual-audit`. Documentation/audit only: no code, no
`extractor/`, no `REPLAY_DATA_REQUIREMENTS.md`/`EXTRACTION_NEEDS.md` touched.
Files edited: `replay_interface/documents/FIDELITY_LEDGER.md` (§1 counts + §2.10/
§2.11/§2.12/§2.16 rows + NEW §2.19 + §3.0 burn-down) and
`replay_interface/documents/REPLAY_PRODUCTION_PLAN.md` §1 counts line.

## 1. New counts (LEDGER3, 2026-07-09)

| Column | Pre-wave (LEDGER2) | Post-wave (LEDGER3) | Δ |
|---|---:|---:|---:|
| FILE | 78 | **90** | +12 |
| CAPTURE | 24 | **24** | 0 |
| FIT | 38 | **34** | −4 |
| UNKNOWN | 46 | **49** | +3 |

Total map-render parameter rows: **197** (all four columns re-tallied against the
§1 layer table — each column sums exactly). **Plus 1 product-UX FIT tracked
outside the render sum**: `SPEED_STEPS_HOURS_PER_SEC=[1,2,4,12,48]` playback rate
(e-shell), explicitly non-parity.

Per-layer moves folded (recounted from the §2 rows, which already carried the
OBJ-1 + wave-2 text but whose §1 sums were stale):

| Layer | Old | New | Cause |
|---|---|---|---|
| Rivers/straits | 4/2/2/2 | 4/2/**1**/2 | B5 retired the fabricated per-class width table; class→width FIT→CAPTURE |
| Trees (T2) | 4/2/3/2 | **6**/2/**1**/2 | OBJ-1 closed template + model-selection + 2nd-sub-draw FITs; B4 closed the fade-shape caveat, added ungated anchor-value UNKNOWN |
| Buildings (B2) | 2/0/2/3 | **7**/0/**0**/3 | OBJ-1 retired Atlas GUESS + shipped `.mesh` decoder; B3 added meshsettings + material-texture-resolution FILE rows (city stays FILE-spec, UNKNOWN unchanged: level/damage + transform/LOD/variant + byte-parity) |
| UI chrome | 8/0/0/2 | **9**/0/0/**3** | D1/D2 added topbar binding-population FILE + number-formatting UNKNOWN |
| Naval supremacy tint | — (new) | **4**/0/**1**/**2** | C6 NEW §2.19 (was 2 scattered "naval=UNKNOWN" placeholders) |

The UNKNOWN column RISES on a net-advancing wave because monolithic
"naval = UNKNOWN" / "building render = UNKNOWN" placeholders were decomposed into
individually-closable, precisely-cited residuals — the intended direction.

## 2. Reconciliation table (residual → old → new → evidence → closure path)

| Residual | Old status | New status | Evidence citation | Closure path |
|---|---|---|---|---|
| River class→width ramp | FIT (fabricated 1.1..4.2 table) | **CAPTURE** | `river_world_synthesis::river_world_width_for_class` `width(k)=1.11+0.17·(k−3)`, gated `gate_river_world_comparison_against_captured_sessions` (k∈{3,4,6,7,9,10,11}, δ<0.02); `river.shader:198` proves width is VB-baked not file-derivable | RE the hoi4.exe stride-32 river-VB builder half-width lookup (binary, other lane) |
| River palette per-index 1/2 (flow-in/out) | (implicit in FILE palette row) | **UNKNOWN-in-files** (wiki convention) | `00_defines.lua:936-938` grounds small/large/not-river split + index-0=source (CAPTURE); 1/2 = HOI4-wiki only; `gate_full_map_river_extraction_vanilla` (11,534,336 px, 0 unclassified) | Official map-format doc or binary RE of VB-builder per-index branch |
| Tree fade interpolation SHAPE | FILE window / "shape C++-side" | **FILE (shape closed)** | `vTreeFade=saturate((350−camY)/100)` `tree.shader:408`; `00_graphics.lua:1068-1069`; gate `gate_tree_season_cb1_vs_live_trees_measure` 8/8 |err|<1e-4 | Closed |
| Tree per-instance yaw/rotation value | FIT | **FIT (confirmed, capture-blocked)** | `deterministic_tree_rotation` stand-in; `vPos_YRot.w` CPU-generated, in no shipped file | CAP9 / T-INST instance-VB capture (IA slot 1) |
| Tree per-instance anchor EXACT value | FILE ("Instance position") | **UNKNOWN (ungated hypothesis)** | transform math FILE (`tree.shader:200-258`); 8/8 count gate can't distinguish cell-center from intra-cell jitter; instance VB never recorded | CAP9 / T-INST instance-VB capture |
| Tree date→season (vSeasonColumn/Lerp) | UNKNOWN | **UNKNOWN (pinned)** | captured 0.0/0.0 bit-identical across 8 draws; date→(col,lerp) is CPU-side | Dated multi-season trees_pass capture or RE of season selector |
| Building `.mesh` geometry/material | FIT | **FILE** | `pdx_mesh.rs` decodes 436 meshes + 535 anims byte-clean (gate_a, 785 validation rows) — OBJ-1 | Closed for decode; render-pass parity CAP9 |
| Building `AtlasCoordinate`/`vUVAnimSpeed` | FIT (GUESS) | **FILE (dead bytes)** | `gate_building_cb1_atlas_uvanim_dead_for_standard` — `#ifdef ATLAS/UV_ANIM` compiled out for `PdxMeshStandard` (OBJ-1) | Closed (skinned variants out of building path) |
| Building meshsettings + material-texture resolution | (uncovered) | **FILE (NEW)** | B3 `gate_vanilla_building_material_textures_resolve` — 24 models, 93 refs, 93 resolved, 0 unresolved; `.gfx` meshsettings authoritative | Closed for file substrate; runtime SRV binding CAP9 |
| Building model→world transform / LOD / variant / byte-parity | (folded in "level/damage") | **UNKNOWN — CAP9 (formalised)** | model-space verts; engine placement/LOD-switch/variant switch not in files (B3 §6) | CAP9 `map_objects_pass` (transform matrix, bound LOD/mesh index, SRVs, variant) |
| Building levels / damage / control | UNKNOWN | **UNKNOWN — recording** | which slots built + damage state runtime (item 10) | Recording R1/R2 |
| City scatter placement | (n/a) | **FILE spec / UNKNOWN per-instance** | `map/cities.txt` density/step/distance params FILE; per-instance needs PRNG seed; no `cities_pass` capture | Capture request C-CITY |
| Naval sea-zone geometry / fill RGB / alpha bounds | UNKNOWN (2 placeholders) | **FILE** | C6 `00_graphics.lua:57-59`; `gate_vanilla_naval_region_parse` (98 naval regions / 3133 sea provinces, 0 orphaned) | Closed for data law; pixel parity capture-gated |
| Naval alpha curve SHAPE | UNKNOWN | **FIT (DERIVED, capture-gated)** | linear `lerp(min,max,clamp(sup/range))` interpreted from define comments; gate `supremacy_alpha_law` | Strategic-navy capture (C6 §5) |
| Naval supremacy magnitude unit (D56) | UNKNOWN | **UNKNOWN (offline-closable)** | fixture 0..1 vs `RANGE=2000` raw; logged REPLAY_DATA_REQUIREMENTS §5 / EXTRACTION_NEEDS A6 | Offline contract decision (inspect D56 emission) |
| Topbar number formatting | (uncovered) | **UNKNOWN — rendered raw** | `binding.rs` `format!("{}")` raw; no `.gui`/loc citation found (D1/D2) | Offline: find topbar loc/gui format string |
| Topbar/identity binding population | (uncovered) | **FILE (seam)** | `binding.rs` typed views→layout; gates G1 (12h×3 POV) + G2 (ideology flip) | Closed; view-path gaps below |
| Session player_countries (observer picker) | UNKNOWN (gap 26) | **UNKNOWN — replay_core view gap** | view reads `session.player_countries`; fixtures nest `world.scenario.players`→Absent (a3, e-shell) | Offline: add `world.scenario.players` fallback (world.rs:1907) |
| replay_core Absent view paths (convoys, world_tension, supply, ratio bars, air_superiority, war_relations, alerts) | (documented by a3/d-ui-bindings) | **UNKNOWN — view/fixture reconciliation** | a3 §"Documented gaps"; d-ui-bindings §"replay_core API gaps"; requirements §9b already enumerates | Offline: reconcile view path ↔ fixture-gen key / add typed view fields |
| Capture instrument: camera-write / console-dispatch | (open) | **fail-closed (roots identified)** | capture-re Roots 1/2: `camera_object_unresolved`; console arg-vector/main-thread marshal unresolved | RE next steps in `lane_reports/capture-re.md` |

## 3. Unlabelled-constant sweep (part 4) — CLEAN

Grepped every module the wave touched for numeric constants lacking a
FILE/CAPTURE/FIT/UNKNOWN provenance. **Zero unlabelled FITs found.**

| Module | Constants checked | Verdict |
|---|---|---|
| `naval_supremacy_tint.rs` | `VANILLA_NAVAL_TINT_DEFINES` (0.5/0.8/2000.0/stripe colours) | FILE-cited to `00_graphics.lua:57-59,37-39`; alpha curve explicitly `DERIVED` (`:349-350`); vanilla values are the gate reference, resolver re-parses the shipped file |
| `transport.rs` | `SPEED_STEPS_HOURS_PER_SEC=[1,2,4,12,48]`, `DEFAULT_SPEED_STEP=2` | Labelled **FIT (product UX)** `:12-24`, explicit "makes no parity claim" |
| `river_mesh_synthesis.rs` | source-strip width ramp, `river_width_for_class` | Labelled CAPTURE `:12-17`; the retired `RIVER_SOURCE_WIDTH_RAMP_STEPS=11.0` FIT is gone; unclassifiable polyline fails closed |
| `river_world_synthesis.rs` | `width(k)=1.11+0.17·(k−3)`, palette predicates | CAPTURE-measured + FILE palette (`00_defines.lua:936-938`); `RiverPixelKind` labels 1/2 UNKNOWN-in-files |
| `tree_files.rs` / `tree_instance_synthesis.rs` | `TREE_CHUNK_WORLD_UNITS=256.0`, `TREE_FADE_NEAR/FAR=250/350`, stride 28/56 | 256 labelled "capture-proven 8/8 exact" (`tree_instance_synthesis.rs:13-32`); fade window FILE; strides CAPTURE |
| `binding.rs` | (no magic numbers) | Every scalar rendered raw with an explicit UNKNOWN-formatting header `:12-31` |
| `building_files.rs` / `map_static_data.rs` | (no float literals; scale = entity.scale × pdxmesh.scale) | All placement/scale values file-sourced; grep for `= N.N` returned zero hits |

## 4. Burn-down queue — top 5 offline-closable (seed the next wave)

Closure cost order: offline-file < fixture/view < RE-lane < capture. These 5 need
**no live capture and no binary RE**:

1. **replay_core topbar view-path reconciliation** — convoys / world_tension /
   supply / ratio-bars read a different path than the fixture writes. Blocks
   topbar population (D1/D2). Owner: replay_core view + fixture-gen.
2. **replay_core `session.player_countries` fallback** (`world.scenario.players`,
   world.rs:1907). Blocks the E6 observer POV picker — the user's core
   observer-mode UX. Owner: replay_core.
3. **Topbar number-formatting law** — find the shipped `.gui`/localisation format
   citation (percent ×100, K/M grouping, slash-join, nukes total-vs-ready). Blocks
   HOI4-parity topbar. Owner: offline file dig.
4. **replay_core diplomacy/air view surfaces** — `war_relations`,
   `air_superiority`, topbar `alerts.*` have no typed view. Blocks diplomacy
   panel, air-supremacy mode, alert badges. Owner: replay_core add typed fields
   (requirements §9b already lists them).
5. **Naval supremacy magnitude unit (D56)** — pin normalised-vs-raw by inspecting
   the extractor D56 emission. Blocks engine-correct naval alpha. Owner: offline
   contract decision.

**Next tier (RE-lane / capture, not offline):** tree instance anchor/yaw
(CAP9 instance-VB); building transform/LOD/variant + object-pass parity (CAP9
`map_objects_pass`); naval alpha curve shape + sea-zone pixel parity
(strategic-navy capture); tree date→season map (RE / dated capture); river
per-index 1/2 flow + width→FILE (RE / official doc). Capture-instrument enablers
(camera-write, console-dispatch) remain fail-closed per capture-re.

## 5. Notes for the orchestrator

- b5-rivers had already written its §2.10 rows; this lane left them and only
  refreshed the §1 count + burn-down. All other 7 lanes had NOT updated the
  ledger — their rows are now reconciled.
- OBJ-1 (mesh decoder / atlas-GUESS retire / tree template + model-selection
  closures) was folded here too: its §2 row text had landed but the §1 sums were
  stale, so the accurate recount necessarily reflects it.
- No git commit made (orchestrator commits). No code / extractor / requirements
  files touched.
