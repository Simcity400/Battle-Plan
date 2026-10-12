# viewer

Native map, playback and image-export process. It resolves a recording's content and uses one wgpu renderer for standalone windows, desktop maps and headless exports. The desktop web UI owns embedded controls/timeline and sends bounded commands to this viewer.

Installation-dependent loading initializes `engine::executable_tables` from the configured game folder before reading settings or presentation content. Document workers and corpus runs validate their selected installation before consuming the shared tables. Executable metadata and shipped variable documentation remain in memory; recordings and disk caches contain no copies of these tables.

## Source map

| Area | Entry points |
|---|---|
| Loading, decoding, hour workers | [document.rs](src/document.rs), [playback.rs](src/playback.rs), [hours.rs](src/hours.rs) |
| Window and embedded maps | [app.rs](src/app.rs), [dashboard_maps.rs](src/dashboard_maps.rs) |
| Camera, scene and clocks | [camera.rs](src/map/camera.rs), [scene.rs](src/map/scene.rs), [presentation.rs](src/map/presentation.rs) |
| Perspective, fog and political fill | [perspective.rs](src/map/perspective.rs), [visibility.rs](src/map/visibility.rs), [political_gpu.rs](src/map/political_gpu.rs) |
| World passes and shader adapters | [map/mod.rs](src/map/mod.rs), [shader_sources.rs](src/map/shader_sources.rs), [shader_scene.rs](src/map/shader_scene.rs) |
| Native GUI, text and counters | [native_gui.rs](src/map/native_gui.rs), [native_resources.rs](src/map/native_resources.rs), [counters/](src/map/counters/mod.rs) |
| Dynamic hover | [province_tooltips/](src/map/province_tooltips/mod.rs), [counter_tooltips.rs](src/map/counter_tooltips.rs), [raid guide](src/map/raids/tooltips/README.md), [combat guide](src/combat/README.md) |
| History, diagnostics and checks | [engaged_index.rs](src/engaged_index.rs), [tables.rs](src/map/tables.rs), [profiling.rs](src/profiling.rs), [corpus/](src/corpus/mod.rs) |

[installed-content](../installed-content/README.md) owns content/asset interpretation; [replay](../replay/README.md) owns compatibility; [dashboard](../dashboard/README.md) owns sources/products/protocol; [desktop](../desktop/README.md) owns host/process lifecycle. Equipment graphic-pool inputs preserve an absent archetype separately from a real empty equipment name.

## Use and export

From the repository root:

```powershell
cargo build -p viewer --release
.\.cargo-target\release\viewer.exe <campaign.battleplan>
.\.cargo-target\release\viewer.exe --dump <campaign.battleplan> --hour <n> --branch <id> --out <frame.png> --camera <world-x,world-z,height>
```

Choose game/user folders on first run. Standalone settings live in sibling `viewer.json`; desktop owns its settings. Select branches explicitly for overlapping hours. `--perspective <TAG>|recorded` chooses country; `--fog on|off` retains perspective. See `--help`/[main.rs](src/main.rs).

*Match game* rasterises screen controls/text at game framebuffer/gui scale, then linearly stretches the completed image. Hit tests undo stretch; interface-pixel layout scales corners at upload. World names/geometry stay unscaled. [Unreadable display settings](../installed-content/src/settings.rs) use 1.0 with warning. Dumps use game frame size/scale 1.0 unless `--ui-scale` overrides it.

### Diagnostics

Diagnostics default off. F3/header saves the choice; `--diagnostics` enables one run. The host owns its toggle. Hidden diagnostics build no frame text. Dumps, window captures, `--counters-json` and on-demand `MapDetails` always report limitations, lazy uncovered reads and reachable counter-hover refusals; dump warnings use stderr.

Partial recordings play verified prefixes with always-visible failure status; only known failed hours become gaps. Readable older recordings distinguish missing fields from zero/empty and refuse only dependent features.

Readable damaged recordings use the shared replay reader's warnings and gaps.
Trailer warnings remain visible in map diagnostics; unreadable blocks show gaps
through the existing playback path.

## Playback, threading and frame budgets

