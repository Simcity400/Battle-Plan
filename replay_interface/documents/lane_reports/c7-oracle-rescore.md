# LANE c7-oracle-rescore — RenderDoc close-zoom parity rescore (plan item C7)

Branch `lane/c7-oracle-rescore`. Scope: re-run the RenderDoc-oracle parity
scoreboard after this session's shadow/PCF (b6), buildings (b3), trees (b4),
rivers (b5), rails (c5), occupation-stripe, and camera laws landed, and report
the NEW mean|err| across the zoom ladder — especially at **h=50** — vs the prior
baseline (0.014–0.03 at h>500, climbing to **0.135 at h=50**).

**Measurement + honest-reporting lane, not a new-law lane.** No git commit; no
`extractor/`/`replay_core`/fixture edits; no module renames. Docs updated:
`FIDELITY_LEDGER.md` §2.20 (new), `REPLAY_PRODUCTION_PLAN.md` §1 oracle note +
C7 row.

---

## Headline

**h=50 scoreboard mean|err| is UNCHANGED at 0.135.** None of this session's
new close-zoom laws are wired into the scoreboard's render path, so the oracle
output is byte-unchanged from the documented baseline. The h=50 gap is now a
**WIRING** gate, not a new-law gate.

Two hard facts block a fresh per-height table and drive the finding:

1. **The mined ladder is gone.** `%LOCALAPPDATA%\hoi4_renderdoc\mined` currently
   holds exactly **one** frame directory — `hoi4_vanilla_frame9232` at MainScene
   cam-height **h≈50.04** (the C7 definition-of-done frame) — plus 22 HDR blobs
   and a `shaders` dir. The 38-frame zoom ladder and any prior `scoreboard.csv`
   are **not present**. A full-ladder table cannot be regenerated from this store.
2. **The new laws are law-only, unwired from the render path.** Every one of the
   7 new modules is `mod` + `pub use` in `map_viewer/src/lib.rs` and nowhere else
   in the render path.

---

## Scoreboard table — baseline vs now

| Height band | Baseline mean\|err\| (plan §1 / renderdoc-oracle note) | Now (this rescore) | Why |
|---|---|---|---|
| h > 500 (far / world) | 0.014 – 0.03 | **unchanged** (not re-measurable — no mined frame at these heights in the current store) | No new law touches the captured-pass render output. |
| h = 50 (close zoom) | **0.135** | **0.135 (unchanged)** | frame9232 present at h≈50.04, but the render path excludes all new close-zoom laws → identical output by construction. |

*No live GPU re-render was performed.* The scoreboard invokes
`replay_app --capture live_PAN2 --bundle …/out/map_bundle --dump-hdr`, which
requires a built `out/map_bundle` (absent in this worktree), a live D3D11 GPU
render, and the mined blob. Because the render path provably excludes every new
law (below), a successful re-render would reproduce **0.135**, not exercise the
new laws — so it would not answer C7's question. Per the lane's no-eyeball rule
the number is reported from the code-path audit, not fabricated.

---

## Which new laws ARE / ARE NOT reflected in the scoreboard

The scoreboard drives the **captured-pass replay** (`replay_app`
`CapturedPassLayer` → `NativeD3d11TerrainRenderer::render_prepared_production_map`).
Its render graph (`d3d11_replay.rs::prepare_production_map_with_layers_for_reference`)
has exactly these layers: **terrain, water, rivers, borders, trees** — all
replayed from the captured `live_PAN2` draw bundles.

