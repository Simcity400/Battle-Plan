# viewer-world

A hovered accessible air base supplies ranges from its registered pools; when it supplies no groups, selected-wing ranges remain. Selected air-wing ranges use the current recorded wing/ace/country range law and the pool base's state centre or carrier province. Equal origins and radii share the game's range-group scratch entry, with empty-wing colour and alternating symbol rotation. Textures, technique, size and colours come from the recorded content stack.

Hovering an at-sea task-force source gathers each member's first carrier base in ship roster order, including missile carriers selected by positive equipment flags. A first carrier with a null air base stops that member's search. The bases share the range-group scratch in source order; with no admitted groups the selected-wing ranges remain. Missing ship, base or registration operands refuse the hovered range field independently.

The GPU scene, world labels, immutable scene content, draw passes and native shader adapters, political fill, terrain and weather, borders, names, rivers, trees, static and animated model rendering, model selection, buildings, railways, trains, strike effects, floating map text and post-processing. Content and shader sources resolve through the recorded installed-content stack.

[Terrain](src/map/terrain.rs) is the approved uniform grid for the game's camera and, for a free camera, the content shader's 64-cell blocks at the coarsest step their distance allows with snapped edges, on every map copy the view reaches; a free camera over a map whose sides are not whole blocks refuses its terrain pass by name. A free camera's [shadow](src/map/shadow.rs) light is fitted as the game camera's is, to its capped water-plane footprint with the same padding and backoff, and never extended for casters; its shadows turn on and fade by eye height against `DRAW_SHADOWS_CUTOFF` and `DRAW_SHADOWS_FADE_LENGTH`.

Aircraft and unit figures retain their own admission, pose and model selection; the walking mode's figure is the viewing country's infantry by the land ladder ([`UnitModelAssets::infantry_figure`](src/map/unit_models.rs)). `AircraftAssets::player` bakes a chosen aircraft for Fly mode on first use from the same roster, refusals and lazy library as recorded flights, and names why one cannot be flown, including the error of a mesh that does not load. The host supplies concrete air-base and region tables and shared recorded air/unit reads from foundation, and composes the GPU scene with counter and raid outputs. These passes consume concrete resources and draw operands, with shared upload, camera, GUI and visibility contracts from viewer-foundation. Selected movement routes and composed battle-plan/raid ribbons arrive as concrete draw data; world owns their GPU submission. [Strike effects](src/map/strike_effects.rs) play the content's nuclear and critical-bombardment entities from recorded `NuclearStrikes` and `MapEvents`, and its missile explosion entity where an aircraft missile bombing scenario finishes or in the mission region of a launch-flagged launch effect whose entity finishes. [Trade-route lines](src/map/trade_routes.rs) build the viewed country's production, supply-transfer, export, import, lend-lease and market lines from the recorded `ResourceOrigins`, `ConvoyClients`, `TradeRoutes` and `RegionalConvoyEfficiency`, drawn with the content's `traderoute` arrow type into the shared arrow manager, which shows them in the map modes with trade routes, and search the province graph within their recorded regions or states, inserting raw crossing controls with port anchors on port land provinces. Shared province-use counts displace controls before smoothing, in captured native directional list order. Lend-lease direction uses the receiver country separately from the giver membership, preserving giver-to-receiver delivery in both views. Missing or duplicate required positions refuse the affected layer by name; market route grouping compares route kind plus native source/destination states across country keys, combines bad-route colour, and preserves the first occurrence in that native list. Missing endpoint states refuse the line by name. Each province supplies its own hotspot variant, crossing controls take the adjacent minimum, and subdivision samples take the neighbouring maximum. Lua colours read three HSV or four RGBA components and convert RGB to linear as the game does, retaining alpha. Crossings whose trade permission or fleet blockade cannot be proved are refused by name. Convoys use counter-seeded segment positions, cross-layer launch collision checks, segment-delta control offsets and randomized sleeps. Hourly mesh uploads preserve the sailing clock and leave a new fleet uninitialized; only presentation frames advance it. Their native graphical seed, cached positions and sleep history are unsaved, so the viewer uses a fresh presentation and reports that limitation; an entity without proved creation bounds is refused. Stored-control route hits take priority over province text and refuse the unimplemented transfer tooltip by name. [Adjacency-rule icons](src/map/adjacency_icons.rs) present the content's `adjacency_rule_mapicon` window on each rule's icon province in the modes that show them, left-aligned and vertically centred. [Resource icons](src/map/resource_icons.rs) draw each state's `resource_mapicon` window in the resources map mode, one list row per resource of a positive nominal amount with its delivered and nominal amounts and the flags of resource rights, from the hour's recorded resource origins; the anchor is the member centroid nearest the rounded mean of the authored member centroids, with horizontal wrapping and authored-order ties; and the flags draw through the GUI context's flag source. [Floating map text](src/map/animated_map_text.rs) prepares each hour's recorded map events per viewing country by the game's own filters on the hour builders, presents each event once, and draws them with the content's animated map text types in per-province queues; a free eye shows each text standing at its origin as it shows map icons. The scene's overlay request says whether the map's interface that stands in the world draws (borders, demilitarised zones, straits, plans, arrows, floating text and names); hidden, it still steps. The map frame's ambient objects with `use_animation = no` play their entity's `default_state` from when they appear, as the game creates every entity instance in that state ([models.rs](src/map/models.rs) `prepare_default_states`); one that can reach a state with several variants stays still and is counted, since the game draws that variant from a per-entity counter no recording holds. Pass-local caches, retained shader bindings, native traversal and animation clocks belong to their existing owners.