[hours.rs](src/hours.rs) builds simulation layers off frames, with read-ahead standalone/source-shared widget decoding and one bounded pool per native root. Views cancel independently. [parallel.rs](src/parallel.rs) preserves input order at process parallelism; raid batches borrow idle builder turns.

Playback keeps speed when builders lag. Show the playhead hour if ready, otherwise the newest built hour before it, all layers/date together. Finish in-flight hours at/before the playhead. Paused seeks, captures and exports await the exact hour. Coalescing preserves playing state/seek counter; hour distance cannot identify a seek.

[presentation.rs](src/map/presentation.rs) separates continuity from unit recreation:

- Forward playback skips/paused catch-up preserve train/weather animation and advance building history over skipped hours.
- Explicit seeks (same-hour too), step back, branch changes and gaps reset continuity. Forward seeks preserve building history only over built hours. Country changes reset entity animation but preserve movement animators; camera/fog changes reset neither.
- `UnitRestart` recreates all on load/seek/step-back/branch change, only the new country's provinces on perspective changes, and none on a gap.

Equal layers share identity/skip uploads. Keep decoding, content/localisation reads, simulation joins and whole-map CPU rebuilds off frames. Builders/destruction run below UI priority, engagement refinement lower. Borrowed work runs at least at its waiting thread's priority. Completed-hour queues follow concurrency; `Dropper` bounds retirements and frees locally when full.

