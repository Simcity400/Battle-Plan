# Political Map-Mode Color Law

## Evidence Scope

This pass validates the captured secondary political color lookup, not a primary
political fill lookup. The available `live_PAN2` capture has a sidecar for
`ProvinceSecondaryColorMap` with `province_secondary_color_map.rgba`, 2816x1024,
11,534,336 bytes, FNV-1a64 `b3df2c5b99e22325`, runtime RGBA8 format, and source
DXGI format 91 (`DXGI_FORMAT_B8G8R8A8_UNORM_SRGB`)
(`C:\Users\Example\AppData\Local\hoi4_captures\live_PAN2\province_secondary_color_map_capture.json:2-11`).
The map-frame bundle declares `ProvinceSecondaryColorMap` at shader slot 6
(`...\live_PAN2\map_frame_bundle.json:684-691`) and records the active SRV as
2816x1024, DXGI 91, slot 6 (`...\live_PAN2\map_frame_bundle.json:896-918`).
The payload manifest records the same `province_secondary_color_map.rgba` bytes
and hash (`...\live_PAN2\map_frame_bundle.json:10435-10439`), and the capture
event confirms a runtime texture readback at slot 6 with the same dimensions and
hash (`...\live_PAN2\terrain_capture_events.jsonl:206`).

Other bundles reference the same lookup family: terrain chunks list
`ProvinceSecondaryColorMap` at slot 6
(`...\live_PAN2\terrain_chunk_library.json:120-130`), trees at slot 15
(`...\live_PAN2\map_frame_trees_pass_bundle.json:704-714`), water at slot 3
(`...\live_PAN2\map_frame_water_pass_bundle.json:708-718`), and the rivers
bundle carries the same payload file/hash
(`...\live_PAN2\map_frame_rivers_pass_bundle.json:4924-4930`). The checked-in
`map_viewer/out/captures/session_20260702d` path was not present in this
worktree, so validation used `C:\Users\Example\AppData\Local\hoi4_captures\live_PAN2`.

## Shader Law

`pdxmap.shader` declares `ProvinceSecondaryColorMap` with Index 6, linear
filtering, and wrap addressing (`<local-path> of Iron IV\gfx\FX\pdxmap.shader:68-76`).
The same shader constructs map UVs with half-texel centers; `uv2.x` is
`(pos.x + 0.5) / MAP_SIZE_X`, while `uv2.y` is vertically flipped as
`(pos.y + 0.5 - MAP_SIZE_Y) / -MAP_SIZE_Y`
(`...\gfx\FX\pdxmap.shader:198-201`). The map-frame constants record
`MAP_SIZE_X = 5632`, `MAP_SIZE_Y = 2048`, and `MAP_POW2/FOW_POW2 = 1`
(`...\live_PAN2\map_frame_bundle.json:10472-10477`), so the captured color
lookup is exactly half-scale: 2816x1024.

The pixel shader applies the lookup through `secondary_color_mask(...,
Input.uv2, ProvinceSecondaryColorMap, ...)`
(`...\gfx\FX\pdxmap.shader:327-328`). `secondary_color_mask` samples the mask
at that UV, multiplies `CalculateOccupationMask(vUV)` by the sampled alpha,
reduces bloom by the same factor, and lerps current diffuse RGB to sampled mask
RGB (`...\gfx\FX\standardfuncsgfx.fxh:979-987`). The separate
`calculate_map_tex_index` terrain-tile decode law is for `TerrainIDMap`, not
for the political secondary color lookup (`...\gfx\FX\standardfuncsgfx.fxh:1065-1073`).

Therefore the implemented province-id-to-texel law is:

1. Output texture shape is 2816x1024 RGBA8, one mip, row pitch 11264.
2. For texel `(x, y)`, sample the center of the corresponding 2x2 full map area:
   `source_x = floor((x + 0.5) * 5632 / 2816)`.
3. Match `uv2.y` by flipping source rows:
   `source_y = 2048 - 1 - floor((y + 0.5) * 2048 / 1024)`.
4. Read that source pixel from `map/provinces.bmp`, map its RGB through
   `map/definition.csv` to a province id, then map province id to state via
   `history/states/*.txt`.

