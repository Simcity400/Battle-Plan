# RENDER-PATH VANILLA-LITERAL AUDIT (LEDGER2)

Mod-compatibility mandate (`MOD_COMPATIBILITY.md`): every game-file read in a
render path must go through the mod-aware `GameFileResolver`/`mod_resolver`; no
1936-scenario literals (tags, country names/colors, start dates, capital ids) may
sit in rendering/synthesis code; every numeric constant in render code must carry
a citation comment or originate in a `*_law.rs` module.

Scope swept: `replay_interface/native/map_viewer/src/**.rs` and
`replay_interface/native/replay_app/src/**.rs` (production render + synthesis +
law modules; `src/bin/dump_*` diagnostics and `#[cfg(test)]` blocks classified
test-only). Originally read-only; remediation below records the applied fixes.
Findings independently verified by
LEDGER2 (each `font.rs`/`constants.rs` hit opened and confirmed) and adversarially
reconciled with the local Codex CLI (read-only, `model_reasoning_effort=xhigh`).

**Headline:** the render path is overwhelmingly resolver-clean. Every
`gfx/ map/ common/ history/` string in production is a *relative-path constant*
handed to `GameFileResolver` (`resolve_file`/`resolve_required`/
`resolve_dir_files`); every production `fs::read` reads either a resolver-produced
path or a pre-resolved bundle/capture artifact (`manifest.json`, `terrain_runtime`,
capture `.bin`). **Zero** country-tag / 1936-scenario literals reach production
render math. The two original low/medium findings are now closed.

---

## Fix queue (sorted by severity)

| # | Sev | file:line | Category | What | Proposed fix | Risk |
|---|---|---|---|---|---|---|
| 1 | **CLOSED** | `replay_app/src/font.rs` | Hard-coded vanilla path / proxy overlay font removed | The machine-specific `DEFAULT_VANILLA_GAME_DIR` and the `ToolTip_Font` TGA proxy path were removed. `load_overlay_font` now accepts only an active `GameFileResolver`, loads the real DDS-backed `hoi_18mbs`, and fails closed on missing/invalid evidence. Overlay construction omits text when `--game`/`BATTLE_PLAN_VANILLA` is absent instead of using a local vanilla fallback. | Keep closed; do not reintroduce a personal path or proxy font. | none |
| 2 | **CLOSED** | `map_viewer/src/constants.rs:5-143`; `map_viewer/src/cpu_terrain.rs:1823-1830`; `map_viewer/src/frame_bundle.rs:115` | Uncited numeric render params | Shader-parameter/color/geometry constants now carry source comments back to shipped `gfx/FX/constants.fxh`, `gfx/FX/standardfuncsgfx.fxh`, `gfx/FX/fow.fxh`, `gfx/FX/tiled_pointlights.fxh`, and `common/defines/00_graphics.lua`. The one non-`static const` group (`CalculateOccupationMask` camera fade and stripe literals) is cited to `standardfuncsgfx.fxh:965-976` and validated at bundle load by `validate_occupation_mask_shader_source`, so formula drift fails closed. | Keep closed; do not add new render constants without citation comments or a law/parser gate. | none |
| 3 | **CLOSED** | `replay_app/src/render.rs`; `replay_app/src/political_scene.rs` | Runtime vanilla constants fallback removed | The captured-pass renderer no longer uses baked vanilla political scene CB0 values when no resolver is present. It requires `--game`/`BATTLE_PLAN_VANILLA` so `common/defines/00_graphics.lua` supplies the outline cutoff and shadow fade constants. The vanilla literal helper is test-only. | Keep closed; product-visible draw state must be resolver-read or capture-read. | none |
| 4 | **CLOSED** | `replay_app/src/main.rs`; `map_viewer/src/d3d11_replay.rs`; `map_viewer/src/underwater_pass.rs` | Product-visible debug env knobs blocked | `replay_app` now rejects `REPLAY_DEBUG_DUMP_UNDERWATER`, `REPLAY_DEBUG_CLEAR_BEFORE_WATER`, `REPLAY_DEBUG_UNDERWATER_INPUTS`, `REPLAY_DEBUG_CONSTANT_REFRACTION`, and `REPLAY_DUMP_LABELS` before startup reaches rendering. These remain lower-level research knobs only, not final-app state. | Keep closed; product render mutations must be explicit engine/file laws or captured data. | none |
| 5 | **CLOSED** | `replay_app/src/main.rs`; `replay_app/src/render.rs` | Product terrain chunk synthesis blocked | `replay_app` now rejects `--synthesize-terrain-chunks` and `--chunks-from-files`, and the product renderer no longer carries the synthesized prepared-map branch. Chunk mesh/LOD work remains lower-level `map_viewer` tooling until mixed-LOD `vSnap` and dependent runtime metadata are engine-derived. | Keep closed; do not reintroduce generated chunk upload into `replay_app` before the runtime/LOD law is complete. | none |
| 6 | **CLOSED** | `replay_app/src/main.rs`; `replay_app/src/timeline.rs`; `replay_app/src/app.rs` | Product synthetic timeline / clock overrides removed | `replay_app` now rejects `--demo-scrub` instead of constructing non-replay owner changes, removes the synthetic scrub provider, rejects `--freeze-clock`, and no longer injects app wall-clock seconds into the captured map scene. Runtime map-clock CB bytes are preserved (`map_clock_time = None`) until a real engine/capture timing law exists. | Keep closed; no synthetic timeline or app-owned animation clock may enter the final replay surface. | none |

