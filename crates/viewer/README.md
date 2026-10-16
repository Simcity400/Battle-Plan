# viewer

Native map, playback and image-export process for the desktop app. It resolves a recording's content and uses one wgpu renderer for the Campaign map, dashboard map widgets and headless exports. It has no interface of its own: the [desktop host](../desktop/README.md) starts it, owns every control, setting and timeline, and sends bounded commands to it. Started on its own, it runs only the tooling below.

Installation-dependent loading initializes `engine::executable_tables` from the configured game folder before reading settings or presentation content. Document workers and corpus runs validate their selected installation before consuming the shared tables. Executable metadata and shipped variable documentation remain in memory; recordings and disk caches contain no copies of these tables.

## Source map

| Area | Entry points |
|---|---|
| Loading, decoding, hour workers | [document.rs](src/document.rs), [playback.rs](src/playback.rs), [hours.rs](src/hours.rs) |
| Campaign map and map widgets | [app.rs](src/app.rs), [embedded.rs](src/embedded.rs), [dashboard_maps.rs](src/dashboard_maps.rs) |
| Camera, scene and clocks | [viewer-foundation](../viewer-foundation/README.md), [viewer-world](../viewer-world/README.md) |
| Perspective, fog and political fill | [viewer-foundation](../viewer-foundation/README.md), [viewer-world](../viewer-world/README.md) |
| World passes and shader adapters | [map/mod.rs](src/map/mod.rs), [viewer-world](../viewer-world/README.md) |
| Native GUI, text and counters | [viewer-foundation](../viewer-foundation/README.md), [viewer-counters](../viewer-counters/README.md) |
| Dynamic hover | [raid guide](../viewer-raids/src/map/raids/tooltips/README.md), [combat guide](../viewer-combat/src/README.md), [viewer-counter-products](../viewer-counter-products/README.md) |
| History, diagnostics and checks | [engaged_index.rs](src/engaged_index.rs), [corpus/](src/corpus/mod.rs), [viewer-foundation](../viewer-foundation/README.md) |

[installed-content](../installed-content/README.md) owns content/asset interpretation; [replay](../replay/README.md) owns recording interpretation; [dashboard](../dashboard/README.md) owns sources/products/protocol; [desktop](../desktop/README.md) owns host/process lifecycle. Equipment graphic-pool inputs preserve an absent archetype separately from a real empty equipment name.

## Use and export

From the repository root:

```powershell
cargo build -p viewer --release
.\.cargo-target\release\viewer.exe --dump <campaign.battleplan> --hour <n> --branch <id> --out <frame.png> --camera <world-x,world-z,height>
```

Interactive maps open in the desktop app. The host passes its effective game/user folders on the viewer's command line and later changes as a `Folders` map action; the viewer saves nothing. Tooling defaults to the desktop app's saved folders (`APPDATA/battle-plan/content.json`) unless `--game-dir`/`--user-dir` name others. Select branches explicitly for overlapping hours. `--perspective <TAG>|recorded` chooses country; `--fog on|off` retains perspective; `--day-night on|off` and `--allied-plans on|off` choose those display switches (default on and off). See `--help`/[main.rs](src/main.rs).

The camera moves by the game's law ([motion.rs](../viewer-foundation/src/map/camera/motion.rs)): edge scrolling (when `corner_scrolling` is on, within 2 pixels of the map's edge), the arrow keys, the wheel and Page Up/Page Down move a target camera, and the shown camera eases towards it. The content's `CAMERA_ZOOM_*` and `CAMERA_INTERPOLATION_SPEED` defines and the user's `scroll_speed`, `zoom_speed` and `camera_speed` supply the operands. The wheel acts on the map under the pointer. Keys and edge scrolling act on the focused map: the window must have the system's focus, and on the desktop dashboard the map must also be the one last pressed with a mouse button (Escape and Tab do not change that), so passing over another widget's border on the way somewhere does not pan it. Middle-button drag panning is a viewer addition the game does not have; it moves the shown camera at once, and the target with it. A fitted camera stays fitted until the camera actually moves.

*Match game* rasterises screen controls/text at game framebuffer/gui scale, then linearly stretches the completed image. Hit tests undo stretch; interface-pixel layout scales corners at upload. World names/geometry stay unscaled. [Unreadable display settings](../installed-content-settings/src/settings.rs) use 1.0 with warning. Dumps use game frame size/scale 1.0 unless `--ui-scale` overrides it.

### Diagnostics

Diagnostics default off. The host owns the choice: F3 in a map asks the host to toggle it, and the host sends the result to every map; `--diagnostics` enables one run. Hidden diagnostics build no frame text. Dumps, window dumps, host exports, `--counters-json` and on-demand `MapDetails` always report limitations, lazy uncovered reads and reachable counter-hover refusals; dump warnings use stderr.

