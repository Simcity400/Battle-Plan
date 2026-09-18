# viewer

The native map and playback application. It resolves the recording's game assets and renders its recorded state. The desktop host embeds this same viewer; there is not a separate web map renderer.

## Use and export

From the repository root:

```powershell
cargo build -p viewer --release
.\.cargo-target\release\viewer.exe <campaign.battleplan>
.\.cargo-target\release\viewer.exe --dump <campaign.battleplan> --hour <n> --branch <id> --out <frame.png> --camera <world-x,world-z,height>
```

On first run, choose the game and Paradox user folders. Local settings are stored beside the executable in `viewer.json`. Playback provides branch selection, a timeline, play/pause/speed, pan and zoom. Select a branch explicitly when an hour exists on more than one branch.

`--perspective <TAG>|recorded` chooses the viewing country. `--fog on|off` controls fog; turning fog off does not remove the country perspective. Missing or mismatching covered content is refused. Some visual assets are outside the recorded hash policy; the [installed-content guide](../installed-content/README.md#asset-contracts) explains that warning. See `--help` for the full CLI.

## Rendering rules

Use the source map below for implementation and the existing [reference captures](../../reference/) for matched comparisons. Native evidence and remaining assumptions live beside each pass, not in a separate rendering diary.

Windowed playback and headless export use the same wgpu rendering path. Preserve the game's verified camera, asset selection, texture colour spaces, shader inputs, blend/depth states and draw order. Do not hide a missing rendering term with a fitted global brightness or colour correction.

Recorded values remain exact. Between hourly samples, moving divisions may glide along their recorded routes using recorded remaining time; the warning area states this presentation exception. Do not interpolate other recorded data. Effects whose seeds, phase or runtime state were not recorded must remain explicitly identified as reconstructions, not exact observations.

Show gaps, unavailable tables, wrong-viewpoint inputs and unsupported content honestly. A carried background sample does not become fresh because the map renders another hour. New visible features may require new recorded operands; do not fabricate them in the renderer.

The main map is an HDR scene. World-space country/state names draw before post-processing; screen labels and counters are output overlays. Water samples a separate single-sample underwater target and blends over ordinary terrain. Preserve the target formats and pass/depth order in [scene.rs](src/map/scene.rs): changing a transparent pass can alter later layers even when its geometry is unchanged.

Keep coordinate conversions with their caller: bitmap rows run downward, world Z uses `height - 1 - row`, and height sampling has caller-specific half-texel offsets, clamping and rounding. Preserve native traversal and tie order rather than replacing them with an apparently equivalent sort or sampler.

## Source map

| Area | Implementation and important distinction |
|---|---|
| Scene, camera and lighting | [scene.rs](src/map/scene.rs), [camera.rs](src/map/camera.rs), [frame.rs](src/map/frame.rs), [post.rs](src/map/post.rs). Simulation time, presentation clocks and post-processing inputs are distinct. |
| Terrain, water and textures | [terrain.rs](src/map/terrain.rs), [water.wgsl](src/map/water.wgsl), [textures.rs](src/map/textures.rs). Keep native sampling, mip chains and per-resource colour spaces; see [asset contracts](../installed-content/README.md#asset-contracts). |
| Borders, rivers and railways | [borders.rs](src/map/borders.rs), [gradient.rs](src/map/gradient.rs), [rivers.rs](src/map/rivers.rs), [railways.rs](src/map/railways.rs). Border strips, country-distance fields and route graphs are different inputs; do not tune one to hide errors in another. |
| Country view and map fill | [perspective.rs](src/map/perspective.rs), [allegiance.rs](src/map/allegiance.rs), [political.rs](src/map/political.rs). Ownership, control, shared vision and fog are not interchangeable; visibility must match the viewing country. |
| Names and icons | [names.rs](src/map/names.rs), [name_pass.rs](src/map/name_pass.rs), [text.rs](src/map/text.rs), [sprites.rs](src/map/sprites.rs). Use authored glyphs and sprite frames; distinguish world geometry from device-pixel UI. |
| Counter grouping and status | [counters.rs](src/map/counters.rs), [counter_order.rs](src/map/counter_order.rs), [counter_groups.rs](src/map/counter_groups.rs), [counter_status.rs](src/map/counter_status.rs). Native first-fit membership depends on candidate, representative and recorded order, not a country/handle sort. |
| Movement, orders and units | [counter_move.rs](src/map/counter_move.rs), [orders.rs](src/map/orders.rs), [unit_models.rs](src/map/unit_models.rs), [trains.rs](src/map/trains.rs), [aircraft.rs](src/map/aircraft.rs). Share movement geometry between counters and figures; preserve route and admission rules. |
| Trees, buildings and models | [trees.rs](src/map/trees.rs), [sprawl.rs](src/map/sprawl.rs), [map_buildings.rs](src/map/map_buildings.rs), [models.rs](src/map/models.rs). Preserve authored/generated geometry and shader variants. Ordinary map meshes do not inherit terrain's gradient borders or occupation stripes. |
| Weather and history | [weather.rs](src/map/weather.rs), [weather_effects.rs](src/map/weather_effects.rs), [post.rs](src/map/post.rs). Snow/mud display caches depend on load and branch history; winter grading uses raw province snow, not that display grid. |
| Availability and limitations | [tables.rs](src/map/tables.rs). Missing old-format operands, wrong-viewpoint data and reconstructed presentation must remain visible to the user. |

## Fidelity limits

A replay can preserve exact simulation values without preserving the native rendered instant. Water/global clocks, camera fades, overlap/stencil producers, particle populations and previous renderer state can require inputs that an hourly sample lacks. A seed alone does not recover their update history. Do not replace these omissions with guessed visibility, fabricated data or fitted colours.

Check the relevant source notes and recording format before treating an old investigation as an open bug or a completed feature. A newer viewer cannot add absent fields to an old recording. Geometry agreement, shader agreement on identical inputs, screenshot similarity and gameplay performance are separate claims.

## Compare changes

```powershell
.\.cargo-target\release\viewer.exe --compare <game.png> <viewer.png> --exclude-game-ui --diff <difference.png>
```

Compare the same recorded hour, branch, perspective, camera and relevant game settings. Report numerical scores and inspect the images; a score improvement alone does not prove every layer is correct. Keep window/headless parity. Reference images and scripts must not become a new corpus-test framework.

For pass isolation, `--dump ... --layers trees` **omits** trees; it does not select only trees. `--counters-json <file>` exports the submitted plates. Omission footprints show where a pass changes the image, not which pass caused all error in those pixels.

`--window-dump` checks the windowed path; its `--animation-time`, `--animation-step` and `--window-frames` options select a repeatable sequence. For headless dumps, `--presentation-time` and `--map-time` set separate clocks that otherwise start at zero; they do not recover native capture times. Match output resolution, upscale and exclusions before comparing. See [main.rs](src/main.rs) or `--help` for accepted combinations.

For local checks and the coordinated end-to-end smoke workflow, see [AGENTS.md](../../AGENTS.md) and the [recorder guide](../recorder/README.md). Embedded lifecycle/control belongs in [desktop](../desktop/README.md) and [desktop-protocol](../desktop-protocol/README.md), not the renderer.

## Historical evidence

The [archived rendering notes](#) retain native traces, experimental scores, rejected candidates and their artifact paths. Those dated results are not an active work plan or proof about the current checkout. Keep lasting interpretations beside source and new captures/reports outside Git; do not rebuild the diary in this README.