The [scene](src/map/scene.rs) draws the model a free camera mode's user steers (`Scene::set_player`, [scene_player.rs](src/map/scene_player.rs)) as one entity in its own pass, animated, lit and casting shadows apart from the unit cutoffs and display switches, and wherever it flies: it draws on every map copy its own bounds reach inside the view volume, however high above the band the map's objects are admitted in. It casts as every caster does, on the canonical and each seam view with the light matrix translated for each. A frame that does not move it changes nothing, and a moved one rebuilds its one-instance field in the storage it keeps.

Shared synthetic GPU inputs come from foundation's opt-in `test-support`; world test helpers are enabled through the same feature for host composition tests. Follow the workspace [working rules](../../AGENTS.md).

Scripted colour layers select their country/state targets before running callbacks.
Selection, including root and per-target filters, captures the same input
refusals as colour callbacks. Any uncertain selection refuses the whole layer,
clears retained borders, and names each distinct input once for that layer.
An absent or empty targets block leaves the layer unrestricted.
A state-controller layer follows the captured cached single-group branch: the whole
state with its state controller when set, otherwise positive province controllers
grouped by canonical country. Borders retain colours between initial, daily and
recorded forced refreshes. Incomplete refresh-event coverage refuses retained colour
layers until a known refresh rebuilds them. A seek or mode selection starts a fresh
presentation, as a game load does.
An effect-driven refresh inherits its caller's temporary store, which the event
does not record. Forced callbacks and target filters therefore refuse unwritten
temporary reads by name instead of assuming zero. Each callback is proved
independently: outputs established solely by earlier targets are conservatively
refused too. Callbacks assigning all consumed outputs still paint, and a proved
zero alpha reads no RGB or highlight values. No transient script or UI state is
captured. A matching refresh event is checked even on a first/scheduled repaint.
When refresh-event coverage cannot exclude an effect refresh on a first or daily
repaint, its callbacks and filters use the same inherited-unknown temporary store.
Any reached uncertain input refuses the affected colour target, including an
uncertified sharing roster (ResearchModifiers). Each distinct input is named and
counted once per target, regardless of how many outputs it could affect. A refusal
withholds any previously painted border for that target. Later known assignments
do not cancel an earlier input refusal; this conservative limitation is accepted
because uncertain control flow must never invent a colour.

Each aircraft plan lazily indexes signed primary-equipment amounts once and
retains model selection results by variant and raw country for that hour alone,
including absence and failures. Actor order and per-occurrence diagnostics stay
unchanged; another hour/content context starts a fresh memo.

Aircraft declaration admission uses `Roster::refused_declaration` for ordinary and lazy bakes, recorded-roster fallbacks, recorded mesh retries and draw counterparts. Fly resolves through that same admission before requiring geometry in the accepted bake, so a plane recovered from the recording can be flown too; meshless actors still retain their events in recorded flights.