## Static Composition Law

Country colors are resolved through `GameFileResolver` only:

1. `common/country_tags/*.txt` maps tags to `common/countries/*.txt`; for
   example GER maps to Germany and KUW maps to Kuwait
   (`...\common\country_tags\00_countries.txt:1-8`,
   `...\common\country_tags\00_countries.txt:135-138`).
2. Country files provide fallback `color` assignments, such as Germany
   `color = { 106 119 89 }` and Kuwait `color = rgb { 153 76 0 }`
   (`...\common\countries\Germany.txt:1-7`,
   `...\common\countries\Kuwait.txt:1-7`).
3. `common/countries/colors.txt` overrides file colors; GER uses HSV,
   SOV and USA use RGB in that file
   (`...\common\countries\colors.txt:3-14`,
   `...\common\countries\colors.txt:256-259`).

For the 1936 static start, ownership/controller comes from the initial
`history = { ... }` block. Brandenburg has initial owner GER
(`...\history\states\64-Brandenburg.txt:1-13`). Kuwait has owner KUW and
province ids 8085, 13366, 13367
(`...\history\states\656-Kuwait.txt:1-26`). Dated blocks are not part of the
1936 start state; Suzhou has owner CHI initially, while its JAP controller entry
is gated under `1938.10.25`
(`...\history\states\1034 - Suzhou.txt:10-34`).

The static secondary rule implemented here is intentionally narrow:

1. Sea, lake, unknown, no-state, no-owner, and impassable texels are transparent
   in the secondary lookup.
2. If controller equals owner, the secondary lookup is transparent.
3. If a static initial controller differs from owner, the primary diagnostic
   color uses controller color and the secondary lookup stores owner color with
   alpha 255, so the shader can stripe back toward the owner color.
4. Any unresolved tag color is a hard error.

Country-tag aliases and cosmetic states are dynamic state, not static file law.
`common/country_tag_aliases/tag_aliases.txt` documents aliases as trigger-based
country mappings and supports government/cosmetic tests
(`...\common\country_tag_aliases\tag_aliases.txt:1-43`,
`...\common\country_tag_aliases\tag_aliases.txt:78-91`). Playback needs the
extractor to provide the current resolved tag/cosmetic state when it diverges
from vanilla 1936 history.

## Validation v1 (live_PAN2, 1936 start)

Command:

```powershell
cargo run -p map_viewer --bin dump_political_colors -- --game "<local-path> of Iron IV" --capture "C:\Users\Example\AppData\Local\hoi4_captures\live_PAN2" --out "C:\Users\Example\AppData\Local\Temp\cdx_lanes\col1_colors\replay_interface\native\map_viewer\out\political_colors_live_PAN2"
```

Result:

| Metric | Value |
| --- | --- |
| Captured symbol | `ProvinceSecondaryColorMap` |
| Generated FNV | `b3df2c5b99e22325` |
| Captured FNV | `b3df2c5b99e22325` |
| Byte exact | `true` |
| Texel match | `100.00000000%` |
| Texel mismatches | `0` |
| Byte mismatches | `0` |
| Channel mismatches | `[0, 0, 0, 0]` |
| Mismatch classes | none |
| Captured all-zero transparent | `true` |

The generated stats file records the descriptors and exact-match result
(`map_viewer/out/political_colors_live_PAN2/political_color_stats.json:4-45`),
class counts and no mismatch samples
(`map_viewer/out/political_colors_live_PAN2/political_color_stats.json:52-60`),
and notes that vanilla 1936 has no visible static secondary occupation mask in
this capture (`map_viewer/out/political_colors_live_PAN2/political_color_stats.json:61-64`).

Class counts:

| Class | Texels |
| --- | ---: |
| `sea_or_lake_transparent` | 1,869,948 |
| `owner_controller_equal_transparent` | 953,348 |
| `impassable_transparent` | 60,288 |

Dump paths:

| Artifact | Path |
| --- | --- |
| Synthesized secondary RGBA | `map_viewer/out/political_colors_live_PAN2/province_secondary_color_map_synthesized.rgba` |
| Owner/controller diagnostic RGBA | `map_viewer/out/political_colors_live_PAN2/province_owner_color_map_synthesized.rgba` |
| Secondary diff heatmap | `map_viewer/out/political_colors_live_PAN2/province_secondary_color_map_diff.bmp` |
| Stats JSON | `map_viewer/out/political_colors_live_PAN2/political_color_stats.json` |