Partial recordings play verified prefixes with always-visible failure status; only known failed hours become gaps. The host's diagnostics list recorded event losses (a same-process reattach) over the interval leading to their second read, whose recorded events are incomplete, and the gaps. Unavailable recording inputs remain distinct from zero/empty and refuse only dependent features.

Readable damaged recordings use the shared replay reader's warnings and gaps.
Trailer warnings remain visible in map diagnostics; unreadable blocks show gaps
through the existing playback path.

## Playback, threading and frame budgets

[map_checkpoints.rs](src/map_checkpoints.rs) supplies exact-hour `Current` values
to playback through dashboard's shared `prepared_map::Reader`. Seek restores the
nearest independent native checkpoint, then reads selected recording tables
through the target hour and applies fewer than 720 recorded updates. Source
parsing still begins at the source block's keyframe. An exact checkpoint needs
no source payload. Steps advance the native decoder in place; a needed source
load retains the rest of that accepted block for later steps. Only one parsed
source block remains between requests. No hourly updates or packet seeds are
stored in prepared data. Checkpoints retain availability, partial counts and
supplying-read hours. The selection is the union of native map inputs and the
shared authored-trigger operands, so mod-defined predicates keep their inputs.
Other tables remain explicitly unavailable. Header/gap searches run on disk;
source reads skip interleaved branches and stop at the accepted hour/block.
Targets after accepted preparation use ordinary playback.
Playback releases its previous local decoded output before advancing; externally
retained displayed hours remain immutable. Hour builders install the results
through the same displayed-hour and presentation path, retaining native routes,
clocks and seek intent. Cleared or corrupt prepared data uses ordinary playback.

The ignored `map_checkpoints::tests::measurement::checkpoint_disk_seek_measurement`
test measures growing prefixes in a release build. Set `BATTLE_PLAN_MAP_BENCH_REPLAY` to
the read-only long campaign, `BATTLE_PLAN_MAP_BENCH_PREFIX_HOURS` to increasing elapsed
hours such as `8760,43800,87600`, and `BATTLE_PLAN_MAP_BENCH_BURNIN_REPLAY` to the continuity
recording. Prefix hours include the first recorded hour and round up to a source
block boundary. Every prefix and the burn-in compare checkpoint strides 360, 720
and 1440. Set `BATTLE_PLAN_MAP_BENCH_EVIDENCE` outside Git, `LOCALAPPDATA` to a private
directory within it, and `BATTLE_PLAN_GAME_DIR` to the configured installation. Optional
`BATTLE_PLAN_MAP_BENCH_PROCESSOR_AFFINITY` is a decimal or hexadecimal Windows process
mask, applied before worker creation. The harness uses its runtime working
directory, so the same test binary can run from a shared measurement worktree.

Each row reports actual checkpoint-only Store bytes per 1,000 recorded hours,
checkpoint load times, empty/repeated/earlier-decoder seeks at 41 equally spaced
clock-hour targets per branch including the last 10%, every consecutive-hour
step and native update-application timings. Source block load stalls, selected
source parse time/frame counts and retained-block steps are reported separately.
Preparation, attribution and source reads stop at the accepted prefix. Whole
checkpoint and ordinary accept times are upper bounds including Store writes.
The production-stride candidate also compares the recording's own keyframes,
without prepared checkpoints, using sequential and parallel selected reads.
Its estimated Store bytes exclude the omitted checkpoint payloads and entries.

Completed prefix reports and `prefixes.jsonl` are synced before Store cleanup;
each completed stride is also saved in `partial-report.json`. Growth reports
include byte/byte-per-hour, timing and memory ratios, with dominant native tables
named in `table-checkpoints.csv`. Native row payloads are independently compressed
by table for attribution; their proportional shares of whole checkpoint bytes
are estimates. Recording section bytes are actual selected source sizes and are
not written to prepared storage. Private bytes are sampled every 10 ms within a
prefix; the Windows lifetime peak commit is separate. Memory includes attribution
and parity alternatives. A separate 438,000-hour projection supplements the real
prefix measurements; integrated dashboard acceptance measures the whole store.
Exact selected rows, availability, metrics and supplying hours are compared with
the ordinary replay decoder. Timings end at decoded `Current`; hour builders,
GPU work and presentation need their separate native checks.

Source loaders bind the host's committed lightweight map-progress page to the
shared decode worker; Campaign map decode lanes carry the same source context.
Requests beyond the known prepared prefix read newly committed progress without
starting preparation. Native widget source ownership comes from the host DTO;
its path and revision open/share the recording on one background source worker.
The host's source ID is echoed for ownership and lifecycle and is never looked
up as a viewer-local analytics lease. Engagement append inheritance checks input
and index/branch/session/gap metadata without rereading compressed block bytes.

[hours.rs](src/hours.rs) builds simulation layers off frames, with read-ahead Campaign map/source-shared widget decoding and one bounded pool per native root. Views cancel independently. [parallel.rs](../viewer-foundation/src/parallel.rs) preserves input order at process parallelism; raid batches borrow idle builder turns.