**Verification of finding 2 (values are file-correct, only the citation is missing):**
- `constants.rs:14 GMT_OFFSET=2793.0` == `00_graphics.lua:934` (`GMT_OFFSET = 2793`) and `standardfuncsgfx.fxh:323`.
- `constants.rs:23 MUD_NORMAL_CUTOFF=10.982` == `constants.fxh:62`.
- `constants.rs:11 NIGHT_AMBIENT_BOOST=3.0` == `constants.fxh:14`.
- `constants.rs:39-58 DAY_AMBIENT_MAP_POS_X=(0.1,0.1,0.05)` … == `constants.fxh:17-22` (`DayAmbientMapPosX = float3(0.1,0.1,0.05)` …) — note these are the shader's `DayAmbientMap*`, NOT the `00_graphics.lua:1053 AMBIENT_LIGHT_*` rig (a common confusion; they are distinct).
- `constants.rs:12/13 NIGHT_OPACITY=0.85 / NIGHT_DARKNESS=0.5` == `standardfuncsgfx.fxh:328/329`.

Because the values were already file-correct, finding 2 was a *provenance-hygiene*
fix, not a fidelity bug. It is closed so the mod-compat mandate's "every render
constant is citation-traceable" is literally satisfied.

---

## (i) Hard-coded vanilla paths bypassing the resolver

Only finding #1 above. Everything else resolves. Confirmed non-violations (all
route through `GameFileResolver`, relative-path constants):
- `bundle_gen.rs` (all `map/` `common/` `gfx/` sources → `resolve_file`), `border_world_synthesis.rs`, `building_files.rs`, `border_pass.rs:85`, `border_law.rs:293-298`, `bloom.rs:295/305`, `grading_law.rs:51-52,690-707`, `replay_app/main.rs:94-101`, `constants.rs:522-529,608-624` (source-manifest path lists).
- Direct `fs::read` reading resolver output: `bloom.rs:296,306`; `grading_law.rs:539,551`; `seasons.rs:310`; `map_text.rs:44,88`; `map_static_data.rs:316`.
- Bundle/capture-artifact opens (pre-resolved, allowed): `render.rs:791,800`; `bundle.rs:78,181,230,252`; `cpu_terrain.rs`, `frame_bundle.rs`, `terrain_chunk_library.rs`, `restore_evidence.rs`.
- `*_from_game_dir` public wrappers (`seasons.rs:303`, `map_static_data.rs:59`, `snow_law.rs`, `pdx_mesh.rs`, `font.rs`) build `GameFileResolver::vanilla(...)` but are doc-commented "Test-only convenience wrapper; production callers pass a `GameFileResolver`" and still resolve. **Not violations**; flagged misuse-prone — a future clippy/lint guard on `GameFileResolver::vanilla(` outside `#[cfg(test)]` would harden this.
- **No `include_bytes!`/`include_str!` of shipped game content** exists in either dir.