The Air and Navy modes fit strategic-region names through the ordinary connected-component name pass. Drawing follows the local `strategic_regions` preference: with it off, a picked province restricts names to its connected name area and a region change restarts the layer fade; with it on, or with no picked province, the native whole layer remains. Sorted component membership stays shared with the cached glyphs, so moving the pointer selects draws without rebuilding placements. This is viewer-local state. Air colours land and water regions from the viewing country's recorded superiority weights and zero-weight active-mission counters; Navy retains the country layer and the sea regions' naval-control and gaining-side effects. Strategic-region attributes join the gradient field by the same nonzero region key as its membership. Missing colour inputs withhold and count the affected regions.

Radar coverage uses the content's radar map symbol, texture, effect, size and own/allied/selected colours in the shared map-arrow manager, after main arrows and before entities. The picker controls these ranges separately from station icons; a station hover keeps its range visible with the switch off. Radius comes from the writer-certified RadarCoverage column. The host centre derives from the recorded state definitions and province pixel bounds, choosing the narrower seam-shifted box with ordinary ties and integer halving; missing certification or unproved bounds refuse the affected group by name. Rotations retain pooled draw-slot angles and start a fresh presentation because the native initial phase is unsaved.

The enabled Radar overlay admits own and direct family groups. With Radar off, a hovered station can supply any recorded group's range in the ordinary radar colour, including a foreign group; this hover path does not require family admission.

Air assignment stripes choose the longest-range selected wing at each resolved base before applying carrier activity, raid and training restrictions. Region reach shares foundation's transport geometry. Content supplies the allowed and blocked colours; an unknown reached operand withholds and counts affected regions. Province colour pixels share the political pass's immutable ID image and refresh only changed province tiles; terrain, water, trees and meshes consume the existing native secondary-colour shader input. Selection stays local to each viewer.

Province colour compute writes a linear storage view; its sRGB sampling view explicitly permits sampling only. Strategic selection opacity occupies the gradient FX texture's green channel, while naval control and contested bytes remain independent. Equal local styles share fills; differing styles and later clearing preserve each view's own textures.

Air mission arrows preserve the owning country's all-active registration order, coalesce equal base/region pairs in first-occurrence order and use any member's active byte. The transient bases reached by Air range preparation control their selected colour: occupied hovered bases and hovered first-carrier bases replace selection; without hovered groups the selected wings supply their pool bases. Gold base frames retain their separate local reference list. State endpoints search the ordinary Air-base building queue in their first listed province; hosted endpoints use the host task force's province centre. Both directions retain the native half-map seam rule. Arrows share type buckets, textures, lighting and submission with the other map ribbons and draw in Air mode independently of battle-plan switches.

Selected Air transfers traverse the recorded country base list and certified direct incoming order. Their lines coalesce source/destination pairs; progress symbols preserve fixed-point interpolation, total-distance gates, native seam scratch and orientation. Arrows and symbols use the existing shared map-arrow passes and appear only in Air mode. Selection stays local to the view.

Navy local selection colours distinguish selected hold missions from selected active missions and fade unselected regions through the content define. Assigned-region stripes combine every selected task force's accessible-region occurrences and each selected fleet's certified mission-region multiset; empty sources leave the denominator unchanged, repeated memberships retain their weight, and overlaps use the native fixed colours and clamped fractional blend. These feedback products share the existing political palette and province-colour passes and record no UI state.

Navy selected-region styles apply only when the recorded template's valid province occurrences are at least one percent sea, including duplicate occurrences and excluding invalid references. The native fixed5 fraction is prepared once from map kinds; it uses province counts rather than pixel areas.

Resistance and Compliance country stripes follow the viewing country's recorded controlled-state roster and each member province's canonical controller. Occupied states use the occupied country's primary map colour through the shared current-autonomy colour chain; the content's NMapMode.OCCUPATION_MAP_MODE_COUNTRY_STRIPE_ALPHA clamps before byte packing. Resistance level, compliance level and the resistance-active flag do not gate these stripes. They use the existing local province-colour texture; changing mode clears the previous field. Missing reached identities, rosters, colours or inheritance inputs withhold and count the affected stripes.

