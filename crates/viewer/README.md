# viewer

When embedded in the desktop shell, the web UI owns map controls and the timeline. Its bounded `MapAction` commands update the same viewing-country, fog and capture state as the standalone controls. The old embedded egui panels are omitted; on-demand diagnostics are returned to Settings through `MapDetails`. Native battle-marker/detail interaction is unchanged.

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

In the desktop dashboard, `dashboard_maps.rs` owns map instances on the same graphics device; the [dashboard crate](../dashboard/README.md) owns recording sources and leases shared with the widgets. Map registry entries keep their renderer on the heap: moving inline renderer state through tree nodes can exhaust the Windows UI thread's stack. Scene permits remain charged through cancelled builds and deferred destruction. Source bytes and compatible CPU content are shared; mutable scenes and their graphics assets are still owned per map.

Use the source map below for implementation and the existing [reference captures](../../reference/) for matched comparisons. Native evidence and remaining assumptions live beside each pass, not in a separate rendering diary.

The map size is the content's provinces bitmap (`MAP_SIZE`), carried by `camera::MapSize` and the shader globals. The WGSL ports carry the shipped shaders' constants, so content that replaces a ported `gfx/FX` file is refused (`scene::PORTED_SHADERS`).

Windowed playback and headless export use the same wgpu rendering path. Preserve the game's verified camera, asset selection, texture colour spaces, shader inputs, blend/depth states and draw order. Do not hide a missing rendering term with a fitted global brightness or colour correction.

Recorded values remain exact. Between hourly samples, moving divisions may glide along their recorded routes using recorded remaining time; the warning area states this presentation exception. Do not interpolate other recorded data. Effects whose seeds, phase or runtime state were not recorded must remain explicitly identified as reconstructions, not exact observations.

Show gaps, unavailable tables, wrong-viewpoint inputs and unsupported content honestly. A carried background sample does not become fresh because the map renders another hour. New visible features may require new recorded operands; do not fabricate them in the renderer.

The main map is an HDR scene. World-space country names draw before post-processing; screen labels and counters are output overlays. Water samples a separate single-sample underwater target and blends over ordinary terrain. Preserve the target formats and pass/depth order in [scene.rs](src/map/scene.rs): changing a transparent pass can alter later layers even when its geometry is unchanged.

Keep coordinate conversions with their caller: bitmap rows run downward, world Z uses `height - 1 - row`, and height sampling has caller-specific half-texel offsets, clamping and rounding. Preserve native traversal and tie order rather than replacing them with an apparently equivalent sort or sampler.

## Source map