## (ii) 1936-scenario / country-tag literals

**Zero production violations.** Every 3-letter tag literal (`"GER"/"SOV"/"USA"/…`)
lives inside `#[cfg(test)] mod tests` or a `src/bin/dump_*` diagnostic:
`map_label_law.rs`, `political_color_synthesis.rs`, `map_text.rs` (map_viewer
tests); `bundle.rs`, `secondary_field.rs`, `palette.rs`,
`gb_field.rs`, `timeline.rs` (replay_app tests); `dump_map_labels.rs:177`
(diagnostic). `"1936"` tokens are data-model/bundle field names
(`state_owner_1936.json`, `Static1936`, timeline test dates), not scenario
literals in render math.

## (iii) Uncited numeric magic constants

No open findings. The formerly bare `constants.rs:5-143` block now carries source
comments, and `CalculateOccupationMask` literals are source-gated against
`standardfuncsgfx.fxh:965-976`. The `*_law.rs` modules
(`border_law`, `water_law`, `light_law`, `snow_law`, `luminance_law`,
`map_label_law`, `border_texture_law`, `terrain_mesh_law`, `grading_law`) are the
citation-bearing origin modules and are clean. Other hex/float hits are documented:
FNV constants (`gb_field.rs:81-82`), measured capture colors (`0x5c5c5c` at
`palette.rs:16`, `bundle.rs:46`, `gb_field.rs:326` — doc-commented "measured in
live capture"), and the `DXGI_FORMAT_*`/`D3D11_*` SDK-enum block
(`constants.rs:184-230`, self-evident DirectX values — excluded).

---

## Grep patterns (rerun to re-audit; zero-violation categories list their proofs)

Run from `replay_interface/native`, over `map_viewer/src` + `replay_app/src`, `*.rs`.

**(i) Hard-coded paths / direct opens:**
```
(gfx/|map/|common/|history/|localisation/|interface/)          # relative game paths → check each flows to resolver
fs::read|fs::File|File::open|File::create|include_str!|include_bytes!|read_to_string|std::fs
[A-Za-z]:\\|SteamLibrary|Hearts of Iron IV|Paradox Interactive|C:/Users|F:/Steam   # absolute/personal paths
GameFileResolver::vanilla|from_game_dir|BATTLE_PLAN_VANILLA            # resolver-construction sites (audit outside #[cfg(test)])
```
Sole production absolute-path hit: `font.rs:38` (finding #1). All others are
`#[cfg(test)]` consts, doc comments, or capture-evidence paths.

**(ii) Tags / dates (zero production hits — proof patterns):**
```
"[A-Z]{3}"          # 3-letter tags — every production hit is inside #[cfg(test)] or src/bin
1936|1939|1918|1937 # start dates — every hit is a data-model field/filename, not render math
```

**(iii) Uncited constants (proof patterns):**
```
0x[0-9a-fA-F]{6,8}|Color\s*\{|\[\s*0?\.[0-9]|0\.\d+f?,\s*0\.\d+f?,\s*0\.\d+   # color/float literals
constants\.fxh|00_graphics\.lua|// cite|measured|law\.rs                      # citation markers (absence over constants.rs:5-143 = finding #2)
```

## Codex reconciliation

The audit sub-agent ran one read-only Codex one-shot (xhigh); Codex found **no
missed** direct game-file open and **no missed** tag/1936 violation across either
tree, and independently confirmed `font.rs:38` is the only production hard-coded
vanilla install path. It made two accepted refinements: (1) `font.rs` routes
through the resolver and does not override an active modset (so it is a
"hard-coded personal vanilla fallback," not a "resolver/modset bypass"; now closed); (2) scope
finding #2 to `constants.rs:5-143` and exclude the `DXGI_*/D3D11_*` SDK-enum block.
It disputed no classification. It additionally suggested a lint guard on
`GameFileResolver::vanilla(` outside test code as a future hardening.