Resistance and Compliance draw state percentage labels and active definition badges through occupation_icons and the shared native GUI pool. The graphical-state anchor shares resource_icons' integer centroid selection, with native wrapping, integer mean conversion, signed distance arithmetic and authored-order ties. Admission uses canonical controller identity, the recorded occupied reference and the cached-target zero-level edge; neither active byte nor reconstructed thresholds substitute. Content defines supply strict parent/badge cutoffs, TOP_MAP_ICON priority and level gradients. Labels truncate whole percentages, preserve resistance trend icons and use the gradient's RGB with opaque text; badges stably sort retained occurrences by definition threshold before the authored grid admits them. Hover geometry belongs independently to the text and each badge icon.
A free view presents occupation markers through the existing world-icon scale, sight and hit transformations, with their anchor distance replacing the game camera height.

Model presentation borrows idle hour-builder turns for independent players. Clock callbacks
that register named states stay in placement order, repeated actor keys keep one serial
player, and palettes and event packets merge in the original placement order. Native-only
passes advance the same players and events without computing the unused name-joined
palette; native source-order actor matrices serve both event resolution and draws and stay
retained while their pose inputs are unchanged. Playerless and frozen actor-local results
are shared per immutable rig and frozen-animation choice; instance transforms stay separate,
and installing meshes clears those results. Native draw preparation installs exact
source-order placement routes with field/library changes and refreshes hidden membership
at its setter. Deferred shader/queue selection and actor-result handles are demanded only
at the original ordered checks; scratch, descriptor commits and arena staging keep their
ordinary serial order. Successful effective programs use invocation-local dense binding
sessions: each draw still stages the same complete constant blocks before resolving its
actual Arena buffer. Packets own immutable resource handles, while descriptor scratch,
independent pixel snapshots and final retained constants refer to immutable byte origins.
Refusals restore only the descriptor's scratch origins, not earlier uploads or texture
scratch; fatal errors preserve the reached prefix and suppress final retained commits.
Warm native binds consume ordered full-block source recipes from their successful effective
program; missing supplies still take the original ordered refusal checks. Caller userdata
selection is deferred to its ordinary demand, and accepted-part masks retain independent
pixel and skin origins without constructing a sparse constants table per warm bind.
Optional routing metadata falls back to the direct walk when it cannot be installed. Other palette consumers retain indexed
mesh/animation name joins and reuse their sampling buffers. Building groups already applied
at full scale do no slot work on camera moves inside the near cutoff; hour changes,
near/far transitions, growth and held mesh-budget creations still run in province order.

Hour builders retain snow/mud grids and prepare winter post-volume factors from
this content's volume records in the original arithmetic order. Installing an
hour adopts those shared products; the scene's graphics clock still admits the
refresh, copies its actual shown current planes to previous, and advances the
same fade. City damage likewise adopts the built map while content equality
controls change flags. Skipped built hours advance that history oldest first at
the last presented view before the final hour is installed.

Hour workers prepare immutable railway vertex buffers and province-ordered draw runs. The scene adopts the complete bank at the same hour publication, keeps its static railway pass and fades, and retires the displaced bank off the window thread. Hours built without a device prepare the same immutable bank synchronously at install; direct renders use that same preparation and adoption entry point.

Political fill metadata updates retain the layer's province-to-region slots and
integer pixel areas only when every source membership word matches. Changed
membership follows the ordinary synchronous planning and diff on the same frame;
styles, effects, storage-limit checks and dirty submissions are never skipped.

## Profiling

`political_find`, `political_acquire`, `political_country`, `political_mode` and
`political_naval` split show-time fill lookup, ownership and layer updates.
`political_area` and `political_mapping` are nested layer-update spans, aggregated
across layers; they must not be added to their inclusive parent laps.

Normal `BATTLE_PLAN_VIEWER_PROFILE` output includes presentation laps by model pass,
placement/field updates and clocks. The `units_*` spans separate schedule,
province visits, deferred creations, collection and install-time operand joins
(`units_prepare_inputs`, inside `units_join_prepared` or `units_join_naval`).
`animation_dispatch_plan` and `animation_dispatch_run_commit` separate animation
layout planning from worker evaluation and ordered serial commit. These laps
share the ordinary profiling switch and add no separate diagnostic mode.