| Area | Implementation and important distinction |
|---|---|
| Scene, camera and lighting | [scene.rs](src/map/scene.rs), [camera.rs](src/map/camera.rs), [frame.rs](src/map/frame.rs), [post.rs](src/map/post.rs) with its LUT blends in [post_luts.rs](src/map/post_luts.rs). Simulation time, presentation clocks and post-processing inputs are distinct; post operands come from the recorded content, never vanilla constants. |
| Terrain, water and textures | [terrain.rs](src/map/terrain.rs), [water.wgsl](src/map/water.wgsl), [textures.rs](src/map/textures.rs). Keep native sampling, mip chains and per-resource colour spaces; see [asset contracts](../installed-content/README.md#asset-contracts). |
| Borders, rivers and railways | [borders.rs](src/map/borders.rs), [gradient.rs](src/map/gradient.rs), [rivers.rs](src/map/rivers.rs), [railways.rs](src/map/railways.rs). Border strips, country-distance fields and route graphs are different inputs; do not tune one to hide errors in another. |
| Country view and map fill | [perspective.rs](src/map/perspective.rs), [allegiance.rs](src/map/allegiance.rs), [political.rs](src/map/political.rs). Ownership, control, shared vision and fog are not interchangeable; visibility must match the viewing country. |
| Names and icons | [names.rs](src/map/names.rs), [name_pass.rs](src/map/name_pass.rs), [text.rs](src/map/text.rs), [sprites.rs](src/map/sprites.rs). Country names are rebuilt from recorded operands in the user's game language by `installed_content::country_names`, which the dashboard shares; political colour is the recorded `CountryAppearance` colour plus the subject rule. Use authored glyphs and sprite frames; distinguish world geometry from device-pixel UI. |
| Counter grouping and status | [counters.rs](src/map/counters.rs), [counter_order.rs](src/map/counter_order.rs), [counter_groups.rs](src/map/counter_groups.rs), [counter_status.rs](src/map/counter_status.rs). Native first-fit membership depends on candidate, representative and recorded order, not a country/handle sort. |
| Movement, orders and units | [counter_move.rs](src/map/counter_move.rs), [orders.rs](src/map/orders.rs), [unit_models.rs](src/map/unit_models.rs), [trains.rs](src/map/trains.rs), [aircraft.rs](src/map/aircraft.rs). Share movement geometry between counters and figures; preserve route and admission rules. |
| Trees, buildings and models | [trees.rs](src/map/trees.rs), [sprawl.rs](src/map/sprawl.rs), [map_buildings.rs](src/map/map_buildings.rs), [models.rs](src/map/models.rs). Preserve authored/generated geometry and shader variants. Ordinary map meshes do not inherit terrain's gradient borders or occupation stripes. |
| Weather | [weather.rs](src/map/weather.rs), [weather_effects.rs](src/map/weather_effects.rs), [post.rs](src/map/post.rs). The snow/mud grid averages the game's recorded per-province display bytes, so it needs no history; winter grading uses raw province snow. Particles start empty when a region's weather starts. |
| Availability and limitations | [tables.rs](src/map/tables.rs). Missing old-format operands, wrong-viewpoint data and reconstructed presentation must remain visible to the user. |

## Fidelity limits

A replay can preserve exact simulation values without preserving the native rendered instant. Water/global clocks, camera fades, overlap/stencil producers, particle populations and previous renderer state can require inputs that an hourly sample lacks. A seed alone does not recover their update history. Do not replace these omissions with guessed visibility, fabricated data or fitted colours.


Map text uses the configured user folder's game language (`settings.txt`), not the recording's.

Check the relevant source notes and recording format before treating an old investigation as an open bug or a completed feature. A newer viewer cannot add absent fields to an old recording. Geometry agreement, shader agreement on identical inputs, screenshot similarity and gameplay performance are separate claims.

## Compare changes

```powershell
.\.cargo-target\release\viewer.exe --compare <game.png> <viewer.png> --exclude-game-ui --diff <difference.png>
```

Compare the same recorded hour, branch, perspective, camera and relevant game settings. Report numerical scores and inspect the images; a score improvement alone does not prove every layer is correct. Keep window/headless parity. Reference images and scripts must not become a new corpus-test framework.

For pass isolation, `--dump ... --layers trees` **omits** trees; it does not select only trees. `--counters-json <file>` exports the submitted plates. Omission footprints show where a pass changes the image, not which pass caused all error in those pixels.

`--window-dump` checks the windowed path; its `--animation-time`, `--animation-step` and `--window-frames` options select a repeatable sequence. For headless dumps, `--presentation-time` and `--map-time` set separate clocks that otherwise start at zero; they do not recover native capture times. Match output resolution, upscale and exclusions before comparing. See [main.rs](src/main.rs) or `--help` for accepted combinations.

For local checks and the coordinated end-to-end smoke workflow, see [AGENTS.md](../../AGENTS.md) and the [recorder guide](../recorder/README.md). Embedded lifecycle/control belongs in [desktop](../desktop/README.md) and the [dashboard crate](../dashboard/README.md), not the renderer.

## Historical evidence

The [archived rendering notes](#) retain native traces, experimental scores, rejected candidates and their artifact paths. Those dated results are not an active work plan or proof about the current checkout. Keep lasting interpretations beside source and new captures/reports outside Git; do not rebuild the diary in this README.

## Campaign appearance

The map pop-down keeps export, Fog of War and recording branches. Political fill always uses recorded controllers; the custom owner/controller toggle, its CLI option and the manual Fit map action have been removed. Initial camera framing remains automatic.

Global, campaign and widget appearance travels through the bounded data service and is saved by scoped keys in LOCALAPPDATA/HOI4Replay/map-settings.json. Trees, shadows, rivers, city sprawl, buildings, 3D units and weather particles gate existing draws and matching shadow casters; they preserve native counters, battle interactions and terrain weather. Anti-aliasing selects Off or device-supported 2?/4?/8? MSAA across every world, post and overlay pass. Off renders directly into single-sample targets. Sample changes retire scenes while retaining compatible CPU content and cameras. Texture quality selects the game?s authored terrain texture variants and invalidates only maps using the changed artwork variant. New campaigns inherit global defaults imported from the configured user folder settings.txt; explicit campaign and widget overrides remain independent. VSync changes the root presentation surface without rebuilding map assets.

The desktop graphics controls have been built and exercised on an isolated Windows desktop. High-quality shader variants, the game's full-map-resolution switch, water-reflection setting and gamma behavior still need verified renderer implementations; they are not presented as working controls.

Installed visual mods use the installed-content cosmetic overlay checks. The recording's content is verified first; only explicitly selected compatible cosmetic artwork is then overlaid. Mod order is explicit, with later selections taking precedence. Changing artwork reloads affected maps; simple graphics switches do not rebuild assets. Original recorded artwork remains the default. Browser and desktop read the same scoped preferences through their native data adapter; a bridge rebuild is required after adding this command.

Preferences resolve HOI4 -> global -> campaign -> widget inheritance. The legacy useGameGraphics field now means inherit graphics from the parent scope; missing markers in older saves remain explicit overrides. Null visual-mod selections inherit independently and ultimately preserve recorded artwork. Forms retain inheritance markers while renderer consumers get resolved values. Main Settings needs no open recording. Current game preferences refresh on source-open and loader settings jobs, never per frame; failed reads retain the last known values and report the error. Widget artwork cache keys include recording identity, ordered cosmetic layers and texture quality. Matching variants share immutable assets and decoders; differing variants cannot overwrite each other.

Dashboard map targets set each viewer's perspective and fog independently. Widget PNG exports route by map-view ID into that child viewer's existing capture path; a missing/loading view is refused rather than exporting the standalone map. Per-widget export fidelity requires a matching rendered-view check; standalone exports alone do not establish it.