Playback keeps speed when builders lag. Show the playhead hour if ready, otherwise the newest built hour before it, all layers/date together. Finish in-flight hours at/before the playhead. Paused seeks, captures and exports await the exact hour. Coalescing preserves playing state/seek counter; hour distance cannot identify a seek.

[presentation.rs](../viewer-foundation/src/map/presentation.rs) separates continuity from unit recreation:

- Forward playback skips/paused catch-up preserve train/weather animation and advance building history over skipped hours.
- Explicit seeks (same-hour too), step back, branch changes and gaps reset continuity. Forward seeks preserve building history only over built hours. Country changes reset entity animation but preserve movement animators; camera/fog changes reset neither.
- `UnitRestart` recreates all on load/seek/step-back/branch change, only the new country's provinces on perspective changes, and none on a gap.

Equal layers share identity/skip uploads. Keep decoding, content/localisation reads, simulation joins and whole-map CPU rebuilds off frames. Builders/destruction run below UI priority, engagement refinement lower. Borrowed work runs at least at its waiting thread's priority. Completed-hour queues follow concurrency; `Dropper` bounds retirements and frees locally when full.

Heap-own dashboard renderers to protect the UI stack. Charge scene permits through cancelled builds/destruction. Share immutable content/sources; each view owns mutable history and follows the [upload-arena contract](../shader-pipeline/README.md#gpu-draw-contract).

Advance the shared [province schedule](../viewer-foundation/src/map/province_schedule.rs) before icon owners/rendering; models/buildings/counters share [admission](../viewer-foundation/src/map/province_admission.rs). Off-screen dirty provinces retain history until phased updates clear it. Present while transitions/dirty owners require frames.

[mesh_budget.rs](../viewer-world/src/map/mesh_budget.rs) shares the content's frame allowance in native province/building/formation order and retries held creations. Deferred land can exceed it; it is not a total cap. Popups change scale, not alpha. Dumps settle even at zero allowance; live exports retain history.

Raid offers keep their rolling cursor/content batch size, stale targets during playback and paused batches until current. The lane keeps shown/predecessor hours and presentation-reset/country stamps; install only matching results. Replace raid quads alone; parallel batches must equal row-order evaluation.

## Rendering rules

Preserve native camera, traversal/ties, assets, sampling/mips/colour spaces, constants, states and draw order. Never fit brightness/grading to hide missing terms. Content supplies map dimensions. Bitmap rows run down; world Z is `height - 1 - row`; height offsets/rounding/clamping belong to each caller.

Do not interpolate hourly simulation. Moving division counters/figures stand at the settled route point; progress drives arrows for the viewer's locally selected units. Movement shares content geometry; unavailable routes fall back to the province with a warning. A seed alone cannot recover runtime history.

The scene is HDR: world names precede post-processing; screen labels/counters are overlays. Water samples a single-sample underwater target. Finish world passes per wrap view; underwater precedes current shadows and reads their previous target. Preserve [scene formats](../viewer-world/src/map/scene.rs)/[pass dependencies](../viewer-world/src/map/scene_passes.rs): isolated failures skip dependent outputs/count once; interleaved-chain errors prevent submission/invalidate post caches.

### Compiled map shaders

Map/GUI effects compile recorded content through [shader-pipeline](../shader-pipeline/README.md), equally for vanilla/mods. [shader_sources.rs](../viewer-world/src/map/shader_sources.rs) owns selection and diagnostics. Viewer-authored shaders handle political compute, atlas quads, post-off copy and BC unpack.

Bind native slots/nulls, stage buffers/physical allocations, byte offsets, layouts/topologies and authored/forced states even when mod declarations change. Retain bindings across refused/missing-technique draws. Unknown in-allocation bytes refuse; reads beyond allocations are zero. Never invent padding.

Decode/globals share client `force_pow2_textures`. File textures retain padding/block extent/mips; generated textures retain size. Font UVs use loaded page extent. Preserve native precision/[colour spaces](../installed-content/README.md#asset-contracts). Adapters own arrow modes/labels, regular-before-alternate queues, tree visibility/ties and particle rings/order. Meshless actors still run state/attachment/particle/light events; retain producer-selected states/clocks and count unsupported events/audio.

Counter type atlases pack complete loaded strips. Each plain sprite retains its horizontal or vertical frame grid, loaded dimensions, native block rounding and fractional UV boundaries; drawable and hover geometry use that sprite's operands. Other sprite kinds are withheld with named, counted geometry refusals.

### Political fill and devices

Fill uses recorded controllers; ownership/control/vision/fog remain distinct. Derive selected-country fog from simulation. Recorded perspective follows LocalPlayer/observer selection; unresolved references use the diagnosed lowest-slot fallback, never guessed participants.

[political_gpu.rs](../viewer-world/src/map/political_gpu.rs) diffs worker-built tables against displayed state after skips/seeks; unchanged frames dispatch nothing. Loaders warm shared content/device static data. Equal fills share states; distinct fills use a bounded reusable pool. Claim/reuse/pool under the registry lock across loader/UI/dropper threads. Copy only recomputed scratch cells; reject storage-limit updates before mutation. Preserve native scan/early-exit/coastal/truncation rules and content search extents.

Political naval FX consumes the recorded native sorted supremacy cache, with control and gaining-side bytes derived for the viewing country. Timed acquisition certifies its native midnight cache epoch; unknown or crossed epochs withhold affected regions with named counts. Equal-current top/second candidates require identical output. Family precedes directed war; gaining neutral rows read the directed naval-blockade pointer from complete relation tuples, with Country0 refusal at that reached operand. Terrain minimums, thresholds and shader artwork resolve through the recorded content stack.

No production CPU fallback. [render::choose_adapter](src/render.rs) selects qualifying hardware at `WGPU_POWER_PREF`, then WARP/lavapipe with notice, equally for windows/widgets/dumps. Unsupported devices/dimensions return `UnsupportedPoliticalCompute`. Share one device per process; serialize GPU harness device creation to avoid concurrent WARP faults.

## Native GUI and hover ownership

Use native resources/byte-preserving names and one retained [Instance](../viewer-foundation/src/map/native_gui/lifecycle.rs) per window. The shared walk owns integer viewport layout, clipping, grids/scrollbars/reparenting and authored controls. Queried extents truncate; rasterised frames can stay fractional.

Hours hold ordered calls; frames apply them. Retain bits/writes/drawables/clocks until overwritten. Undefined/unchanged sprite names keep the drawable; replacement starts fresh clocks. Tick all related instances' clocks, reverse-registration transitions, then draws; present while `Instance::active` holds, including inherited shader clocks.

Pools belong to type/view, reusing state in native request/erase order; grid keys include pool/slot/place. Draw `Pool::visible` only, excluding erased fading icons/items. Refused updates retain membership/place but withhold draws until writes repair every unknown part; structural failures require reconstruction. Hide alone cannot repair unknown drawable state.

Producers own text/values, clipped geometry owns hits, and [ToolTip](../viewer-foundation/src/tooltip_painter.rs) owns wrapping/baking/placement/delay. Hit-test reverse paint/child order with transparent-text pass-through; pool order is not paint order. Disabled raid frames own hover. Combat windows precede map hits; map order is raid, land marker, naval marker, counter, province. New targets reset delay; same-province refresh preserves it.

Cache by content/frame/perspective/vision/settings. Raid text coalesces off-thread with same-icon carry. Use shared typed localisation/scripted-text evaluators. Acquire inline sprites from all loaded/runtime-selected text; absent declared artwork refuses layout. Count occurrences, keeping independent fragments.

Faction-pin highlights reuse the displayed hour's political regions and demilitarised zone. The [highlight lane](src/app/faction_highlight.rs) queues border plans on the hour builders and installs only results matching the current hour, perspective, border identity and hovered regions. Clearing or changing hover restores the prepared ordinary plans while the new highlight is pending.

Province text covers the default political callback only, with fog-gated buildings and potentially empty stateless sea/lakes, and then appends the viewing country's [air region statistics](../viewer-counter-products/src/map/province_tooltips/air_region.rs) for the province's strategic region: the received-damage grids the strategic air view draws in its default state, ours and the enemy's, each item its tooltip key's first line and its value as the game writes it, from AirRegionCombat after the latest [daily prune](../replay/README.md#derived-on-playback) that visited the viewing country, which the hourly AirDailyRoster of the frame's branch decides: a lowest-priority worker reads the recording's blocks for it and wakes the window, and until it answers the hover shows no statistics, so drawing decodes nothing. The worker searches only the latest question, stops between block reads when a newer one or closing supersedes it, and keeps each distinct roster once however many samples repeat it. The native view draws them in its own panels, which the viewer does not have; a region without records adds nothing, and a recording without the table, a roster gap before the visiting pass, or a roster that changed within a pass's tick refuses them by name. [Picking](../viewer-foundation/src/map/province_pick.rs) uses native ray/height cells, not terrain triangles. Province local time/night read the record's clock: a held clock as recorded, an updated one derived for the row's hour from the recorded content ([replay derivation](../replay/README.md#derived-on-playback)); recordings without clocks derive only when the recorded suspension gates prove the update ran. Province temperatures are the recorded ones before format 102 and otherwise the [replay derivation](../replay/README.md#derived-on-playback) from the frame's anchor with the recorded content, which steps on from the last frame of the same anchor; a refused derivation refuses the temperature line. Other map modes/selection additions need inputs. Ordinary maps keep carrier wing stacks empty. Battle plans include the viewing country and, when the local Allied battle plans option is enabled, native admitted allied occurrences. Unit selection stays local: owned-counter clicks select, Shift-click toggles and Escape clears; branch or viewing-country changes clear it. Complete censuses retire missing handles while unavailable censuses retain selection. Selected province text uses current-hour route operands through [selected movement](../viewer-foundation/src/map/movement_reads/selected_common.rs), with named refusals for missing inputs. The native move-rejection search reads the recorded controller and owner areas (`Provinces.controller_area`, `.owner_area`, `.area_node`; an area's land count is its provinces with that label and its first land's nonsea index comes from the content), a selected army's exile from its DivisionStatus exile bit and its commander's retreat bans from the hour's DivisionRetreatBans, which the game reads live (recordings before format 102 refuse them by name). Its transport fallback answers the game's naval-base branches and refuses by name at the retired admission operands past them.

Snow/mud display takes retained edges and refresh operands from the recorded native boundary. Dirty refreshes preserve the previous plane and advance a local fade on presentation frames, including paused playback. Seeks, step backs, branch/perspective changes and gaps rebind the target snapshot; forward playback, camera and fog changes preserve phase. Historical absence and read refusal remain named gaps.

The selected transport line follows the [native law](../installed-content-unit-path/src/unit_path/selected/transport.rs) over what is recorded: a hovered province without a naval base, a selection with no army or railway gun, or one whose units stand at the target or in its controller area without a naval base has no transport fragment, and the game's naval-base rejections are exact. Its access stage reads the unit's logical owner and the target's controller through the hour's relations (direct war, RelationLinks military access, factions and subject access). The stages before it and after it refuse by name, since the army's real-template coastal test, the transfer, convoys, sea route and risk are not recorded, so the line still refuses before access is reached; it never shows zero risk. Railway callers retain their own early returns and append laws.

## Fidelity limits

- Terrain deliberately uses a uniform full-resolution grid, differing from native distant-coast LOD.
- Interface text holding the 0x14 or 0x1A control refuses by name: the game draws them from a per-font sprite table it allocates but never fills, so their pixels are undefined. Map names and order labels read 0x14 as a codepoint without a glyph, as the game's map glyph runs do; a country name's fit still counts the 16 pixels the text measure gives it.
- Map text uses configured game language. Stored localised bytes without keys require recording-language agreement; proven literals follow the shared stored-text rule.
- Division arrival times are [estimates](../viewer-foundation/src/movement_timing.rs): the native walk over the recorded route, each edge at the native land speed ([eta_derive](../replay/src/eta_derive.rs)) of the recorded base speed times the common modifier plus the edge's terms, then the organisation and fuel factors, the native 1 km/h floor and the content's movement scale. The common modifier adds the owner's (expeditionary, else roster) country army speed and, for a hostile move (the recorded route ends at a province controlled by a country the logical owner is at war with), attack speed; the selected commander block's army speed (DivisionStatus.commander_modifiers); the location's province and region weather speed and, for a hostile move, attack speed entries times the acclimatization factor; the out-of-supply term (the received ratio and remaining grace, exact for a division with days out of supply (DivisionStatus.supply_ratio_exact and supply_grace_exact), whose penalty alone reads them; days out of supply, the country's and commander block's out-of-supply factors and the recorded defines; an exiled division bypasses it); the enemy air term where the logical owner holds a war relation (the owner's RaidAirScores pair and movement impact over the location's strategic region, the air speed define and the stat block's anti-air reduction, never above zero); and a retreating division's retreat terms or a withdrawing one's withdrawal term. Each edge adds the stat block's terrain movement and the terrain's own adjuster movement, its fort movement into a fortified province and its snow movement into snow, the direct commander's terrain movement, the destination state's infrastructure term and, across a crossing, the river terms with the commander block's river crossing factor. The organisation factor is the low-organisation slowdown of a hostile move from the organisation and maximum at their display steps; the fuel factor the recorded fuel over the stat block's capacity. Every modifier entry takes its recorded definition and the DLC feature mask. They omit the location state's speed modifiers, the template's priority sub-unit armour and category speed terms, the noncore-supply term (shown nowhere when the recorded bypass byte is set) and a relation's `river_crossing_factor_against`, whose operands are not recorded, so they drift from the game wherever those apply; a special-forces out-of-supply factor needs the template's special-forces share, which refuses the estimate by name. A division in combat that is not withdrawing refuses its estimate by name: the combat's movement factor replaces the modifier's one and is not recorded. So does a strategically redeploying division, which moves at the road or rail speed: its departure provinces' railway health is not recorded; and a naval transfer or a route edge to or from the sea, which moves at the owner's transport speed, not recorded since the selected transport preview retired. A missing operand of an applied term refuses the estimate by name; progress and arrows remain. Selected travel times the native route search's result the same way. Railway guns keep fresh current-hour native route queries; a sea-crossing or naval-transfer leg moves at the gun's logical owner's recorded transport speed in the leg's source strategic region (OwnerTransportSpeeds, recorded for the regions the owner's guns stand in or route through), and a leg whose speed was refused, or lies in a region off those paths, refuses the duration by name. The duration follows a route searched afresh to the endpoint, which can cross a region the stored paths do not (a selected destination, or the counter's own endpoint once conditions changed since the gun's path was stored). Task forces walk their stored route at native per-region speeds; their prospective travel refuses because route admission inputs are not recorded. The native client counter cache remains unrecorded. Counter hovers append no date.
- Recordings store drifting values at the precision the game shows them ([display steps](../replay-data/src/display.rs)): division organisation and strength, supply ratio and grace (exact beside the steps for a division with days out of supply), the unit supply footers' amounts, the land combat window's stats, scales and modifier values, the CAS hover's prevented damage, combat commanders' skill and learning-trait progress, and naval combat organisation and escape progress. Values the game prints directly read the same; derived displays (combat progress, strength shares and bars, stat boxes, damage estimates and the heading built from them, and the CAS hover's enemy strength and organisation totals) can differ from the game in their last digit at boundaries.
- Counter attrition hovers show the counter's cached hourly total (the game evaluates it afresh) and the lines the division attrition calculation writes whose operands are recorded: upkeep, the route province's terrain, the out-of-supply line (the exact received ratio and grace of a division with days out of supply normalized by the army attrition supply threshold, days out of supply, the country's and selected commander block's out-of-supply factors and the recorded out-of-supply attrition define), training and border war ([attrition.rs](../viewer-counter-products/src/map/counter_tooltips/attrition.rs)); a moving unit's weather, heat and winter lines are absent where its province's recorded weather entries write none. Its equipment, state, moving-state and modifier-source lines, and the weather lines' source trees where they appear, refuse by name, as does a supply line whose special-forces factor needs the template's share. A division counter's supply child adds no footer: the hover's supply status line is its total, without the consumer breakdown (demand, receipts by source and hub, air drops, disruption) that railway-gun and task-force supply children keep. The selected-army hover shows the blocked-strait notice without attack-modifier previews. The selected-army hover's attack modifiers are the [replay derivation](../replay/README.md#derived-on-playback) against the retained route's last move; an entry whose operand is missing is refused by name and counted while the others stay.
- A counter's supply child appends the native [supply footer](../viewer-counter-products/src/map/counter_tooltips/supply.rs) (demand, receipts by state and hub, air drops, flow reduction, disruption) for divisions, railway guns and task forces. A footer sums its members' recorded hundredths, so a footer over several units, its received and flow-reduction percentages, the order of hubs whose receipts print equal and the 0.05 hub cut-off can differ from the game at boundaries. Recordings without the division footer refuse it by name.
- Hover parts whose per-operand breakdowns or operands are not recorded refuse by name and are counted; the window and counter values beside them remain. The [combat guide](../viewer-combat/src/README.md) lists the combat hovers; the naval positioning, screening-correction, escape and carrier-sortie source lines read the recorded source trees, whose stored labels show in another language only through the keys that render to them. A port counter's strike-force status keeps its in-combat, not-ready, engagement, aggression-rule and intel arms; the readiness arms that weigh ships' strike words refuse.
- Cursor-zero counter columns can differ from the client's ordering/mixed-country flags. Preserve recorded members and native first/last representative rules.
- Engagement keeps native lag through the shared index. Pending/ambiguous history stays diagnosed; recorded naval bytes settle only the recording player's view. Naval windows refresh only while engaged and otherwise retain compatible earlier contents. Unseen frames use view/index history, then land authored/naval binding initial values with counted reasons; fallback establishes no history.
- Presentation history follows the game's laws but is not the captured instant. Ambient actors, weather and entity-event particles, entity lights, and unit, building, train and aircraft animation phases run on the viewer's clocks from fixed seeds, with the content's producers, assets, counts, durations and timelines; the game's wall-clock random draws and process clocks are not recorded. A fresh post chain takes its first adaptation value at once, the underwater pass's first previous shadow target is clear, and shadows are projected for the viewer's camera. What plays is exact where the recording decides it: ambient kinds and placements, weather regions, unit and building states, flights and their paired outcomes, train routes, and nuclear impacts, whose type's effect entity plays at the province's standstill anchor from when the impact's hour is presented, because the instant within the hour is not recorded, and critical shore bombardments, which play the game's small missile explosion entity the same way. Missile bombing scenarios fly from the recorded base-equipment wings (the game's client-side missile air scenarios, scheduled after scouts with `MISSILES_DIVISION_FACTOR` and `MAX_MISSILE_BOMBING_SCENARIOS`) to their mission's last target, and each that finishes plays the content's `missile_explosion_entity` in that province's effect slot on the aircraft clock; launch effects that bit-17 missions and launch-flagged definitions start at their bases are not drawn and are counted. The scenario's rocket rides the rig's `rocket` attachment node and ends the scenario, without a landing, at its first pose at or below the terrain. Vanilla's v2 rocket mesh, like every mod in the corpus, has no such node; the game logs that and the rocket rides the scenario entity's root, so it flies from its base to the target at `NAirGfx.ROCKET_SPEED`, lands on the terrain there and explodes as the rig starts. A rocket that rides a real node meets the game's near-ground band (`AIRPLANES_GROUND_COLLISION_OFFSET_Y` and the actor's ground-explosion state), which is not ported, so such a rig is refused by name and its scenarios counted; missing `MISSILES_DIVISION_FACTOR`, `MAX_MISSILE_BOMBING_SCENARIOS` or a zero factor omit only base-equipment wings (counted), and a missing `ROCKET_SPEED` refuses only a rootless rocket rig. A scenario rig of any kind whose mesh or animation file does not read is refused by name and its flights are counted; the map still opens. The explosion stands on the province's standstill anchor, as a nuclear impact's does, while the rocket lands at the scenario's integer centroid, as in the game.
- Floating map text plays the game's animated text for the recorded map events (fake division reveals, railway gun overruns, critical shore bombardments, general captures, paradrop landings, resistance activities and building damage): each event is admitted for the viewing country by the game's own local filter, reconstructed from the hour's recorded relations, fog, intel and modifiers; its text is localised through the recorded content stack and drawn with the content's text type, font, colour, speed and the game's rise-and-fade transform in the province's queue. The hour builders interpret each hour's events for the viewing country (admission, localisation and explosion placement); presenting an hour only queues them, each recorded event once, including one a later interval lists because it was in flight at its cut. An event's instant within its hour is not recorded, so its text starts when its hour is presented; continuing playback also starts the events of hours a forward skip passed, and a seek, step back, branch change or viewing-country change starts the queues again. Toggling fog keeps what was presented; a failed hour retried at the same hour presents nothing twice. Intervals with incomplete coverage are named in the diagnostics, and the events they hold still show. A no-pair land model whose fallback rows name different entities takes the second row of the game's labelled option list, built in the display language from the recorded variant names and country tags; a label that cannot be built refuses that figure by name. Warm-up/settling does not recover a captured game instant; consult producer refusals.

## Campaign appearance

Every map choice resolves on its own HOI4 -> global -> campaign -> widget ([map_display.rs](../dashboard-core/src/map_display.rs)): a scope stores only the choices it overrides, and each unset choice takes its parent's resolved value, so changing one leaves the others inheriting. The HOI4 level is the game's own value where `settings.txt` stores one (the map-rendering options, `multi_sampling`, `template_counter`, `hide_daynight_cycle`, `show_allied_plans` and the master, ambient, sound-effect and voice volumes), else the default. Null visual mods inherit independently, and the root shows none. Read game settings on source-open/settings jobs, never per frame; failures keep last values with diagnostics. The host tells the viewer when it saves settings, and the viewer reads them again.

Visual mods are any installed mods the user selects, mounted in the engine's order above the verified recorded stack as its [presentation stack](../installed-content/README.md#visual-mods). Terrain atlas and normal, colour, water, sea-bed, city-light, snow, mud, ice, reflection, fog-of-war and naval-dominance textures ([map_assets.rs](src/map_assets.rs)), border and strait textures, river surface textures, the railway texture and the tree tint/season bitmaps read the presentation stack. The bitmaps that place terrain, heights, rivers and trees, province and state geometry, positions, defines, shaders, models, interface, fonts and localisation read the recorded stack, so a mod's map changes never move anything. Artwork painted for another map (a colour map, tree tint) is stretched over the recorded one and can misalign; that is expected. A mod file the loader refuses, or one the upload would refuse in the colour space it is drawn in (faces, non-square cube faces, a format without that colour space, an empty extent), is drawn from the recording instead and named. Device size limits remain upload failures, as they are in the game. Renderers key maps by the set of chosen mods, so equivalent choices share one load; installing or updating a chosen mod shows after the map next loads. Mounted mods and every one that cannot be mounted are named in the load warnings, and the files each mod supplied are counted with the read diagnostics.

Show/hide switches gate drawing only: simulation, hover and picking, selection, combat windows, engagement, presentation clocks and the other layers carry on, and a hidden world object stops casting its shadow without changing anyone else's. Everything a frame advances (model histories and placements, train and aircraft plans with their random streams, strikes, map-frame actors, movement arrows and world text) is stepped in one place that reads no switch except to mark placed actors hidden, so a hidden layer shows again exactly as if it had never been hidden. The weather switch is the one exception, as in the game: turning weather off clears its particle simulations and clock. Each item has a main switch; its parts (border kinds, rails, bridges and trains, unit figures and aircraft by mission, counter and icon kinds, battle-plan families and labels) draw only while the main switch is on too. Hidden borders, demilitarised zones, straits and selected movement arrows still leave the passes after them the inputs they retain. A demilitarised zone splits its edges out of the ordinary border buckets, so while the zone display is hidden its edges draw as the ordinary land borders they are (no highlight, the class fade), and not at all while land borders are hidden; the faction hover outline is interaction feedback and keeps drawing. The counter filter also covers ships in port, by their task force's owner. The map table, nuclear impacts, critical bombardment explosions and missile explosions share one pass, each kind with its own switch: a hidden one stays placed, so impact admission, clocks and retirement carry on, and only its drawing, shadows and effects stop. Floating map text has a switch per recorded map event family; a hidden kind keeps its queue, phase and lifetime and only goes undrawn. Hidden counters and icons stay laid out, hit-tested and reported, so their hovers and combat windows still open; the tooltip switch hides only the painting. The counter filter draws all countries, the countries a human controls at the shown hour, or chosen tags; an hour lacking the tables it reads draws everyone. Battle-plan families filter arrow admission, so their labels follow, and raid paths share the battle-plan main switch. A hidden aircraft mission (missiles in flight are one) keeps its flights placed and animated and only stops drawing them, their shadows and their events, so a hidden missile still explodes where it lands. Fog of war stays a perspective choice, not a display switch. MSAA covers world/post/overlays; sample changes retire scenes, retaining compatible content/cameras. Quality selects authored textures; cache by recording, visual mod selection and quality. VSync updates the root surface. River/water/reflection choose native techniques; full map resolution renders the world at full size or truncating half dimensions with the compiled restore pass; matched visual/runtime validation remains separate. Gamma changes nothing, as in the game: its renderer applies the stored `graphics.gamma` only through a DirectX 9 device ramp, and the DirectX 11 renderer the viewer reproduces applies none, in a window or fullscreen. Day/night is a local display choice: disabling it fixes virtual lights while simulation time and province clocks remain authoritative. NATO choice also governs technology, carrier and nested scripted unit icons.

Widgets keep independent perspective/fog and export by view ID; missing/loading views refuse. Simple draw switches retain assets; artwork changes reload affected maps.

Sound volumes (master, ambient, units, combat, voice) resolve with the other choices and start from the game's `master_volume`, `ambient_volume`, `sound_fx_volume` (units and combat) and `voice_volume`, kept exactly as the game stores them (49.6 stays 49.6; only a value outside 0-100 is brought into range). Each view holds its resolved `MapAudio`. The viewer plays no map sound yet: the volumes are stored and resolved for the map audio, which is to play a category at `MapAudio::gain` on the focused map only.

## Compare changes

Tests that read interface grammar, content registries or fixed modifier names initialize runtime metadata through `engine::executable_tables::initialize_for_tests()`. Set `BATTLE_PLAN_GAME_DIR` to the supported installation; those tests fail explicitly when it is unset. Pure geometry and recording metadata fixtures need no installation.

Use [AGENTS.md](../../AGENTS.md) for scoped checks and [recorder](../recorder/README.md) for oracle/smoke integration. Match hour/branch/perspective/camera/resolution/settings; report scores and inspect images. Compilation, identical-input pixels, live fidelity and performance are separate checks.

```powershell
.\.cargo-target\release\viewer.exe --compare <game.png> <viewer.png> --exclude-game-ui --diff <difference.png>
```

UI masking supports 2560x1440/3440x1440 only. `--layers trees` omits trees; `--counters-json` exports submitted plates/diagnostics. `--window-dump` opens a window of its own only to check the windowed loading/rendering path against `--dump` (the [smoke run](../../tools/smoke.ps1) requires identical bytes) and for profiling sweeps, with repeatable animation controls. Headless presentation/map clocks are separate/default zero. Dumps settle pool requests at fixed hour/clocks; `--settle` advances the interval. Actor-local failures omit all parts; infrastructure/identity/frame/output errors remain fatal.

CPU GUI checks use synthetic technique, sampler, blend and clock metadata. Installed effect checks compare compiled submission with independently packed direct draws using resolved shader sources. They use `BATTLE_PLAN_GAME_DIR` and the configured compiler; `BATTLE_PLAN_SHADER_CONTENT_OVERLAY` adds an ordered, platform-separated list of content folders above the game layer. Module-local tests also cover political full/incremental/multi-map exactness and recording sweeps, plus [content-only scenes](src/map/offline_scene.rs), which prove no recorded unit/political history. Serialize GPU harnesses. `BATTLE_PLAN_VIEWER_CHECK_OVERLAY` verifies raid replacement equality.

| Profiling switch | Use |
|---|---|
| `BATTLE_PLAN_VIEWER_PROFILE` | CPU laps; `--window-dump` sweeps: `play:<hours/s>`, `pan:<units/s>`, `zoom:<factor/s>`, `hours`, `seek_end`. |
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