Ordinary PROFILE also reports coarse same-thread wall and cycle CPU totals as
`mt_history_update`, `mt_unit_place`, `mt_unit_animate`, `mt_building_animate`,
`mt_city_animate`, `mt_aircraft_animate` and `mt_lights`, each with `_cpu` and
`_calls`. The renderer uses the shared presentation scopes. History includes
its interleaved building/unit visits and deferred collection; player wall spans
include borrowed-work waits, but their CPU excludes helpers. Lights includes
entity-particle advance, light evaluation, tiles and the original upload call:
uploads are immediate. These are inclusive source intervals within existing
frame/render totals, not an additive frame partition. Unreached stages have no
sample; short/zero samples are retained and unavailable CPU stays distinct.
`building_field_reused` and `building_field_updated` count the existing optional
placement result for each reached building pass, not inferred unchanged inputs.
With PROFILE off these scopes sample no clocks and create no records, counts or
observation storage. Placement, clocks, helpers, seeds and refusals are unchanged.

Hour-builder `buildings.*` children on the same `PROFILE built` line split vision,
token memo lookup, links, detection, naval-base membership, row joins/summaries,
authored slots, supply/floating harbours, city damage, neighbour wait/share and
warnings. `buildings.damage_validate` and `buildings.damage_map` are included in
`buildings.city_damage`; short/zero/unavailable samples are not filtered. Ordinary
entry points use the same body with disabled child scopes. No input, slot, refusal
or per-hour history is cached or skipped by this attribution.

Scene CPU `pass_*` laps mirror the render-pass boundaries even without GPU
queries and sum repeated wrap-view labels. They distinguish native model
encoding from recycling, arena flush, final encoder finish and queue submit.
`native_lit_prepare` and `native_caster_prepare` time each ordered preparation;
`native_encode_caller` and `native_turns_finish_wait` separate caller encoding
from joining borrowed frame turns. All use only normal `BATTLE_PLAN_VIEWER_PROFILE`.

The shared weather entry points report `mt_weather_packets` and
`mt_weather_prepare_lower`, with `_cpu` and `_calls`, in ordinary PROFILE.
These are same-thread wall and cycle-derived CPU totals, including every
reached wrap view and error prefix; packet wall includes borrowed-work waits,
not the helpers' CPU. The second scope covers retained binds, native packing, staging and immediate
command recording in their original interleaved order. These scopes are nested
inside existing frame/render totals, not
additional costs. Disabled profiling reads neither clock and adds no records.


PROFILE's `mt_sprite_upload` measures the immediate sprite upload's caller CPU
and wall time. Like the other `mt_*` scopes, it retains short/zero and unavailable
samples and nests inside the frame/render totals.


Unit builders retain only the equipment-graphics census keyed by held table
allocations and availability; current-hour selections and errors remain live.
The rig source copies only LOD distances, skeleton and locator names;
the original first-joint-index retention still follows that copy.

Permanent builder PROFILE children split unit ensure reads, registry/name waits,
shown/recorded loads, write-wait/merge and completion. Bake-worker CPU belongs to
its child records, not caller CPU; overlapping wall scopes are not additive.
Bridge/geography and naval inputs/formations/competitors/combats keep their
ordinary ordered work under their named child spans.


Ordinary builder PROFILE includes bounded `units.selection.*` stage totals for
land selection: template misses, pair classification, both stable sorts, pair
ranking, graphics rules/conventional resolution, fallback, numbered/transport
ladders and camouflage. Inclusive `land`, `paired`, `graphics` and `fallback`
parents are not additive with their children or the lazy bridge/geography reads.
Each reached stage emits at most one record per hour, in first-completion order;
no clocks or records are sampled per equipment row. Disabled scopes read no
clocks and allocate no record storage. Selection order, memo keys, errors and
live graphics operands are unchanged. These scopes end before ensure and naval
preparation; shared graphics helpers do not add naval work to land totals.


Land selection lazily retains successful variant classification facts for one
immutable authored catalogue, keyed by held variant-definition/token tables and
availability. Only equipment/archetype catalogue indices and the pure category
mask are shared. Current rows and variant lookup, errors, both stable sorts,
requirements, exclusions, ranking and live graphics answers still run at their
original demand. Replacing authored content starts an independent bank; clones
of that immutable content can share facts.
