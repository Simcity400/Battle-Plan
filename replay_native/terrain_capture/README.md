# terrain_capture

Rust-native contract crate for extracting HOI4's engine-generated
`TerrainIDMap` and related terrain draw inputs from the DirectX 11 renderer.

This crate is deliberately separate from the gameplay replay collector. The map
renderer needs generated GPU textures, scene constants, the terrain vertex
buffer, and the D3D input layout before it can reproduce HOI4 terrain
faithfully. The current crate encodes the D3D11 hook contract and validates the
state machine needed to identify pdxmap terrain pixel shader variants, use the
TerrainIDMap slot declared by each shader/cache variant, and prepare staging
readbacks.

It does not fabricate a TerrainIDMap from `terrain.bmp`. The production proof
artifact is a structured `map_frame_bundle.json` captured from an active pdxmap
terrain draw, with raw files, hashes, resource descriptors, shader/reflection
identity, IA bindings, draw arguments, and explicit blockers for anything not
yet captured.

The capture run is session-oriented, not a sequence of unrelated one-off dumps.
`map_capture_session.json` declares `terrain_active_pass` as the required first
capture moment. Water, rivers, borders, and trees are named capture moments with
strict shader-family gates. Cities and overlays remain deferred blockers until
they have equivalent shader/resource signatures. No later layer is approximated
from static map bitmaps.

## Commands

Build the DLL and attach helper:

```powershell
cargo build -p terrain_capture
```

Attach the DLL to the running HOI4 window thread:

```powershell
cargo run -p terrain_capture --bin terrain_capture_attach -- --pid <hoi4-pid> --dll target\debug\terrain_capture.dll --shader-cache "<HOI4 shadercache dir>" --dwell-ms 190000
```

Pass `--capture-moment water_pass`, `--capture-moment rivers_pass`,
`--capture-moment borders_pass`, or `--capture-moment trees_pass` to capture a
specific non-terrain map bundle. For the production map path, prefer a single
session sequence:

```powershell
cargo run -p terrain_capture --bin terrain_capture_attach -- --exe "<HOI4 install>\hoi4.exe" --dll target\debug\terrain_capture.dll --working-dir "<HOI4 install>" --shader-cache "<HOI4 shadercache dir>" --capture-moments terrain_active_pass,water_pass,rivers_pass,borders_pass,trees_pass --pre-resume-timeout-ms 30000 --dwell-ms 190000
```

The injected runtime treats runtime textures and shader metadata as
session-wide evidence, but records `MapFrameBundle` completion per named moment.
It advances from terrain to water only after the terrain bundle and terrain
runtime inputs are complete, then advances through rivers, borders, and trees as
each real shader-gated pass bundle is captured. The hook accepts those moments
only when the active pixel shader proves the matching vanilla shader family:

- `water_pass`: `pdxwater.shader` resources such as `HeightTexture`,
  `LeanTexture1`, `LeanTexture2`, and `SpecularMap`
- `rivers_pass`: `river.shader` resources including `RiverData`,
  `LeanTexture1`, `LeanTexture2`, `SnowMudData`, and gradient-border channels
- `borders_pass`: `border.shader` resources including `BorderDiffuse`,
  `SnowMudData`, `IntelMap`, and `ShadowMap`
- `trees_pass`: `tree.shader` resources including `TreeMaskTexture`, `TintMap`,
  `SeasonMap`, and gradient-border channels

Launch HOI4 suspended, inject the DLL before the primary game thread runs, then
resume:

```powershell
cargo run -p terrain_capture --bin terrain_capture_attach -- --exe "<HOI4 install>\hoi4.exe" --dll target\debug\terrain_capture.dll --working-dir "<HOI4 install>" --shader-cache "<HOI4 shadercache dir>" --pre-resume-timeout-ms 30000 --dwell-ms 190000
```

Early launch waits for `terrain_capture_hooks_installed` before resuming the
game's primary thread. If hooks do not install, the helper terminates the still
suspended process instead of letting HOI4 run without the D3D hooks.

The attach helper stages a per-run DLL copy next to the `--dll` path before
injecting. That keeps a stale or still-running HOI4 process from locking the
Rust build output at `target\debug\terrain_capture.dll`.