Determinism was checked by writing a second run to
`map_viewer/out/political_colors_live_PAN2_run2` and comparing SHA-256 hashes:

| Artifact | SHA-256 |
| --- | --- |
| `province_secondary_color_map_synthesized.rgba` | `D4D2639AD94B0211839461226EB57A5E185829E5C4A9783C2DA9F0A3DA032BF5` |
| `province_owner_color_map_synthesized.rgba` | `4527E962BF1EDEF11CCC26A43645DDCBA4B2ED0872D4802BD2F8984C2AAFF5D3` |
| `province_secondary_color_map_diff.bmp` | `8442ED8A69A4A54FCA630E724E3C709C876A5A94C13619DFC8425071391541F3` |
| `political_color_stats.json` | `F995E044935F5A682230127D2F8FE2FE6F354D01B73DFE01CC42C8962F30A477` |

## Validation v2 (restore_gate_1, ongoing vanilla war save)

> **STORE NOTES (updated 2026-07-19):** `hoi4_captures` paths in this historical
> section are stale and the original `restore_gate_1` directory remains absent.
> However, the retained-RDC recovery packet at
> `re_evidence/rendering_laws/RESTORE_GATE_RETAINED/` now exports two fresh
> same-frame observations: a terrain consumer at t6 and a water consumer at t3,
> both binding an all-zero 2816x1024 `ProvinceSecondaryColorMap` with identical
> SHA-256 `d4d2639a...`. This re-proves the consumer, descriptor, and transparent
> case, but remains vacuous for the rare non-transparent producer law. Neither
> RDC is claimed to be the lost original frame.

### Intent

