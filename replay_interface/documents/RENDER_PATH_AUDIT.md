# RENDER-PATH VANILLA-LITERAL AUDIT (LEDGER2)

Mod-compatibility mandate (`MOD_COMPATIBILITY.md`): every game-file read in a
render path must go through the mod-aware `GameFileResolver`/`mod_resolver`; no
1936-scenario literals (tags, country names/colors, start dates, capital ids) may
sit in rendering/synthesis code; every numeric constant in render code must carry
a citation comment or originate in a `*_law.rs` module.

Scope swept: `replay_interface/native/map_viewer/src/**.rs` and
`replay_interface/native/replay_app/src/**.rs` (production render + synthesis +
law modules; `src/bin/dump_*` diagnostics and `#[cfg(test)]` blocks classified
test-only). Read-only; no code changed. Findings independently verified by
LEDGER2 (each `font.rs`/`constants.rs` hit opened and confirmed) and adversarially
reconciled with the local Codex CLI (read-only, `model_reasoning_effort=xhigh`).

**Headline:** the render path is overwhelmingly resolver-clean. Every
`gfx/ map/ common/ history/` string in production is a *relative-path constant*
handed to `GameFileResolver` (`resolve_file`/`resolve_required`/
`resolve_dir_files`); every production `fs::read` reads either a resolver-produced
path or a pre-resolved bundle/capture artifact (`manifest.json`, `terrain_runtime`,
capture `.bin`). **Zero** country-tag / 1936-scenario literals reach production
render math. **Two** findings survive, both low/medium severity.

---

## Fix queue (sorted by severity)

| # | Sev | file:line | Category | What | Proposed fix | Risk |
|---|---|---|---|---|---|---|
| 1 | **MED** | `replay_app/src/font.rs:38` | Hard-coded vanilla path | `const DEFAULT_VANILLA_GAME_DIR = r"<local-path> of Iron IV"` — a machine-specific personal Steam path baked into production. Used by `default_vanilla_game_dir_if_present()` (`:284-287`) which feeds the overlay-font fallback in `load_overlay_font(None)` (`:251-258`). | Remove the baked literal. Require `--game`/`BATTLE_PLAN_VANILLA` (or a platform Steam-library probe) for the overlay-font fallback. The path is non-portable and forces vanilla-only in the no-resolver branch. | med — production reachable, but degrades to a clear error on other machines; does NOT bypass an active modset (`main.rs` `build_game_file_resolver` errors if `--mod`/`--use-dlc-load` is passed without `--game`, so it never silently overrides a mod). |
| 2 | **LOW-MED** | `map_viewer/src/constants.rs:5-143` | Uncited numeric render params | Wall of bare shader-parameter/color/geometry constants with **no citation comment**: `GMT_OFFSET=2793.0`, `MUD_NORMAL_CUTOFF=10.982`, `NIGHT_AMBIENT_BOOST=3.0`, `FEATHER_MIN/MAX`, `SOUTH/NORTH_POLE_OFFSET`, `SNOW_COLOR`, `DAY_AMBIENT_MAP_*`, `NIGHT_AMBIENT_MAP_*`, `GB_THRESHOLD/GB_OPACITY_*`, etc. Values are **correct** (verified against shipped files, see below) but carry no provenance a mod-maintainer can trace. | Add a per-const citation comment (`// gfx/FX/constants.fxh:NN` / `// 00_graphics.lua:NN`), or fold the color/threshold group into a `*_law.rs` with provenance (the pattern the `*_law.rs` modules already follow). | low-med — a documentation/provenance gap, not a wrong value; the module is central (one place), not scattered magic numbers. |

**Verification of finding 2 (values are file-correct, only the citation is missing):**
- `constants.rs:14 GMT_OFFSET=2793.0` == `00_graphics.lua:934` (`GMT_OFFSET = 2793`) and `standardfuncsgfx.fxh:323`.
- `constants.rs:23 MUD_NORMAL_CUTOFF=10.982` == `constants.fxh:62`.
- `constants.rs:11 NIGHT_AMBIENT_BOOST=3.0` == `constants.fxh:14`.
- `constants.rs:39-58 DAY_AMBIENT_MAP_POS_X=(0.1,0.1,0.05)` … == `constants.fxh:17-22` (`DayAmbientMapPosX = float3(0.1,0.1,0.05)` …) — note these are the shader's `DayAmbientMap*`, NOT the `00_graphics.lua:1053 AMBIENT_LIGHT_*` rig (a common confusion; they are distinct).
- `constants.rs:12/13 NIGHT_OPACITY=0.85 / NIGHT_DARKNESS=0.5` == `standardfuncsgfx.fxh:328/329`.

Because the values are all correct, finding 2 is a *provenance-hygiene* fix, not a
fidelity bug. It is listed so the mod-compat mandate's "every render constant is
citation-traceable" is literally satisfied.

---

## (i) Hard-coded vanilla paths bypassing the resolver

Only finding #1 above. Everything else resolves. Confirmed non-violations (all
route through `GameFileResolver`, relative-path constants):
- `bundle_gen.rs` (all `map/` `common/` `gfx/` sources → `resolve_file`), `border_world_synthesis.rs`, `building_files.rs`, `border_pass.rs:85`, `border_law.rs:293-298`, `bloom.rs:295/305`, `grading_law.rs:51-52,690-707`, `replay_app/main.rs:94-101`, `constants.rs:522-529,608-624` (source-manifest path lists).
- Direct `fs::read` reading resolver output: `bloom.rs:296,306`; `grading_law.rs:539,551`; `seasons.rs:310`; `map_text.rs:44,88`; `map_static_data.rs:316`.
- Bundle/capture-artifact opens (pre-resolved, allowed): `render.rs:791,800`; `bundle.rs:78,181,230,252`; `cpu_view.rs`, `cpu_terrain.rs`, `frame_bundle.rs`, `terrain_chunk_library.rs`, `restore_evidence.rs`.
- `*_from_game_dir` public wrappers (`seasons.rs:303`, `map_static_data.rs:59`, `snow_law.rs`, `pdx_mesh.rs`, `font.rs`) build `GameFileResolver::vanilla(...)` but are doc-commented "Test-only convenience wrapper; production callers pass a `GameFileResolver`" and still resolve. **Not violations**; flagged misuse-prone — a future clippy/lint guard on `GameFileResolver::vanilla(` outside `#[cfg(test)]` would harden this.
- **No `include_bytes!`/`include_str!` of shipped game content** exists in either dir.

## (ii) 1936-scenario / country-tag literals

**Zero production violations.** Every 3-letter tag literal (`"GER"/"SOV"/"USA"/…`)
lives inside `#[cfg(test)] mod tests` or a `src/bin/dump_*` diagnostic:
`map_label_law.rs`, `political_color_synthesis.rs`, `map_text.rs` (map_viewer
tests); `bundle.rs`, `map_labels.rs`, `secondary_field.rs`, `palette.rs`,
`gb_field.rs`, `timeline.rs` (replay_app tests); `dump_map_labels.rs:177`
(diagnostic). `"1936"` tokens are data-model/bundle field names
(`state_owner_1936.json`, `Static1936`, timeline test dates), not scenario
literals in render math.

## (iii) Uncited numeric magic constants

Only finding #2 above (`constants.rs:5-143`). The `*_law.rs` modules
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
"hard-coded personal vanilla fallback," not a "resolver/modset bypass"); (2) scope
finding #2 to `constants.rs:5-143` and exclude the `DXGI_*/D3D11_*` SDK-enum block.
It disputed no classification. It additionally suggested a lint guard on
`GameFileResolver::vanilla(` outside test code as a future hardening.