The current attach proof writes `terrain_capture_probe.jsonl` beside the DLL.
The production hook writes capture events under `terrain_capture\` beside the
DLL. A successful run writes:

- `terrain_capture\map_capture_session.json`
- `terrain_capture\map_frame_bundle.json`
- `terrain_capture\map_frame_drawNNN_ia_vertex_buffer_slot0.bin`
- `terrain_capture\map_frame_drawNNN_ia_vertex_buffer_slot1.bin`
- `terrain_capture\map_frame_drawNNN_ia_index_buffer.bin`
- `terrain_capture\map_frame_drawNNN_vs_constant_buffer_slotN.bin`
- `terrain_capture\map_frame_drawNNN_ps_constant_buffer_slotN.bin`
- per-draw `draw_state.output_merger` render-target and depth-stencil texture
  descriptors inside `map_frame_bundle.json`
- `terrain_capture\pdxmap_pixel_shader_<fnv1a64>.bin`
- `terrain_capture\pdxmap_vertex_shader_<fnv1a64>.bin`
- `terrain_capture\terrain_id_map.rgba`
- `terrain_capture\terrain_id_map_capture.json`
- `terrain_capture\snow_mud_data.rgba`
- `terrain_capture\snow_mud_data_capture.json`
- `terrain_capture\terrain_scene_cb0.bin`
- `terrain_capture\terrain_scene_cb0_capture.json`
- `terrain_capture\terrain_vertex_buffer_sample.bin`
- `terrain_capture\terrain_vertex_buffer_capture.json`
- `terrain_capture\terrain_input_layout_capture.json`
- `terrain_capture\vanilla\gfx\FX\constants.fxh`
- `terrain_capture\vanilla\gfx\FX\pdxmap.fxh`
- `terrain_capture\vanilla\gfx\FX\pdxmap.shader`
- `terrain_capture\terrain_capture_events.jsonl`

The capture path hooks `ID3D11Device::CreateVertexShader`,
`ID3D11Device::CreatePixelShader`, `ID3D11Device::CreateInputLayout`,
`ID3D11DeviceContext::IASetInputLayout`, `ID3D11DeviceContext::VSSetShader`,
`ID3D11DeviceContext::PSSetShader`,
`ID3D11DeviceContext::PSSetShaderResources`, and the terrain draw calls. It
only accepts an SRV after the active pixel shader has been associated with a
`TerrainIDMap_Texture` `register(tN)` declaration. It only accepts an input
layout after the captured `D3D11_INPUT_ELEMENT_DESC` array for the active
`ID3D11InputLayout*` matches
`gfx/FX/pdxmap.shader:VS_INPUT_TERRAIN_NOTEXTURE`. It only accepts an active
terrain vertex shader after bytecode reflection proves `POSITION0`, `TEXCOORD0`,
and the `pdxmap.fxh` VS constant-buffer slot used by the terrain draw path.

For late attach sessions that miss `CreatePixelShader`, the attach helper can
scan the HOI4 shader cache and write a strict
`terrain_capture_control.json` beside the DLL. That control file contains only
the shader-cache `MAP_SIZE`, `MAP_POW2`, `FOW_POW2`, relative cache-file names,
and the `TerrainIDMap_Texture register(tN)` slots seen in pdxmap terrain
variants. Late attach capture still fails closed unless the candidate SRV is
bound on one of those slots and the source texture dimensions exactly match the
shader-cache map size. When a terrain frame bundle is written, the active
shader's declared TerrainIDMap slot selects the matching shader-cache signature
and serializes it as `shader_cache_constants` in `map_frame_bundle.json`.
The bundle also records output-merger render-target/depth-stencil resource
identity, `Texture2D` descriptors, and the last captured
`ClearRenderTargetView` / `ClearDepthStencilView` provenance for every terrain
draw. The renderer requires this evidence whenever the captured draw state has
depth testing enabled.

The readback validates a single-sample shader-resource `Texture2D`, supports
`DXGI_FORMAT_R8G8B8A8_UNORM` and the observed runtime
`DXGI_FORMAT_B8G8R8A8_UNORM`, copies base mip 0 through a staging texture,
strips GPU row padding, converts BGRA storage to canonical RGBA channel order,
and writes raw RGBA bytes for `map_viewer import-terrain-runtime`.

## Current evidence

A `--continuelastsave` early-injection run can launch straight into the current
launcher continue save. The 2026-06-30 one-restart terrain+water attempt proved
that path and produced a fresh terrain capture attempt, but it correctly stayed
blocked instead of promoting an incomplete bundle:

- active HOI4 launch/injection succeeded with
  `--capture-moments terrain_active_pass,water_pass` and
  `--continuelastsave --gdpr-compliant`
- HOI4 reached the real DirectX 11 map path and created a `pdxwater` pixel
  shader (`pdxwater_pixel_shader_5939c77566452cc9.bin`)
- terrain capture recorded a real `TerrainIDMap` (`5632x2048`,
  `DXGI_FORMAT_B8G8R8A8_UNORM`, FNV-1a64 `f493368c56224ab1`) and an active
  terrain draw sequence with 21 `DrawIndexed` records
- the production renderer rejected the bundle because the capture was incomplete:
  the terrain render target and depth-stencil target were 4x MSAA, unresolved
  per-sample payloads were not captured, `SnowMudData` was not bound/captured,
  the terrain frame never reached a clean render-target boundary, and the
  post-pass render-target payload was missing
- `water_pass` did not advance to bundle capture because the session contract
  requires the prior terrain moment to validate before dependent water capture

This is a useful fail-closed result. The next terrain/water milestone must
either support native MSAA target/depth payload capture/replay, or run a
documented capture profile with HOI4 `settings.txt` `multi_sampling` disabled
before the early-injection launch. Water should then be captured after the fresh
terrain bundle validates.

A restarted main-menu attach with shader-cache control produced:

- shader-cache control: `MAP_SIZE 5632x2048`, `MAP_POW2 1,1`,
  `FOW_POW2 1,1`, allowed TerrainIDMap slots `[3, 5]`
- rejected candidate: slot `3`, dimensions `512x512`, rejected before format
  acceptance
- captured candidate: slot `5`, dimensions `5632x2048`,
  `source_dxgi_format = 87` (`DXGI_FORMAT_B8G8R8A8_UNORM`), byte length
  `46137344`, FNV-1a64 `f493368c56224ab1`
- imported runtime input: `map_viewer import-terrain-runtime`
- rendered production terrain output: `map_viewer render-terrain-d3d11`

A late attach after adding input-layout capture produced:

- captured `TerrainIDMap`, `SnowMudData`, `TerrainSceneCB0`, and
  `TerrainVertexBuffer`
- rejected the active terrain input layout with
  `active terrain input layout ... was not created after hook install; D3D11
  does not expose its original D3D11_INPUT_ELEMENT_DESC array`
- wrote no `terrain_input_layout_capture.json`, by design

A late attach after the session contract was added produced raw terrain runtime
inputs again (`TerrainIDMap`, `SnowMudData`, `TerrainSceneCB0`, and a terrain
vertex-buffer sample), but did not produce `map_frame_bundle.json` because the
active pdxmap pixel-shader identity was not observed after hook install. This is
a fail-closed result: the terrain session is blocked until the game is launched
through the early-injection path, or until another shader/binding transition
provides real active shader bytecode/reflection evidence.

An early `--exe` launch with the DLL injected before the primary game thread ran
captured the active terrain input layout and structured map-frame bundle:

- `POSITION0`: DXGI format `14` (`R16G16B16A16_SINT`), input slot `0`,
  aligned byte offset `0`
- `TEXCOORD0`: DXGI format `38` (`R16G16_SINT`), input slot `1`, aligned byte
  offset `0`; this is signed integer height data matching
  `VS_INPUT_TERRAIN_NOTEXTURE`, not normalized data
- previous captured draw sequence: two `DrawIndexed` records, each with
  `draw_count = 24576`, `draw_start = 0`, `base_vertex_location = 0`, IA
  vertex buffer slots `0` and `1`, IA index buffer bytes, topology, VS/PS
  constant-buffer bytes, PS SRV bindings, and PS sampler state
- current extractor contract: append active terrain draws while a pdxmap terrain
  pixel shader is active, keep the manifest blocked while the terrain pass is
  open, and finalize the bundle only when `PSSetShader` transitions away from
  pdxmap terrain
- active shader identity: pdxmap pixel shader bytecode file plus reflection
  evidence for `TerrainIDMap` slot `5` and `SnowMudData` slot `7`
- updated active vertex-shader contract: capture the active pdxmap VS bytecode,
  reflect the shader input signature, require `POSITION0` and `TEXCOORD0`, and
  require the `pdxmap.fxh` terrain draw constant buffer at VS slot `1`
- runtime inputs: `terrain_id_map.rgba` (`5632x2048`,
  `DXGI_FORMAT_B8G8R8A8_UNORM`, FNV-1a64 `f493368c56224ab1`) and
  `snow_mud_data.rgba` (`1408x512`, `DXGI_FORMAT_B8G8R8A8_UNORM`, FNV-1a64
  `7dd6d2345f18243d`)
- source evidence: copied and hashed vanilla `gfx/FX/pdxmap.shader`,
  `gfx/FX/pdxmap.fxh`, and `gfx/FX/constants.fxh`
- `map_frame_bundle.json`: records the active draw plus the coherent terrain
  draw sequence, with all captured files referenced by relative path, byte
  length, hash, format/dimension descriptors, shader-declared names where
  available, `shader_cache_constants` selected by the active TerrainIDMap slot,
  and explicit blockers if any required runtime input is missing
- `map_capture_session.json`: records the session capture plan, the
  `terrain_active_pass` moment status, the required payload classes
  (shader bytecode/reflection, input layouts, all vertex/index buffers, constant
  buffers, SRVs/textures, sampler states, draw args, resource descs, hashes, and
  provenance), and deferred future map-layer moments gated behind terrain
  validation
- production renderer command:
  `map_viewer render-terrain-d3d11 --map-frame-bundle <capture dir|map_frame_bundle.json>`
- production renderer result: consumes the clean bundle, recreates the captured
  D3D11 shaders, IA buffers/layouts, constant buffers, SRVs/textures, samplers,
  rasterizer/blend/depth-stencil/output-merger state, and replays the captured
  terrain draw sequence into an offscreen native D3D11 render target. The older
  CPU terrain renderer is retained only as a validation/reference path. The
  replacement app should use `NativeD3d11TerrainBundle::load(...)` plus
  `NativeD3d11TerrainRenderer::prepare(...)` / `render_prepared(...)`, not CLI
  argument parsing.

## Remaining gap

The strict early-launch path now proves a coherent active-frame manifest rather
than only one-off symbols. The renderer accepts the self-contained
`terrain_active_pass` bundle with `shader_cache_constants` in
`map_frame_bundle.json`; it no longer depends on the legacy `map_bundle`
`terrain_runtime.json` sidecar for TerrainIDMap dimensions or shader-cache map
constants.

The reference CPU renderer consumes the captured multi-stream terrain draw for
validation:

- decode `POSITION0` from IA slot `0` (`R16G16B16A16_SINT`, stride `8`)
- decode `TEXCOORD0` from IA slot `1` (`R16G16_SINT`, stride `4`, captured
  offset preserved from the live bind)
- apply the captured index buffer, topology, draw arguments, constant buffers,
  SRV bindings, sampler states, and shader bytecode/reflection evidence without
  flattening the layout into a guessed single vertex struct
- validate post-diffuse shader inputs for every captured terrain draw, not just
  `active_draw`
- load post-diffuse resources and constant-buffer state from the draw currently
  being rasterized
- run the shader-derived terrain tail after border light removal: city lights
  from `CityLightsAndSnowNoise`, null-SRV `ShadowMap` FOW behavior, distance
  fog from `standardfuncsgfx.fxh`, and final `DayNightWithBlend`
- require output-merger render-target/depth-stencil descriptors and captured
  clear-call provenance when captured terrain draw state has depth testing
  enabled

The remaining renderer blockers are:

- integrate the native D3D11 replay backend into the replacement production app
  surface; do not make the app call CLI argument parsing. The backend now has a
  terrain-first production pass graph that can consume validated water, rivers,
  borders, and trees bundles through `prepare_production_map_with_layers(...)`
  and replay them in order into the app-owned terrain color/depth target
- keep validating any D3D11 output-merger dependencies that are required by the
  captured frame: depth load/clear contents, depth-test/write behavior, culling,
  and exact rasterizer state
- capture and validate a real `water_pass` bundle from HOI4 with the same
  fail-closed contract now used by the renderer: reflected SRVs, reflected
  pixel/vertex constant buffers, sampler slots, camera-updateable `TerrainSceneCB0`,
  topology, draw state, and output target/depth descriptors
- capture real `rivers_pass`, `borders_pass`, and `trees_pass` bundles from
  HOI4 now that the hook has grounded shader/resource signatures for those
  layers; map-viewer has fail-closed validators, production pass-graph promotion
  gates, and D3D11 production replay plumbing for these bundle types, but actual
  parity still depends on real captured layer bundles
- identify equivalent grounded shader/resource signatures for `cities_pass` and
  `overlays_pass`; until then the attach helper rejects those moments instead
  of capturing an unrelated draw

Do not fall back to a guessed slot, guessed input layout, or `terrain.bmp`
expansion.