v1 validated only against a 1936-start capture (`live_PAN2`) whose secondary map
was all-transparent — a weak test, because controller equals owner nearly
everywhere at game start, so the law's "transparent" prediction matched trivially.
The `2026-07-06` session
`C:\Users\Example\AppData\Local\hoi4_captures\restore_gate_1\` captured
`ProvinceSecondaryColorMap` from the user's ONGOING vanilla save (loaded via
`--continuelastsave`, matching `autosave.hoi4`, an `IRE` game on "Operation
Postern v1.19.2.0" with many war/event flags set). A war in progress means
occupations, so the secondary map was expected to be NON-trivial — the strong
test v1 could not run.

### Verdict: the captured ongoing-save secondary map is ALL-ZERO

A raw byte scan of
`restore_gate_1\province_secondary_color_map.rgba` (11,534,336 bytes =
2816x1024x4) finds **0 non-zero bytes, 0 non-transparent texels, 0 non-zero-alpha
texels** — fully transparent. Its declared FNV-1a64 is `b3df2c5b99e22325`, which
is **byte-identical to the v1 `live_PAN2` all-zero capture** (same hash, same
length). So the strong test also produced an all-transparent secondary map,
despite the war-in-progress save.

Command:

```powershell
cargo run -p map_viewer --bin dump_political_colors -- --game "<local-path> of Iron IV" --capture "C:\Users\Example\AppData\Local\hoi4_captures\restore_gate_1" --out "...\map_viewer\out\political_colors_restore_gate_1"
```

| Metric | Value |
| --- | --- |
| Captured symbol | `ProvinceSecondaryColorMap` |
| Captured shader slot | `3` (water pass; terrain pass declares slot `6`) |
| Generated FNV | `b3df2c5b99e22325` |
| Captured FNV | `b3df2c5b99e22325` |
| Byte exact | `true` |
| Texel match | `100.00000000%` |
| Captured all-zero transparent | `true` |
| Synthesized all-zero transparent | `true` |
| **Validation is vacuous** | **`true`** |

The 100% / byte-exact match here is **VACUOUS**: both the synthesized static map
and the captured runtime map are fully transparent, so no non-transparent texel
was ever exercised. Transparent-equals-transparent proves nothing about the
province-id -> occupation-stripe mapping. `compare_political_colors_to_capture`
now emits `validation_is_vacuous` and `all_synthesized_texels_transparent_zero`
so this can never be misread as a strong pass
(`map_viewer/src/political_color_synthesis.rs`, `PoliticalColorComparisonReport`).

### Render-path validation (the capture is genuine, not a dumper artifact)

The all-zero map is the resource actually bound at the draw, not a stray/cleared
texture:

1. Terrain bundle `active_draw.shader_resources[6].declared_name` =
   `ProvinceSecondaryColorMap`, sourced from
   `runtime_texture_readback_ProvinceSecondaryColorMap_finalize`
   (`restore_gate_1\map_frame_bundle.json`). The water bundle binds the same
   payload at slot 3 (`restore_gate_1\map_frame_water_pass_bundle.json`).
2. The dumper works: other runtime readbacks in the SAME session came back richly
   non-zero — `terrain_id_map.rgba`, `gradient_border_channel1.rgba`,
   `snow_mud_data.rgba`, `light_index_map.rgba`. So an all-zero
   `ProvinceSecondaryColorMap` is the game's genuine content for this frame in the
   default 3D terrain view, not a readback failure.

### Shader semantics confirmed (occupation stripes)

`pdxmap.shader:328` calls
`secondary_color_mask(diffuse.rgb, normal, Input.uv2, ProvinceSecondaryColorMap, vBloomAlpha)`.
`standardfuncsgfx.fxh:979-987`:
`vColorMask = tex2D(TexMaskSampler, vUV).rgba; vOccupationMask =
CalculateOccupationMask(vUV) * vColorMask.a; vColor = lerp(vColor,
vColorMask.rgb, vOccupationMask)`. `CalculateOccupationMask`
(`standardfuncsgfx.fxh:965-976`) is a diagonal cosine STRIPE pattern. Therefore:

- **texel alpha** = per-province occupation-stripe strength (0 = no stripe).
- **texel rgb** = the color the terrain is striped toward.
- The SRV is sampled unconditionally by the terrain/water/river/tree/mesh shaders
  (`pdxmap`, `pdxwater`, `river`, `tree`, `pdxmesh` all call
  `secondary_color_mask`); there is no shader-side mapmode gate. Whether stripes
  are written is decided CPU-side inside `hoi4.exe`.

An all-transparent map is thus fully consistent with the shader law: alpha 0
everywhere -> `vOccupationMask` 0 everywhere -> no stripes rendered. It does not,
however, *prove* the province-id -> stripe mapping, because no province produced a
non-transparent texel to check.

### Blocker: ongoing-save owner/controller ground truth is not parseable

The strong test also needs the ground-truth owner/controller state for this
frame, to synthesize the expected occupation stripes and diff them. There is no
extractor feed yet, so the only source is the save file itself. This is BLOCKED:

- `autosave.hoi4` (the `--continuelastsave` target) has magic `HOI4bin5` — the
  Clausewitz BINARY (Ironman-style) token format. All 12 most-recent saves are
  `HOI4bin5`; no plaintext (`HOI4txt`) save exists.
- Structural field keys (`owner`, `controller`, `state`, `id`, ...) are encoded as
  2-byte NUMERIC tokens. The literal strings `controller`/`owner`/`province`
  present in the file are inside quoted VALUES (`air_controller`,
  `vlasov_owner_flag`, `SIK_a_modern_province` — event/idea/flag names), not
  struct keys.
- Decoding numeric tokens -> field names requires the game's binary save-token
  dictionary, embedded in `hoi4.exe` and not shipped as any plain file (the game's
  `common/operation_tokens` and `common/synchronized_dynamic_tokens` are unrelated
  content-token files, not the save codec table). No `rakaly`/`melt`-class tool is
  present on PATH, and the repo has no the game save-token table or binary-save parser.

Building that would be a save-format reverse-engineering epic, which is out of
scope for this job. Per the job's STOP rule, the ground-truth-diff arm of the
strong test is reported as blocked rather than attempted.

### Determinism (2x)

Two runs to `political_colors_restore_gate_1` and
`political_colors_restore_gate_1_run2` produced identical SHA-256 for all four
artifacts:

| Artifact | SHA-256 |
| --- | --- |
| `province_secondary_color_map_synthesized.rgba` | `D4D2639AD94B0211839461226EB57A5E185829E5C4A9783C2DA9F0A3DA032BF5` |
| `province_owner_color_map_synthesized.rgba` | `4527E962BF1EDEF11CCC26A43645DDCBA4B2ED0872D4802BD2F8984C2AAFF5D3` |
| `province_secondary_color_map_diff.bmp` | `8442ED8A69A4A54FCA630E724E3C709C876A5A94C13619DFC8425071391541F3` |
| `political_color_stats.json` | `B17FF0873DB702A4C868947FD3CB59B6241532615D25E7ED0BBBECE2FDED5746` |

(The three synthesized-artifact hashes equal v1's, as expected: synthesis reads
the same static installed content. The stats JSON hash differs from v1 because it records
the new `restore_gate_1` capture descriptor and the new vacuous/transparent
fields.)

### Proves vs assumes after v2

| Claim | Status after v2 |
| --- | --- |
| Secondary map shape / sampler / slot (2816x1024 RGBA8, `pdxmap` slot 6, water slot 3, Linear/Wrap) | **PROVEN** from shipped shaders + capture descriptors, both captures byte-match. |
| The SRV drives occupation STRIPES: alpha = stripe strength, rgb = stripe target color, procedural diagonal mask, sampled unconditionally, CPU-gated | **PROVEN** from shipped `pdxmap`/`standardfuncsgfx` shaders. |
| A fully transparent secondary map -> no stripes rendered | **PROVEN** (shader lerp with `vOccupationMask == 0`). |
| The game writes an all-transparent secondary map in the default 3D terrain view for BOTH a 1936 start AND an ongoing war save | **OBSERVED across two captures AND EXPLAINED (2026-07-07, user testimony)**: the player (long-time vanilla the game player) reports occupation is never displayed with stripes in normal play — "occupation pretty much looks the same as normal land", i.e. the province simply takes the CONTROLLER's color in the primary fill. An all-transparent secondary map is therefore the NORMAL steady state, not an anomaly. Whatever populates the stripe path is a rarer mechanism (hypotheses: resistance/collaboration states, a specific mapmode/interaction, or DLC-specific display), currently unknown and LOW PRIORITY. |
| The province-id -> non-transparent occupation-stripe (rgb+alpha) mapping | **STILL ASSUMED and now DEPRIORITIZED** (see row above: stripes are not how ordinary occupation renders). The synthesis's controller!=owner -> alpha-255 stripe emission is WRONG per user testimony and must NOT graduate: ordinary occupation belongs in the PRIMARY fill (controller color), which the replay's owner/controller politics path already models. The stripe synthesis stays available only for whatever the rare trigger turns out to be, behind its own gate. Validation focus moves to the PRIMARY province fill texture. |

## Dynamic Data Needed

Static file law reproduces the all-transparent secondary map captured so far
(1936 start and, byte-identically, the ongoing war save), but that agreement is
vacuous — no non-transparent occupation-stripe texel has been captured yet.
Validating the province-id -> stripe write law, and exact playback over time,
still needs extractor data for:

1. Current controller/owner changes after game start.
2. Occupation state that makes the secondary mask non-transparent.
3. Cosmetic tags, country-tag aliases, civil-war/generated tags, and ideology
   state when those alter the effective country color.
4. A primary political fill SRV or render-target capture. The emitted
   owner/controller texture is a deterministic diagnostic, but the available
   ground truth validated only `ProvinceSecondaryColorMap`.

## Product Rendering Status (2026-07-08)

Replay-driven political color substitution is blocked. The app must not present
generated owner/controller colors as the real the game political fill until either:

1. The primary political fill SRV/render target is captured and replayed, or
2. The exact engine generation path is reverse-engineered from `hoi4.exe`.

The executable contains the relevant `CGradientBorderManager` /
`CGradientBorder::CFillBufferThreaded` trail, but the current Rust
GradientBorder generator still contains captured-fit distance pieces. It remains
a research helper only. Product rendering keeps generated GB and secondary
textures inert and, for captured passes, leaves HOI4-captured political SRVs in
place instead of substituting generated replay textures.