Grep audit (references outside each module's own file + tests):

| New law (module) | Plan item | In render path? | Evidence |
|---|---|---|---|
| Shadow / PCF (`shadow_law`) | B6 | **NO** | Only ref is `lib.rs` (`mod` + `pub use`). Render path's sole "shadow" is the pre-existing COL3 shadow-fade height law in the political scene CB0 — **not** the B6 PCF map. |
| Buildings (`building_files`) | B3 | **NO** | `lib.rs` only. No `with_buildings` pass; buildings stay CAP9-blocked. |
| Rails (`rail_files`) | C5 | **NO** | `lib.rs` only. No rail draw pass. |
| Occupation stripes (`occupation_stripe_law`) | — | **NO** | `lib.rs` only. |
| Tree scatter (`tree_instance_synthesis`) | B4 | **NO** | `lib.rs` (`pub(crate) use`) only. The trees the scoreboard renders are the **captured** PAN2 trees pass, not the B4 file-derived scatter. |
| Camera interp (`camera_interp_law`) | — | **NO** | `lib.rs` only. (The scoreboard uses the CAM1 measured tilt table baked into `camera.rs` / the scoreboard's own `dz()`.) |
| Snow / mud feed (`snow_mud_feed`) | c-weather | **NO** | `lib.rs` only. Cannot faithfully feed the winter frame9232. |
| Rivers world-synthesis (`river_world_synthesis`) | B5 | partial (synthesis side) | Referenced by `river_mesh_synthesis` + `d3d11_replay.rs::synthesize_river_world_indices` (world-synthesis path). The scoreboard's `--capture` path still uses the **captured** rivers bundle, so B5 synthesis is not what the scoreboard measures either. |

The b6-shadows lane report states this in its own words: the modules are "all
`mod` + `pub use` in `map_viewer/src/lib.rs`" and "gates are hand-computed
shader-reference values + resolver FILE-citation substring gates … **NOT** pixel
validation of runtime textures."

---

## The remaining h=50 gap and what closes it

The 0.135 residual at h=50 is the **unrendered dynamic/close-zoom content**:
units, buildings, rails, ground shadows, occupation stripes — none of which the
captured-pass replay draws (they are not among terrain/water/rivers/borders/trees,
and PAN2's bundle set has no buildings/objects/rail pass loaded).

**Closing it is WIRING, not new law:**

1. Add draw passes to `ProductionMapPassGraph` / `d3d11_replay.rs` for the
   already-derived laws: a shadow-map pass (b6), a buildings/map-objects pass
   (b3, still needs the CAP9 object-pass capture for the runtime model→world
   transform), a rail ribbon pass (c5, needs CAP9 ribbon width/UV), an occupation
   stripe layer, and the b4 file-derived tree scatter (to replace the captured
   trees on the decoupled path). Each needs bundle/synthesis plumbing + a
   `with_*` layer, mirroring the existing `with_rivers_capture` /
   `with_trees_capture` wiring.
2. Feed `snow_mud_feed` into the terrain scene CB so winter frames match.
3. Make the **particles descope decision** (C7's explicit prerequisite). If
   descoped, restate the h=50 target on particle-free frames or as a documented
   delta.
4. **Re-mine the zoom ladder** (only h≈50 frame9232 survives) so a per-height
   table can be produced, and gate base-terrain parity on a non-snow (h≥288)
   frame per the winter confound.

Until (1)–(3) land, the oracle number **cannot** move: the definition-of-done
metric for Phase C is currently pinned at the baseline by the absence of wiring,
not by any missing law.

---

## Winter confound (documented, respected)

frame9232 (h=50) is **winter** ground (mined ground frames h≤250 are winter).
`snow_mud_feed` exists but is itself unwired from the render path, so a faithful
`SnowMudData` input cannot be reconstructed into the scoreboard render even if it
were re-run. Per the lane rule, snow inputs were **NOT** faked to improve the
score; the h=50 base-terrain diff stays winter-contaminated until either
`snow_mud_feed` is wired + the engine snow state reconstructed, or a non-snow
(h≥288) frame is re-mined.

---

## Build / test

- `cargo build -p map_viewer` — **green** (23 pre-existing warnings, 0 errors).
- `cargo test -p map_viewer` — **718 passed / 0 failed / 1 ignored** (698 lib +
  20 integration). No code changed in this lane; docs only.