Heap-own dashboard renderers to protect the UI stack. Charge scene permits through cancelled builds/destruction. Share immutable content/sources; each view owns mutable history and follows the [upload-arena contract](../shader-pipeline/README.md#gpu-draw-contract).

Advance the shared [province schedule](src/map/province_schedule.rs) before icon owners/rendering; models/buildings/counters share [admission](src/map/province_admission.rs). Off-screen dirty provinces retain history until phased updates clear it. Present while transitions/dirty owners require frames.

[mesh_budget.rs](src/map/mesh_budget.rs) shares the content's frame allowance in native province/building/formation order and retries held creations. Deferred land can exceed it; it is not a total cap. Popups change scale, not alpha. Dumps settle even at zero allowance; live exports retain history.

Raid offers keep their rolling cursor/content batch size, stale targets during playback and paused batches until current. The lane keeps shown/predecessor hours and presentation-reset/country stamps; install only matching results. Replace raid quads alone; parallel batches must equal row-order evaluation.

## Rendering rules

Preserve native camera, traversal/ties, assets, sampling/mips/colour spaces, constants, states and draw order. Never fit brightness/grading to hide missing terms. Content supplies map dimensions. Bitmap rows run down; world Z is `height - 1 - row`; height offsets/rounding/clamping belong to each caller.

Do not interpolate hourly simulation. Moving division counters/figures stand at the settled route point; progress drives arrows for the viewer's locally selected units. Movement shares content geometry; unavailable routes fall back to the province with a warning. A seed alone cannot recover runtime history.

The scene is HDR: world names precede post-processing; screen labels/counters are overlays. Water samples a single-sample underwater target. Finish world passes per wrap view; underwater precedes current shadows and reads their previous target. Preserve [scene formats](src/map/scene.rs)/[pass dependencies](src/map/scene_passes.rs): isolated failures skip dependent outputs/count once; interleaved-chain errors prevent submission/invalidate post caches.

### Compiled map shaders

Map/GUI effects compile recorded content through [shader-pipeline](../shader-pipeline/README.md), equally for vanilla/mods. [shader_sources.rs](src/map/shader_sources.rs) owns selection and diagnostics. Viewer-authored shaders handle political compute, atlas quads, post-off copy and BC unpack.

Bind native slots/nulls, stage buffers/physical allocations, byte offsets, layouts/topologies and authored/forced states even when mod declarations change. Retain bindings across refused/missing-technique draws. Unknown in-allocation bytes refuse; reads beyond allocations are zero. Never invent padding.

Decode/globals share client `force_pow2_textures`. File textures retain padding/block extent/mips; generated textures retain size. Font UVs use loaded page extent. Preserve native precision/[colour spaces](../installed-content/README.md#asset-contracts). Adapters own arrow modes/labels, regular-before-alternate queues, tree visibility/ties and particle rings/order. Meshless actors still run state/attachment/particle/light events; retain producer-selected states/clocks and count unsupported events/audio.

Counter type atlases pack complete loaded strips. Each plain sprite retains its horizontal or vertical frame grid, loaded dimensions, native block rounding and fractional UV boundaries; drawable and hover geometry use that sprite's operands. Other sprite kinds are withheld with named, counted geometry refusals.

### Political fill and devices

Fill uses recorded controllers; ownership/control/vision/fog remain distinct. Derive selected-country fog from simulation. Recorded perspective follows LocalPlayer/observer selection; unresolved references use the diagnosed lowest-slot fallback, never guessed participants.

[political_gpu.rs](src/map/political_gpu.rs) diffs worker-built tables against displayed state after skips/seeks; unchanged frames dispatch nothing. Loaders warm shared content/device static data. Equal fills share states; distinct fills use a bounded reusable pool. Claim/reuse/pool under the registry lock across loader/UI/dropper threads. Copy only recomputed scratch cells; reject storage-limit updates before mutation. Preserve native scan/early-exit/coastal/truncation rules and content search extents.

Political naval FX consumes the recorded native sorted supremacy cache, with control and gaining-side bytes derived for the viewing country. Timed acquisition certifies its native midnight cache epoch; unknown or crossed epochs withhold affected regions with named counts. Equal-current top/second candidates require identical output. Family precedes directed war; gaining neutral rows read the directed naval-blockade pointer from complete relation tuples, with Country0 refusal at that reached operand. Terrain minimums, thresholds and shader artwork resolve through the recorded content stack.

No production CPU fallback. [render::choose_adapter](src/render.rs) selects qualifying hardware at `WGPU_POWER_PREF`, then WARP/lavapipe with notice, equally for windows/widgets/dumps. Unsupported devices/dimensions return `UnsupportedPoliticalCompute`. Share one device per process; serialize GPU harness device creation to avoid concurrent WARP faults.

## Native GUI and hover ownership

Use native resources/byte-preserving names and one retained [Instance](src/map/native_gui/lifecycle.rs) per window. The shared walk owns integer viewport layout, clipping, grids/scrollbars/reparenting and authored controls. Queried extents truncate; rasterised frames can stay fractional.

Hours hold ordered calls; frames apply them. Retain bits/writes/drawables/clocks until overwritten. Undefined/unchanged sprite names keep the drawable; replacement starts fresh clocks. Tick all related instances' clocks, reverse-registration transitions, then draws; present while `Instance::active` holds, including inherited shader clocks.

Pools belong to type/view, reusing state in native request/erase order; grid keys include pool/slot/place. Draw `Pool::visible` only, excluding erased fading icons/items. Refused updates retain membership/place but withhold draws until writes repair every unknown part; structural failures require reconstruction. Hide alone cannot repair unknown drawable state.

Producers own text/values, clipped geometry owns hits, and [ToolTip](src/combat/tooltip_painter.rs) owns wrapping/baking/placement/delay. Hit-test reverse paint/child order with transparent-text pass-through; pool order is not paint order. Disabled raid frames own hover. Combat windows precede map hits; map order is raid, land marker, naval marker, counter, province. New targets reset delay; same-province refresh preserves it.

Cache by content/frame/perspective/vision/settings. Raid text coalesces off-thread with same-icon carry. Use shared typed localisation/scripted-text evaluators. Acquire inline sprites from all loaded/runtime-selected text; absent declared artwork refuses layout. Count occurrences, keeping independent fragments.

Faction-pin highlights reuse the displayed hour's political regions and demilitarised zone. The [highlight lane](src/app/faction_highlight.rs) queues border plans on the hour builders and installs only results matching the current hour, perspective, border identity and hovered regions. Clearing or changing hover restores the prepared ordinary plans while the new highlight is pending.

Province text covers the default political callback only, with fog-gated buildings and potentially empty stateless sea/lakes. [Picking](src/map/province_pick.rs) uses native ray/height cells, not terrain triangles. Fixed-point time/night require known-unsuspended weather state; never recompute held values. Other map modes/selection additions need inputs. Ordinary maps keep carrier wing stacks empty. Battle plans include the viewing country and, when the local Allied battle plans option is enabled, native admitted allied occurrences. Unit selection stays local: owned-counter clicks select, Shift-click toggles and Escape clears; branch or viewing-country changes clear it. Complete censuses retire missing handles while unavailable censuses retain selection. Selected province text uses current-hour route operands through [selected movement](src/map/raids/reach/selected_common.rs), with named refusals for missing inputs.

Snow/mud display takes retained edges and refresh operands from the recorded native boundary. Dirty refreshes preserve the previous plane and advance a local fade on presentation frames, including paused playback. Seeks, step backs, branch/perspective changes and gaps rebind the target snapshot; forward playback, camera and fog changes preserve phase. Historical absence and read refusal remain named gaps.

[Selected transport](src/map/raids/reach/selected_transport.rs) joins authenticated current-hour convoy, unit, access and sea-search inputs. Its [nested graph](src/map/raids/reach/selected_transport_graph.rs) demands spotting and damage lazily, preserving failure order, repeated selections, each weight and independent output buffers. Missing reached risk operands refuse the result; they never become zero. Railway callers retain their own early returns and append laws.

## Fidelity limits

- Terrain deliberately uses a uniform full-resolution grid, differing from native distant-coast LOD.
- Map text uses configured game language. Stored localised bytes without keys require recording-language agreement; proven literals follow the shared stored-text rule.
- Legacy stored-route ETA permits the approved five-hour operand span; future/expired provenance refuses. Current division, railway-gun and task-force counter arrival lines use fresh current-hour route queries. The native client counter cache remains unrecorded and can lag a fresh query. Counter hovers append no date.
- Cursor-zero counter columns can differ from the client's ordering/mixed-country flags. Preserve recorded members and native first/last representative rules.
- Engagement keeps native lag through the shared index. Pending/ambiguous history stays diagnosed; recorded naval bytes settle only the recording player's view. Naval windows refresh only while engaged and otherwise retain compatible earlier contents. Unseen frames use view/index history, then land authored/naval binding initial values with counted reasons; fallback establishes no history.
- Ambient RNG/phase, particles, retained-target initial contents, some placement/selection producers and shadow projection remain reconstructed/unavailable. Warm-up/settling does not recover a captured game instant; consult producer refusals.

## Campaign appearance

Preferences resolve HOI4 -> global -> campaign -> widget. Preserve inheritance markers separately from resolved values; null cosmetics inherit independently. Read game settings on source-open/settings jobs, never per frame; failures keep last values with diagnostics. Verify recorded artwork before overlaying explicitly selected compatible cosmetics in ordered precedence.

Switches gate existing draws/shadow casters, preserving counters, interactions and terrain weather. MSAA covers world/post/overlays; sample changes retire scenes, retaining compatible content/cameras. Quality selects authored textures; cache by recording, ordered cosmetics and quality. VSync updates the root surface. River/water/reflection choose native techniques; full map resolution renders the world at full size or truncating half dimensions with the compiled restore pass; matched visual/runtime validation remains separate. Gamma lacks a verified implementation. Day/night is a local display choice: disabling it fixes virtual lights while simulation time and province clocks remain authoritative. NATO choice also governs technology, carrier and nested scripted unit icons.

Widgets keep independent perspective/fog and export by view ID; missing/loading views refuse. Simple draw switches retain assets; artwork changes reload affected maps.

## Compare changes

Tests that read interface grammar, content registries or fixed modifier names initialize runtime metadata through `engine::executable_tables::initialize_for_tests()`. Set `BATTLE_PLAN_GAME_DIR` to the supported installation; those tests fail explicitly when it is unset. Pure geometry and recording metadata fixtures need no installation.

Use [AGENTS.md](../../AGENTS.md) for scoped checks and [recorder](../recorder/README.md) for oracle/smoke integration. Match hour/branch/perspective/camera/resolution/settings; report scores and inspect images. Compilation, identical-input pixels, live fidelity and performance are separate checks.

```powershell
.\.cargo-target\release\viewer.exe --compare <game.png> <viewer.png> --exclude-game-ui --diff <difference.png>
```

UI masking supports 2560x1440/3440x1440 only. `--layers trees` omits trees; `--counters-json` exports submitted plates/diagnostics. `--window-dump` checks window loading/rendering with repeatable animation controls. Headless presentation/map clocks are separate/default zero. Dumps settle pool requests at fixed hour/clocks; `--settle` advances the interval. Actor-local failures omit all parts; infrastructure/identity/frame/output errors remain fatal.

CPU GUI checks use synthetic technique, sampler, blend and clock metadata. Installed effect checks compare compiled submission with independently packed direct draws using resolved shader sources. They use `BATTLE_PLAN_GAME_DIR` and the configured compiler; `BATTLE_PLAN_SHADER_CONTENT_OVERLAY` adds an ordered, platform-separated list of content folders above the game layer. Module-local tests also cover political full/incremental/multi-map exactness and recording sweeps, plus [content-only scenes](src/map/offline_scene.rs), which prove no recorded unit/political history. Serialize GPU harnesses. `BATTLE_PLAN_VIEWER_CHECK_OVERLAY` verifies raid replacement equality.

| Profiling switch | Use |
|---|---|
| `BATTLE_PLAN_VIEWER_PROFILE` | CPU laps; window sweeps: `play:<hours/s>`, `pan:<units/s>`, `zoom:<factor/s>`, `hours`. |
| `BATTLE_PLAN_VIEWER_PROFILE_HZ`, `BATTLE_PLAN_VIEWER_PROFILE_SWEEP` | Sweep pace/length. |
| `BATTLE_PLAN_VIEWER_NO_VSYNC`, `BATTLE_PLAN_VIEWER_GPU_WAIT` | Uncapped presentation; GPU drain. |
| `BATTLE_PLAN_VIEWER_GPU_PASSES` | Frame-correlated pass timestamps; measure frames separately because collection waits. |
| `BATTLE_PLAN_VIEWER_BENCH_FRAMES` | Extra dump frames with CPU/drain timing. |
| `BATTLE_PLAN_VIEWER_BENCH_MODELS`, `BATTLE_PLAN_VIEWER_MODEL_FRAMES` | Live model histories after settled dump. |
| `BATTLE_PLAN_MAP_SCENE_RGBA16F` | New-path HDR readback before grading, outside benchmark timing. |

Choose matched game/viewer baselines from your own recording and installation. [Scale tests](src/unit_scale_tests.rs)/[fixtures](examples/unit_scale_fixture.rs) check volume, not fidelity/frame acceptance. Measure whole views, set affinity before startup and keep builds out of timings. Captures and reports stay outside Git.

## Mod corpus check

```powershell
.\.cargo-target\release\viewer.exe --mod-corpus --out <run> --workshop <steamapps/workshop/content/394360> [--game-dir <game>] [--user-dir <user>] [--cache <cache>] [--only vanilla,<stack>,...] [--jobs <n>] [--values[=text]]
.\.cargo-target\release\viewer.exe --mod-corpus-diff <base-run> <head-run> [--out <diff.md>]
```

[corpus/readers.rs](src/corpus/readers.rs)/[dashboard::content_check](../dashboard/src/content_check.rs) own coverage. Runs use vanilla/individual mods with dependencies/DLC/language and scratch descriptors, not loose user files. Name missing dependencies/unrun readers; reuse external hash caches and keep installs read-only.

Driver findings never pollute live catalogues. Typed channels decide kinds; `content` reports content interpretation, while `offline` reports missing recorded context. Preserve those distinctions, actionable refusals/fallbacks, occurrence counts/examples and shared notes once. Diff diagnostic multisets from manifests only.

`--values` hashes normalized values; `--values=text --only <stack>` writes the same stream. Preserve list order; normalize hash containers/exclude timing/caches. This checks interpretation, not pixels/shaders/recorded evaluation. Runs stay under external captures.
